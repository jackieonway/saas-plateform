//! Fresh initialization and UUIDv7 contracts, only in
//! disposable databases. No shared installation or migration record is repaired.
use super::*;
use sqlx::migrate::Migrate;
use std::time::Duration;

async fn history(pool: &PgPool) -> Result<Vec<Value>> {
    Ok(
        sqlx::query_scalar("SELECT to_jsonb(m) FROM _sqlx_migrations m ORDER BY version")
            .fetch_all(pool)
            .await?,
    )
}

async fn assert_lock_available(pool: &PgPool) -> Result<()> {
    let mut connection = pool.acquire().await?.detach();
    tokio::time::timeout(Duration::from_secs(2), connection.lock()).await??;
    connection.unlock().await?;
    sqlx::Connection::close(connection).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn canonical_initialization_has_no_bundled_microapps_and_restarts_cleanly() -> Result<()> {
    let database = TestDatabase::create_through(1).await?;
    let result = async {
        crate::database::run_migrations(&database.pool).await?;
        let before = history(&database.pool).await?;
        crate::database::run_migrations(&database.pool).await?;
        ensure!(before == history(&database.pool).await?, "idempotent startup changed migration records");
        let examples: i64 = sqlx::query_scalar("SELECT count(*) FROM applications WHERE code IN ('vue-workshop','legacy-jquery','report-center')")
            .fetch_one(&database.pool).await?;
        let menus: i64 = sqlx::query_scalar("SELECT count(*) FROM menus WHERE path LIKE '/micro/%'")
            .fetch_one(&database.pool).await?;
        ensure!(examples == 0 && menus == 0, "fresh initialization retained legacy demo records");
        let defaults: (String, String) = sqlx::query_as("INSERT INTO applications(code,name,id) VALUES('fresh-default','Fresh default',$1) RETURNING entry_type,runtime_mode")
            .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        ensure!(defaults == ("HTML".into(), "TRUSTED".into()), "fresh defaults are not qiankun");
        assert_lock_available(&database.pool).await?;
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn uuid_v7_target_enforces_versions_variants_and_application_generated_ids() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result = async {
        let uuid_columns: Vec<(String, String)> = sqlx::query_as(
            "SELECT t.relname::text,a.attname::text FROM pg_attribute a \
             JOIN pg_class t ON t.oid=a.attrelid JOIN pg_namespace n ON n.oid=t.relnamespace \
             WHERE n.nspname='public' AND t.relkind IN ('r','p') AND a.attnum>0 \
             AND a.atttypid='uuid'::regtype AND NOT a.attisdropped ORDER BY t.relname,a.attname",
        ).fetch_all(&database.pool).await?;
        // 0012 covers 107 columns; 0014 adds nine concrete credential identity,
        // application and parent-session UUID references. 0015 adds the notification
        // recipient, publisher and member-preference UUIDs. 0016 adds the global
        // session ID, account reference and pre-auth challenge ID. 0017 adds 15
        // independent OIDC client/request/grant/code/access/refresh UUID columns.
        // 0018 adds 18 checked UUID columns: browser/flow 4, BFF session 9,
        // platform binding 1, and idempotent account-logout receipts 4.
        // 0019 adds ten checked UUID references in platform-owned customer contexts.
        // 0020 adds 24 independently checked deployment, command, audit and receipt UUIDs.
        ensure!(uuid_columns.len() == 189, "unexpected platform UUID schema: {} columns", uuid_columns.len());
        let checked_uuid_columns: Vec<(String, String)> = sqlx::query_as(
            "SELECT DISTINCT t.relname::text,a.attname::text FROM pg_constraint c \
             JOIN pg_class t ON t.oid=c.conrelid JOIN pg_namespace n ON n.oid=t.relnamespace \
             JOIN pg_attribute a ON a.attrelid=t.oid AND a.attnum=ANY(c.conkey) \
             WHERE n.nspname='public' AND t.relkind IN ('r','p') AND c.contype='c' \
             AND c.convalidated AND cardinality(c.conkey)=1 AND a.atttypid='uuid'::regtype \
             AND NOT a.attisdropped AND pg_get_constraintdef(c.oid) LIKE '%is_uuid_v7(%' \
             ORDER BY t.relname::text,a.attname::text",
        ).fetch_all(&database.pool).await?;
        ensure!(uuid_columns == checked_uuid_columns,
            "UUID v7 checks do not cover the exact platform UUID column set: schema={uuid_columns:?} checked={checked_uuid_columns:?}");
        let defaults: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_attribute a JOIN pg_class t ON t.oid=a.attrelid \
             JOIN pg_namespace n ON n.oid=t.relnamespace JOIN pg_attrdef d ON d.adrelid=t.oid AND d.adnum=a.attnum \
             WHERE n.nspname='public' AND t.relkind='r' AND a.atttypid='uuid'::regtype AND NOT a.attisdropped",
        ).fetch_one(&database.pool).await?;
        ensure!(defaults == 0, "persistent UUIDs still have database defaults");
        expect_sqlstate(
            sqlx::query("INSERT INTO users(username,password_hash) VALUES('missing-id','unused')")
                .execute(&database.pool).await,
            "23502", "an omitted application UUID",
        )?;
        // Intentional negative fixtures: legacy v4, nil, and both non-RFC variants.
        for invalid in [
            Uuid::new_v4(),
            Uuid::nil(),
            Uuid::parse_str("019a1163-0000-7000-0000-000000000001")?,
            Uuid::parse_str("019a1163-0000-7000-c000-000000000001")?,
        ] {
            expect_sqlstate(
                sqlx::query("INSERT INTO users(id,username,password_hash) VALUES($1,'invalid-id','unused')")
                    .bind(invalid).execute(&database.pool).await,
                "23514", "an invalid UUID version or variant",
            )?;
        }
        let id = crate::util::uuid7::new_uuid();
        let inserted: Uuid = sqlx::query_scalar(
            "INSERT INTO users(id,username,password_hash) VALUES($1,'explicit-v7','unused') RETURNING id",
        ).bind(id).fetch_one(&database.pool).await?;
        ensure!(inserted == id, "database replaced the application UUID");
        // Nullable UUID references remain nullable after adding STRICT checks.
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(id,name,code) VALUES($1,'Nullable UUID fixture','nullable-uuid') RETURNING id",
        ).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let optional: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Nullable parent') RETURNING parent_id",
        ).bind(crate::util::uuid7::new_uuid()).bind(tenant).fetch_one(&database.pool).await?;
        ensure!(optional.is_none(), "nullable UUID result changed");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn changed_or_historical_migration_history_is_rejected_without_repair() -> Result<()> {
    let db = TestDatabase::create().await?;
    let result = async {
        let before = history(&db.pool).await?;
        ensure!(
            before.len() == 1,
            "fresh baseline must have exactly one migration"
        );
        sqlx::query(
            "UPDATE _sqlx_migrations SET checksum=decode(repeat('00',48),'hex') WHERE version=1",
        )
        .execute(&db.pool)
        .await?;
        let corrupted = history(&db.pool).await?;
        ensure!(
            crate::database::run_migrations(&db.pool).await.is_err(),
            "changed history was adopted"
        );
        ensure!(
            corrupted == history(&db.pool).await?,
            "migration history was repaired"
        );
        assert_lock_available(&db.pool).await?;
        Ok(())
    }
    .await;
    db.finish(result).await
}
