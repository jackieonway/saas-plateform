mod config;
mod releases;
mod notifications;
mod app_token;
mod authorization;
mod database;
mod error;
mod extractors;
mod routes;
mod seed;
mod session;
mod state;
mod static_files;
mod util;

#[cfg(test)]
mod scope_tests;

#[cfg(test)]
mod release_tests;

#[cfg(test)]
mod notification_tests;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use tower_http::cors::CorsLayer;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // .env 值优先于环境变量 (沙箱中存在遗留的全局 DATABASE_URL, 必须以 backend/.env 为准)
    dotenvy::dotenv_override().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Arc::new(Config::from_env());

    // PostgreSQL 连接 (失败重试, 最多 30 次 × 2s)
    let mut attempt = 0u32;
    let pool = loop {
        match PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&config.database_url)
            .await
        {
            Ok(pool) => break pool,
            Err(e) => {
                if config.auto_create_database && database::is_database_missing(&e) {
                    database::create_missing_database(&config.database_url).await?;
                    continue;
                }
                attempt += 1;
                if attempt >= 30 {
                    anyhow::bail!("数据库连接失败: {e}");
                }
                tracing::warn!("数据库未就绪 ({e}), 2 秒后重试 ({attempt}/30)");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    };
    tracing::info!("PostgreSQL 连接成功");

    // 迁移
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("数据库迁移完成");

    // 种子 (幂等)
    seed::run(&pool).await?;

    let state = AppState::new(pool, config.clone());

    // 后台维护任务: 每 10 分钟把过期仍 PENDING 的设备码标记为 EXPIRED (供设备码轮询端点识别)
    {
        let maintenance_pool = state.pool.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(600)).await;
                if let Err(e) = sqlx::query(
                    "UPDATE oauth_device_codes SET status = 'EXPIRED' WHERE expires_at < now() AND status = 'PENDING'",
                )
                .execute(&maintenance_pool)
                .await
                {
                    tracing::warn!("设备码过期标记失败: {e}");
                }
            }
        });
    }

    // 每小时清理: 已过期超 7 天的 OAuth 令牌与终态设备码 (启动先跑一次即生效;
    // 与 POST /api/oauth/tokens/purge 共用 purge_stale_oauth_data, 语义见该函数文档;
    // 取代原先「过期 1 天即删」的激进清理, 保留 7 天排查窗口)
    {
        let purge_pool = state.pool.clone();
        tokio::spawn(async move {
            loop {
                match routes::oauth_admin::purge_stale_oauth_data(&purge_pool).await {
                    Ok((tokens, devices)) => {
                        tracing::info!("OAuth 历史数据清理完成: 删除令牌 {tokens} 行, 设备码 {devices} 行");
                    }
                    Err(e) => tracing::warn!("OAuth 历史数据清理失败: {e}"),
                }
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });
    }

    let mut app = axum::Router::new()
        .merge(routes::api_router())
        .nest("/oauth", routes::oauth2::oauth_router())
        .with_state(state.clone())
        .layer(CorsLayer::permissive());

    // 生产单进程模式: 仅宿主 HTML 导航使用 SPA fallback，缺失资源/API 保留 404。
    if !config.static_dir.is_empty() {
        app = app.fallback_service(static_files::service(&config.static_dir));
        tracing::info!("静态文件目录: {}", config.static_dir);
    }

    let addr: SocketAddr = config.bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Dante Cloud 后端已启动: http://{}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}
