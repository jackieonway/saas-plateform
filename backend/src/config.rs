use std::sync::Arc;

/// 环境配置 (见 backend/.env)
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Config {
    pub database_url: String,
    /// 目标数据库不存在时使用同一连接账号创建；生产环境建议保持关闭。
    pub auto_create_database: bool,
    pub bind_addr: String,
    pub jwt_secret: String,
    pub session_ttl_hours: i64,
    pub credential_ticket_ttl_minutes: i64,
    pub access_token_ttl_seconds: i64,
    pub refresh_token_ttl_seconds: i64,
    /// 对外基础地址 (为空则使用相对路径)
    pub public_app_url: String,
    /// 生产模式静态文件目录 (SPA fallback)
    pub static_dir: String,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| default.to_string())
}

fn env_i64(key: &str, default: i64) -> i64 {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: env_or("DATABASE_URL", "postgres://dante@127.0.0.1:5432/dante_cloud"),
            auto_create_database: env_bool("AUTO_CREATE_DATABASE", false),
            bind_addr: env_or("BIND_ADDR", "0.0.0.0:3001"),
            jwt_secret: env_or("JWT_SECRET", "dante-cloud-dev-secret-change-me"),
            session_ttl_hours: env_i64("SESSION_TTL_HOURS", 12),
            credential_ticket_ttl_minutes: env_i64("CREDENTIAL_TICKET_TTL_MINUTES", 10),
            access_token_ttl_seconds: env_i64("ACCESS_TOKEN_TTL_SECONDS", 7200),
            refresh_token_ttl_seconds: env_i64("REFRESH_TOKEN_TTL_SECONDS", 604800),
            public_app_url: env_or("PUBLIC_APP_URL", ""),
            static_dir: env_or("STATIC_DIR", ""),
        }
    }

    /// 对外跳转基础地址 (末尾不带 /), 为空表示相对路径
    pub fn public_base(&self) -> String {
        self.public_app_url.trim_end_matches('/').to_string()
    }
}

#[allow(dead_code)]
pub type SharedConfig = Arc<Config>;
