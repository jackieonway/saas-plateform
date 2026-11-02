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
    let initialization_mode = std::env::var_os("PLATFORM_DATABASE_INIT_MODE");
    let initialization_mode = initialization_mode
        .as_deref()
        .map(|value| {
            value.to_str().ok_or_else(|| {
                anyhow::anyhow!("PLATFORM_DATABASE_INIT_MODE must be seed, import or imported")
            })
        })
        .transpose()?;
    database::ensure_server_initialization_mode(initialization_mode)?;
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

    if config.identity_remote_enabled {
        verify_identity_deployment(&pool, &config).await?;
        tracing::info!(
            event = "IDENTITY_DEPLOYMENT_VERIFIED",
            "Independent identity deployment verified without migrations or seed"
        );
    } else if let Some(imported) = &config.imported_database {
        let verified = tokio::time::timeout(Duration::from_secs(30), imported.verify(&pool)).await;
        if !matches!(verified, Ok(Ok(()))) {
            let _ = tokio::time::timeout(Duration::from_secs(10), pool.close()).await;
            anyhow::bail!("IMPORTED_DATABASE_VERIFICATION_FAILED");
        }
        tracing::info!(
            event = "IMPORTED_DATABASE_VERIFIED",
            "Imported database verified without migrations or seed"
        );
    } else {
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
    }

    let state = AppState::try_new(pool, config.clone())?;
    artifact_distribution::start(state.clone());

    let mut app = axum::Router::new()
        .merge(routes::api_router_for_deployment(
            config.identity_remote_enabled,
        ))
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

