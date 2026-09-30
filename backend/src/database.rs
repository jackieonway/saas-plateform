use std::str::FromStr;

use anyhow::{Context, Result};
use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, Error, PgConnection};

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

pub fn is_database_missing(error: &Error) -> bool {
    matches!(error, Error::Database(db) if db.code().as_deref() == Some("3D000"))
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::quote_identifier;

    #[test]
    fn quotes_database_identifier() {
        assert_eq!(quote_identifier("test_db"), "\"test_db\"");
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
    }
}
