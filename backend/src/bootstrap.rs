use crate::{
    artifact_distribution, client_ip, config, database, observability, region_seed,
    release_archive_maintenance, routes, seed, static_files,
};

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::Response;
use sqlx::postgres::PgPoolOptions;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::config::Config;
use crate::state::AppState;

pub(crate) async fn run() -> anyhow::Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if !arguments.is_empty()
        && tokio::task::spawn_blocking(move || release_archive_maintenance::run(arguments))
            .await??
    {
        return Ok(());
    }
    config::load_environment()?;
    let config = Arc::new(Config::try_from_env()?);
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    if config.is_production() {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    // PostgreSQL 连接 (失败重试, 最多 30 次 × 2s)
    let mut attempt = 0u32;
    let pool = loop {
        match PgPoolOptions::new()
            .max_connections(config.db_max_connections)
            .acquire_timeout(Duration::from_millis(config.db_acquire_timeout_ms))
            .after_connect({
                let statement = config.db_statement_timeout_ms.to_string();
                let lock = config.db_lock_timeout_ms.to_string();
                move |connection, _| {
                    let statement=statement.clone(); let lock=lock.clone();
                    Box::pin(async move {
                        sqlx::query("SELECT set_config('statement_timeout',$1,false),set_config('lock_timeout',$2,false)")
                            .bind(statement).bind(lock).execute(connection).await?;
                        Ok(())
                    })
                }
            })
            .connect(&config.database_url)
            .await
        {
            Ok(pool) => break pool,
            Err(e) => {
                if config.auto_create_database && database::is_database_missing(&e) {
                    database::create_missing_database(&config.database_url)
                        .await
                        .map_err(|_| anyhow::anyhow!("DATABASE_AUTO_CREATE_FAILED"))?;
                    continue;
                }
                attempt += 1;
                if attempt >= 30 {
                    anyhow::bail!("DATABASE_CONNECTION_FAILED");
                }
                tracing::warn!(
                    error_code = "DATABASE_NOT_READY",
                    attempt,
                    "Database not ready; retrying"
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    };
    tracing::info!("PostgreSQL 连接成功");

    // 迁移文件在编译时嵌入，新增迁移须重建后端镜像。
    database::run_migrations(&pool)
        .await
        .map_err(|_| anyhow::anyhow!("DATABASE_MIGRATION_FAILED"))?;
    tracing::info!("数据库迁移完成");

    // 种子 (幂等)
    seed::initialize(&pool, &config).await.map_err(|error| {
        // Preserve documented action codes; no SQL/URL/password error chains.
        let message = error.to_string();
        if message.starts_with("PRODUCTION_DEMO_") || message.starts_with("BOOTSTRAP_ADMIN_") {
            anyhow::anyhow!(message)
        } else {
            anyhow::anyhow!("DATABASE_INITIALIZATION_FAILED")
        }
    })?;
    region_seed::initialize(&pool).await.map_err(|error| {
        tracing::error!(error = %error, "Region catalog initialization failed");
        anyhow::anyhow!("REGION_INITIALIZATION_FAILED")
    })?;

    let state = AppState::new(pool, config.clone());
    artifact_distribution::start(state.clone());

    // 后台维护任务: 每 10 分钟把过期仍 PENDING 的设备码标记为 EXPIRED (供设备码轮询端点识别)
    {
        let maintenance_pool = state.pool.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(600)).await;
                if sqlx::query(
                    "UPDATE oauth_device_codes SET status = 'EXPIRED' WHERE expires_at < now() AND status = 'PENDING'",
                )
                .execute(&maintenance_pool)
                .await
                .is_err() {
                    tracing::warn!(error_code="DEVICE_EXPIRY_MAINTENANCE_FAILED", "Device expiry maintenance failed");
                }
            }
        });
    }

    // Historical deletion is opt-in. The default preserves OAuth history;
    // operators may explicitly invoke the audited administrative purge endpoint.
    if config.oauth_auto_purge {
        let purge_pool = state.pool.clone();
        tokio::spawn(async move {
            loop {
                match routes::oauth_admin::purge_stale_oauth_data(&purge_pool).await {
                    Ok((tokens, devices)) => {
                        tracing::info!(
                            "OAuth 历史数据清理完成: 删除令牌 {tokens} 行, 设备码 {devices} 行"
                        );
                    }
                    Err(_) => tracing::warn!(
                        error_code = "OAUTH_PURGE_FAILED",
                        "OAuth historical purge failed"
                    ),
                }
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });
    }

    let mut app = axum::Router::new()
        .merge(routes::api_router())
        .nest("/oauth", routes::oauth2::oauth_router())
        .with_state(state.clone());

    // 生产单进程模式: 仅宿主 HTML 导航使用 SPA fallback，缺失资源/API 保留 404。
    if !config.static_dir.is_empty() {
        app = app.fallback_service(static_files::service(&config.static_dir));
        tracing::info!(
            event = "STATIC_FILES_ENABLED",
            "Static file hosting enabled"
        );
    }

    let addr: SocketAddr = config.bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("平台后端已启动: http://{}", addr);
    let app = app
        .layer(cors_layer(&config))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            observability::observe,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            normalize_client_ip,
        ));
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    state.pool.close().await;
    Ok(())
}

async fn shutdown_signal() {
    let control_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _=control_c=>{}, _=terminate=>{} }
    tracing::info!(event = "SHUTDOWN_REQUESTED", "Graceful shutdown requested");
}

pub(crate) fn cors_layer(config: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> = config
        .cors_allowed_origins
        .iter()
        .map(|origin| HeaderValue::from_str(origin).expect("validated origin"))
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            axum::http::HeaderName::from_static("x-request-id"),
        ])
        .expose_headers([axum::http::HeaderName::from_static("x-request-id")])
}

#[cfg(test)]
#[tokio::test]
async fn cors_requires_the_configured_host_origin_for_every_api() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let host_origin = "http://127.0.0.1:3000";
    let config = Config::from_values(&std::collections::HashMap::from([(
        "CORS_ALLOWED_ORIGINS".into(),
        host_origin.into(),
    )]))
    .unwrap();
    let app = axum::Router::new()
        .route("/api/external/menus", axum::routing::get(|| async { "ok" }))
        .route("/api/external/roles", axum::routing::get(|| async { "ok" }))
        .route("/api/auth/profile", axum::routing::get(|| async { "ok" }))
        .layer(cors_layer(&config));
    for path in [
        "/api/external/menus",
        "/api/external/roles",
        "/api/auth/profile",
    ] {
        for origin in [host_origin, "null", "http://unapproved.example"] {
            let request = Request::builder()
                .method(Method::OPTIONS)
                .uri(path)
                .header(header::ORIGIN, origin)
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                .body(Body::empty())
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK);
            assert_eq!(
                response
                    .headers()
                    .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .and_then(|value| value.to_str().ok()),
                (origin == host_origin).then_some(host_origin),
                "unexpected CORS grant for {origin} on {path}"
            );
        }
    }
}

/// Normalize before all route handlers and observability. Headers supplied by an
/// untrusted peer cannot choose the audit or OAuth rate identity.
pub(crate) async fn normalize_client_ip(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    client_ip::normalize(&mut request, &state.config.trusted_proxy_ips);
    next.run(request).await
}
