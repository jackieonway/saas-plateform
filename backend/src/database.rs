use std::{borrow::Cow, str::FromStr};

use anyhow::{Context, Result};
use sqlx::migrate::{AppliedMigration, Migrate, Migrator};
use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, Error, PgConnection, PgPool};

static MIGRATIONS: Migrator = sqlx::migrate!("./migrations");

// Exact SHA384 digests of the initial schema before removing bundled legacy
// micro-application seeds. Recognize both checkout line endings, never arbitrary
// changed migrations. Historical database records are not rewritten.
const LEGACY_INITIAL_CHECKSUMS: [&str; 2] = [
    "7fd8f637a29b56267fab3cc3b3a73669790f7a75f15e91e7efd8ce9a66fba229a49045b39c5d119414ce4e4e95166feb",
    "b45372cdc2f03ec7a2c23dc45a6c572f35d55c72468d048db8d3cb037e06f2ab91b8fb74d5426de8c355693420a0f949",
];

fn compatible_migrator(applied: &[AppliedMigration]) -> Migrator {
    let mut migrations = MIGRATIONS.iter().cloned().collect::<Vec<_>>();
    if let Some(historical) = applied.iter().find(|migration| migration.version == 1) {
        if LEGACY_INITIAL_CHECKSUMS.contains(&hex::encode(&historical.checksum).as_str()) {
            if let Some(initial) = migrations
                .iter_mut()
                .find(|migration| migration.version == 1)
            {
                // v1 is already applied, so its SQL will never be rerun. Only
                // its in-memory expected identity is selected for this database.
                initial.checksum = Cow::Owned(historical.checksum.to_vec());
            }
        }
    }
    Migrator {
        migrations: Cow::Owned(migrations),
        ignore_missing: false,
        locking: false,
        no_tx: MIGRATIONS.no_tx,
    }
}

/// Apply canonical initialization to fresh databases and upgrade either exact
/// historical baseline without modifying its stored checksum. Validation and
/// execution share SQLx's advisory lock and one dedicated connection.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    // Detach so a cancelled/failed migration cannot return a session-level
    // advisory lock to the pool. Dropping this real connection closes it.
    let mut connection = pool.acquire().await?.detach();
    connection.lock().await?;
    let result: Result<()> = async {
        connection.ensure_migrations_table().await?;
        let applied = connection.list_applied_migrations().await?;
        compatible_migrator(&applied)
            .run_direct(&mut connection)
            .await?;
        Ok(())
    }
    .await;
    let unlocked = connection.unlock().await;
    let closed = connection.close().await;
    result?;
    unlocked?;
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
        assert_eq!(quote_identifier("test_db"), "\"test_db\"");
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn baseline_compatibility_is_exact_and_keeps_all_other_integrity_checks() {
        let canonical = MIGRATIONS
            .iter()
            .find(|migration| migration.version == 1)
            .unwrap();
        for checksum in LEGACY_INITIAL_CHECKSUMS {
            let applied = AppliedMigration {
                version: 1,
                checksum: Cow::Owned(hex::decode(checksum).unwrap()),
            };
            let selected = compatible_migrator(&[applied]);
            let initial = selected
                .iter()
                .find(|migration| migration.version == 1)
                .unwrap();
            assert_eq!(hex::encode(&initial.checksum), checksum);
            assert_eq!(initial.sql, canonical.sql);
            assert!(!selected.ignore_missing);
            assert!(!selected.locking);
            assert_eq!(selected.iter().count(), MIGRATIONS.iter().count());
            for original in MIGRATIONS.iter().filter(|migration| migration.version != 1) {
                assert_eq!(
                    selected
                        .iter()
                        .find(|migration| migration.version == original.version)
                        .unwrap()
                        .checksum,
                    original.checksum
                );
            }
        }
        let unknown = AppliedMigration {
            version: 1,
            checksum: Cow::Owned(vec![0; 48]),
        };
        assert_eq!(
            compatible_migrator(&[unknown])
                .iter()
                .find(|migration| migration.version == 1)
                .unwrap()
                .checksum,
            canonical.checksum
        );
    }

    #[test]
    fn fresh_initialization_has_no_bundled_micro_application_records() {
        let initial = MIGRATIONS
            .iter()
            .find(|migration| migration.version == 1)
            .unwrap();
        for code in [
            "vue-workshop",
            "legacy-jquery",
            "report-center",
            "/src/micro-apps/",
            "IFRAME",
            "ESM",
        ] {
            assert!(
                !initial.sql.contains(code),
                "legacy initialization still contains {code}"
            );
        }
        assert!(initial.sql.contains("DEFAULT 'HTML'"));
        assert!(initial.sql.contains("DEFAULT 'TRUSTED'"));
    }
}