async fn verify_identity_deployment(pool: &sqlx::PgPool, config: &Config) -> anyhow::Result<()> {
    let issuer = config
        .oidc_issuer
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("IDENTITY_DEPLOYMENT_ISSUER_REQUIRED"))?;
    let verified = tokio::time::timeout(Duration::from_secs(3), async {
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM public.identity_deployment_state d \
             JOIN pg_catalog.pg_roles r ON r.rolname=current_user \
             WHERE d.singleton AND d.schema_version=1 AND d.issuer=$1 \
             AND d.platform_runtime=current_user AND current_user=session_user \
             AND r.rolcanlogin AND NOT r.rolsuper AND NOT r.rolcreaterole \
             AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members m WHERE m.member=r.oid) \
             AND NOT r.rolcreatedb AND NOT r.rolreplication AND NOT r.rolbypassrls \
             AND NOT has_schema_privilege(current_user,'public','CREATE') \
             AND NOT has_database_privilege(current_user,current_database(),'CREATE') \
             AND NOT has_database_privilege(current_user,current_database(),'TEMP') \
             AND NOT has_column_privilege(current_user,'public.users','password_hash','SELECT') \
             AND NOT has_column_privilege(current_user,'public.users','password_hash','UPDATE') \
             AND NOT has_column_privilege(current_user,'public.users','status','UPDATE') \
             AND NOT has_column_privilege(current_user,'public.users','session_version','UPDATE') \
             AND NOT has_table_privilege(current_user,'public.users','DELETE') \
             AND NOT has_table_privilege(current_user,'public.users','TRUNCATE') \
             AND NOT has_column_privilege(current_user,'public.users','id','UPDATE') \
             AND NOT has_any_column_privilege(current_user,'public.users','INSERT') \
             AND NOT has_table_privilege(current_user,'public.users','TRIGGER') \
             AND NOT EXISTS(SELECT 1 FROM unnest(ARRAY[ \
                 'platform_app_capabilities','platform_resource_services','platform_resource_bindings','platform_machine_delegations']::text[]) AS target(name) \
                 WHERE to_regclass('public.'||target.name) IS NULL \
                    OR NOT (has_table_privilege(current_user,'public.'||target.name,'SELECT') \
                        AND has_table_privilege(current_user,'public.'||target.name,'INSERT') \
                        AND has_table_privilege(current_user,'public.'||target.name,'UPDATE') \
                        AND has_table_privilege(current_user,'public.'||target.name,'DELETE')) \
                    OR (SELECT relowner FROM pg_catalog.pg_class WHERE oid=to_regclass('public.'||target.name)) \
                       IS DISTINCT FROM (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=d.platform_owner)) \
             AND has_function_privilege(current_user,'public.platform_app_capability_active(text)','EXECUTE') \
             AND public.identity_phase20_schema_ready() \
             AND has_function_privilege(current_user,'public.platform_identity_management_actor_active(uuid,uuid,text,boolean,boolean)','EXECUTE') \
             AND has_function_privilege(current_user,'public.identity_admin_assurance(uuid,uuid,uuid,bigint,uuid,bigint,text)','EXECUTE') \
             AND NOT has_function_privilege(current_user,'public.identity_auth_review_intent_active(uuid,uuid,text,text,text,jsonb)','EXECUTE') \
             AND NOT has_function_privilege(current_user,'public.identity_auth_invitation_intent_active(uuid,uuid,text,text,text,uuid,uuid)','EXECUTE') \
             AND has_table_privilege(current_user,'public.platform_auth_release_bindings','SELECT') \
             AND NOT has_any_column_privilege(current_user,'public.platform_auth_release_bindings','INSERT,UPDATE,REFERENCES') \
             AND NOT has_table_privilege(current_user,'public.platform_auth_release_bindings','DELETE,TRUNCATE,TRIGGER') \
             AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class a \
                 JOIN pg_catalog.pg_namespace an ON an.oid=a.relnamespace \
                 WHERE an.nspname='public' AND a.relname IN \
                 ('sso_sessions','sso_login_challenges','auth_provider_settings','auth_signing_keys', \
                  'auth_clients','auth_client_credentials','auth_client_permissions', \
                  'auth_applications','auth_application_admissions','auth_subject_sectors','auth_subject_links', \
                  'auth_authorization_requests','auth_authorization_codes','auth_grants', \
                  'auth_access_tokens','auth_access_token_scopes','auth_grant_scopes', \
                  'auth_refresh_families','auth_refresh_tokens','auth_rp_sessions','auth_context_interactions', \
                  'auth_consents','auth_consent_scopes','auth_consent_groups','auth_consent_group_clients', \
                  'auth_consent_revocation_outbox','auth_logout_command_receipts','auth_delivery_jobs','auth_delivery_targets', \
                  'auth_invitations','auth_invitation_verifications','auth_invitation_activation_receipts', \
                  'auth_totp_enrollments','auth_totp_factors','auth_totp_recovery_codes', \
                  'auth_mfa_recovery_receipts','auth_mfa_recovery_approvals','auth_security_notification_outbox', \
                  'identity_service_clients','identity_audit_logs','identity_account_command_receipts', \
                  'identity_auth_command_receipts','identity_consent_command_receipts', \
                  'identity_account_revocation_facts') AND \
                 (has_any_column_privilege(current_user,a.oid,'SELECT') \
                  OR has_any_column_privilege(current_user,a.oid,'INSERT') \
                  OR has_any_column_privilege(current_user,a.oid,'UPDATE') \
                  OR has_any_column_privilege(current_user,a.oid,'REFERENCES') \
                  OR has_table_privilege(current_user,a.oid,'DELETE') \
                  OR has_table_privilege(current_user,a.oid,'TRUNCATE') \
                  OR has_table_privilege(current_user,a.oid,'TRIGGER'))) \
             AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname !~ '^pg_' AND n.nspname<>'information_schema' \
                 AND pg_has_role(current_user,c.relowner,'MEMBER')) \
             AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_proc p \
                 JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                 WHERE n.nspname !~ '^pg_' AND n.nspname<>'information_schema' \
                 AND pg_has_role(current_user,p.proowner,'MEMBER')) \
             AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace n \
                 WHERE n.nspname !~ '^pg_' AND n.nspname<>'information_schema' \
                 AND has_schema_privilege(current_user,n.oid,'CREATE')) \
             AND NOT pg_has_role(current_user,d.identity_owner,'MEMBER') \
             AND NOT pg_has_role(current_user,d.migration_owner,'MEMBER') \
             AND NOT pg_has_role(current_user,d.platform_owner,'MEMBER') \
             AND NOT pg_has_role(current_user,d.revocation_bridge_owner,'MEMBER') \
             AND NOT pg_has_role(current_user,d.projection_owner,'MEMBER'))",
        )
        .bind(issuer)
        .fetch_one(pool)
        .await?;
        anyhow::ensure!(valid, "IDENTITY_DEPLOYMENT_PRIVILEGES_INVALID");
        let endpoint = config
            .identity_service_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("IDENTITY_SERVICE_URL_REQUIRED"))?;
        let client = config
            .identity_http_client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("IDENTITY_SERVICE_TRANSPORT_REQUIRED"))?;
        let mut response = client
            .get(format!("{endpoint}/health/ready"))
            .send()
            .await?;
        anyhow::ensure!(
            response.status().as_u16() == 200,
            "IDENTITY_SERVICE_NOT_READY"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= 8192,
                "IDENTITY_READY_RESPONSE_INVALID"
            );
            bytes.extend_from_slice(&chunk);
        }
        let body: serde_json::Value = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            body == serde_json::json!({"status":"UP","service":"saas-identity","issuer":issuer}),
            "IDENTITY_READY_RESPONSE_INVALID"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await;
    if !matches!(verified, Ok(Ok(()))) {
        let _ = tokio::time::timeout(Duration::from_secs(10), pool.close()).await;
        anyhow::bail!("IDENTITY_DEPLOYMENT_VERIFICATION_FAILED");
    }
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
