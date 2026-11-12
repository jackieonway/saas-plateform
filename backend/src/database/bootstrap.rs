//! 空库的显式平台初始化，与只建表的迁移命令和只读运行账号分开。
use anyhow::{Context, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

const INITIALIZATION: &str = include_str!("../../initialization/0001_platform.sql");

/// 初始化结果。重复执行返回原有标识，且 `created` 为 false。
#[derive(Debug, sqlx::FromRow)]
pub struct BootstrapResult {
    pub created: bool,
    pub tenant_id: Uuid,
    pub organization_id: Uuid,
    pub admin_id: Uuid,
}

fn validate_credentials(username: &str, password: &str) -> Result<()> {
    anyhow::ensure!(
        (3..=64).contains(&username.len())
            && username
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c)),
        "超管用户名须为 3～64 位字母、数字、点、下划线或中划线"
    );
    anyhow::ensure!(
        password.chars().count() >= 12
            && password.len() <= 1024
            && !password.chars().any(char::is_control)
            && password.chars().any(|c| c.is_ascii_uppercase())
            && password.chars().any(|c| c.is_ascii_lowercase())
            && password.chars().any(|c| c.is_ascii_digit())
            && password.chars().any(|c| c.is_ascii_punctuation()),
        "初始密码须为至少 12 位，包含大小写字母、数字和符号，最多 1024 字节"
    );
    Ok(())
}

/// 在已完成当前迁移的空库中创建平台租户、根组织和超管及基础授权。
///
/// 使用数据库管理连接，在运行角色权限交接之前调用；不执行迁移或启动服务。
/// 密码使用 Argon2id 随机盐保存。事务失败不会留下部分数据，已有初始化仅核对并返回。
pub async fn initialize_platform(
    pool: &PgPool,
    username: &str,
    password: &str,
) -> Result<BootstrapResult> {
    validate_credentials(username, password)?;
    let owned_password = password.to_owned();
    let hash = tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(owned_password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|value| value.to_string())
            .map_err(|_| anyhow::anyhow!("初始密码摘要生成失败"))
    })
    .await
    .context("密码处理任务失败")??;
    let mut tx = pool.begin().await.context("初始化事务启动失败")?;
    sqlx::query("SET LOCAL search_path = public, pg_temp")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('saas.bootstrap.username',$1,true), set_config('saas.bootstrap.password_hash',$2,true)")
        .bind(username).bind(hash).execute(&mut *tx).await?;
    sqlx::raw_sql(INITIALIZATION)
        .execute(&mut *tx)
        .await
        .context("平台初始化失败：需要当前结构空库，或与原平台超管一致的已初始化数据")?;
    super::regions::initialize_in_tx(&mut tx)
        .await
        .context("平台地区基础数据初始化失败")?;
    let result = sqlx::query_as(
        "SELECT created,tenant_id,organization_id,admin_id FROM platform_bootstrap_result",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await.context("平台初始化提交失败")?;
    Ok(result)
}

/// 从明确的进程环境执行初始化；用户名默认 admin，密码和数据库 URL 必填。
/// 不读取 .env、不接收命令行凭据，也不输出密码或摘要。
pub async fn run_bootstrap_command() -> Result<BootstrapResult> {
    let database_url = std::env::var("DATABASE_URL").context("必须指定 DATABASE_URL")?;
    let username = std::env::var("BOOTSTRAP_ADMIN_USERNAME").unwrap_or_else(|_| "admin".into());
    let password =
        std::env::var("BOOTSTRAP_ADMIN_PASSWORD").context("必须指定 BOOTSTRAP_ADMIN_PASSWORD")?;
    validate_credentials(&username, &password)?;
    let options = super::migrate_only::database_options(&database_url)?;
    anyhow::ensure!(
        !matches!(
            options.get_database(),
            None | Some("postgres" | "template0" | "template1")
        ),
        "必须指定专用平台数据库"
    );
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(options)
        .await
        .context("平台初始化连接失败")?;
    let result = async {
        super::run_migrations(&pool)
            .await
            .context("平台结构基线初始化或校验失败")?;
        initialize_platform(&pool, &username, &password).await
    }
    .await;
    pool.close().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_rejects_weak_or_malformed_credentials() {
        assert!(validate_credentials("admin", "Owned-Initial-Password9!").is_ok());
        for password in [
            "",
            "admin123",
            "aaaaaaaaaaaa",
            "InitialPassword123",
            "Initial-Password9!\n",
        ] {
            assert!(validate_credentials("admin", password).is_err());
        }
        for username in ["", "ad", " admin", "admin\n", "a' OR 1=1"] {
            assert!(validate_credentials(username, "Owned-Initial-Password9!").is_err());
        }
    }
}
