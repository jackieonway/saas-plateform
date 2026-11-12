//! Single fresh-installation baseline. Historical databases are not adopted.
use anyhow::{Context, Result};
use sqlx::migrate::Migrator;
use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, Error, PgConnection, PgPool};
use std::str::FromStr;
pub mod bootstrap;
mod imported;
mod migrate_only;
pub mod regions;
pub use imported::{ImportedDatabase, ImportedDatabaseError};
pub use migrate_only::{
    ensure_server_initialization_mode, run_migration_command, MigrationCommandError,
};
static MIGRATIONS: Migrator = sqlx::migrate!("./migrations");

/// Apply the fresh baseline or verify an unchanged current installation.
/// SQLx checks history and checksum; closing the detached connection releases
/// advisory locks on cancellation and every error path.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    let mut connection = pool.acquire().await?.detach();
    let result: Result<()> = async {
        let history: bool=sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NOT NULL").fetch_one(&mut connection).await?;
        if !history {
            let occupied: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p','v','m','S'))").fetch_one(&mut connection).await?;
            anyhow::ensure!(!occupied,"FRESH_DATABASE_REQUIRED: recreate the explicitly selected old database");
        }
        MIGRATIONS.run_direct(&mut connection).await?;
        Ok(())
    }.await;
    let closed = connection.close().await;
    result?;
    closed?;
    Ok(())
}

/// 只在目标库明确不存在时调用。使用同一账号连接 PostgreSQL 维护库。
pub async fn create_missing_database(database_url: &str) -> Result<()> {
    let options = PgConnectOptions::from_str(database_url).context("DATABASE_URL 格式无效")?;
    let name = options
        .get_database()
        .filter(|name| !name.is_empty())
        .context("DATABASE_URL 必须指定目标数据库名")?;
    anyhow::ensure!(
        !matches!(name, "postgres" | "template0" | "template1"),
        "不能自动创建 PostgreSQL 维护库"
    );
    let quoted_name = quote_identifier(name);
    let mut connection = PgConnection::connect_with(&options.database("postgres"))
        .await
        .context("连接 PostgreSQL 维护库失败；请检查账号的连接权限")?;

    // CREATE DATABASE 不支持参数化标识符；双引号转义后作为单个标识符使用。
    let statement = format!("CREATE DATABASE {quoted_name}");
    match sqlx::query(&statement).execute(&mut connection).await {
        Ok(_) => {
            tracing::info!("目标数据库已创建");
            Ok(())
        }
        // 并发启动的另一个进程可能刚创建相同数据库。
        Err(Error::Database(error)) if error.code().as_deref() == Some("42P04") => Ok(()),
        Err(error) => Err(error).context("自动创建目标数据库失败；请检查 CREATEDB 权限"),
    }
}

/// Identify PostgreSQL's missing-database error without treating other connection
/// failures as authorization to create a database.
pub fn is_database_missing(error: &Error) -> bool {
    matches!(error, Error::Database(db) if db.code().as_deref() == Some("3D000"))
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quotes_database_identifier() {
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
    }
    #[test]
    fn only_fresh_sso_uuid7_baseline_is_embedded() {
        assert_eq!(MIGRATIONS.iter().count(), 1);
        let initial = MIGRATIONS.iter().next().unwrap();
        assert_eq!(initial.version, 1);
        assert!(initial.sql.contains("CREATE TABLE public.bff_sessions"));
        assert!(initial.sql.contains("CREATE FUNCTION public.is_uuid_v7"));
        assert!(!initial.sql.contains("INSERT INTO public.oauth_clients"));
        assert!(!initial
            .sql
            .contains("memberships_user_id_organization_id_key"));
    }
}
