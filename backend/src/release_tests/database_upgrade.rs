//! Canonical initialization and exact historical-baseline upgrades, only in
//! disposable databases. No shared installation or migration record is repaired.
use super::*;
use sqlx::migrate::Migrate;
use std::time::Duration;

const HISTORICAL_INITIAL: [&str; 2] = [
    "7fd8f637a29b56267fab3cc3b3a73669790f7a75f15e91e7efd8ce9a66fba229a49045b39c5d119414ce4e4e95166feb",
    "b45372cdc2f03ec7a2c23dc45a6c572f35d55c72468d048db8d3cb037e06f2ab91b8fb74d5426de8c355693420a0f949",
];

/// Reconstruct only the old loader constraints inside a disposable upgrade
/// fixture, before inserting records that predate qiankun-only admission.
pub(super) async fn historical_loader_constraints(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql("ALTER TABLE applications DROP CONSTRAINT applications_runtime_mode_check;
        ALTER TABLE applications ADD CONSTRAINT applications_runtime_mode_check CHECK(runtime_mode IN ('TRUSTED','IFRAME'));
        ALTER TABLE application_releases DROP CONSTRAINT application_releases_entry_type_check;
        ALTER TABLE application_releases ADD CONSTRAINT application_releases_entry_type_check CHECK(entry_type IN ('HTML','ESM'));")
        .execute(pool).await?;
    Ok(())
}

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
async fn uuid_v7_migration_rejects_legacy_data_without_partial_schema_or_history_changes(
) -> Result<()> {
    let database = TestDatabase::create_through(11).await?;
    let result = async {
        // Intentional legacy v4 fixture: the source must be rekeyed offline,
        // rather than accepting a partially enforced target schema at startup.
        let old_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users(id,username,password_hash) VALUES($1,'legacy-v4-source','unused')")
            .bind(old_id).execute(&database.pool).await?;
        let before_history = history(&database.pool).await?;
        let before_row: Value = sqlx::query_scalar("SELECT to_jsonb(u) FROM users u WHERE id=$1")
            .bind(old_id).fetch_one(&database.pool).await?;
        let before_defaults: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT table_name,column_default FROM information_schema.columns \
             WHERE table_schema='public' AND column_name='id' AND data_type='uuid' ORDER BY table_name",
        ).fetch_all(&database.pool).await?;
        ensure!(crate::database::run_migrations(&database.pool).await.is_err(), "legacy v4 data passed the target migration");
        ensure!(before_history == history(&database.pool).await?, "rejected UUID migration changed SQLx history");
        let after_row: Value = sqlx::query_scalar("SELECT to_jsonb(u) FROM users u WHERE id=$1")
            .bind(old_id).fetch_one(&database.pool).await?;
        ensure!(before_row == after_row, "rejected UUID migration changed a business row");
        let after_defaults: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT table_name,column_default FROM information_schema.columns \
             WHERE table_schema='public' AND column_name='id' AND data_type='uuid' ORDER BY table_name",
        ).fetch_all(&database.pool).await?;
        ensure!(before_defaults == after_defaults, "rejected UUID migration partially removed defaults");
        let constraints: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_constraint WHERE conname LIKE 'uuid_v7_%'")
            .fetch_one(&database.pool).await?;
        let function: bool = sqlx::query_scalar("SELECT to_regprocedure('public.is_uuid_v7(uuid)') IS NOT NULL")
            .fetch_one(&database.pool).await?;
        ensure!(constraints == 0 && !function, "rejected UUID migration partially installed checks");
        assert_lock_available(&database.pool).await?;
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops isolated PostgreSQL databases using TEST_DATABASE_URL"]
async fn exact_historical_baselines_upgrade_concurrently_without_rewriting_history() -> Result<()> {
    for checksum in [
        None,
        Some(HISTORICAL_INITIAL[0]),
        Some(HISTORICAL_INITIAL[1]),
    ] {
        let database = TestDatabase::create_through(8).await?;
        let result = async {
            historical_loader_constraints(&database.pool).await?;
            // The test deliberately constructs pre-cleanup defaults and legacy
            // records. Only its own v1 metadata is stamped with a known fixture.
            sqlx::raw_sql("ALTER TABLE applications ALTER COLUMN entry_type SET DEFAULT 'ESM'; ALTER TABLE applications ALTER COLUMN runtime_mode SET DEFAULT 'IFRAME';")
                .execute(&database.pool).await?;
            if let Some(checksum) = checksum {
                sqlx::query("UPDATE _sqlx_migrations SET checksum=decode($1,'hex') WHERE version=1")
                    .bind(checksum).execute(&database.pool).await?;
            }
            let legacy: Uuid = sqlx::query_scalar("INSERT INTO applications(code,name,base_path,entry_type,runtime_mode,entry_url,id) VALUES('old-isolated','Preserve business','/micro/old-isolated','HTML','IFRAME','https://old.example/app/',$1) RETURNING id")
                .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
            let builtin: Uuid = sqlx::query_scalar("INSERT INTO applications(code,name,base_path,id) VALUES('system','Platform builtin','/system',$1) RETURNING id")
                .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
            let menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,type,title,path,id) VALUES($1,'MENU','Preserved menu','/micro/old-isolated',$2) RETURNING id")
                .bind(legacy).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
            let before = history(&database.pool).await?;
            let (first, second) = tokio::join!(crate::database::run_migrations(&database.pool), crate::database::run_migrations(&database.pool));
            first?; second?;
            let after = history(&database.pool).await?;
            for row in &before { ensure!(after.contains(row), "historical migration metadata changed"); }
            let actual_versions: Vec<i64> = after.iter().map(|row| row["version"].as_i64()
                .ok_or_else(|| anyhow!("migration history has an invalid version"))).collect::<Result<_>>()?;
            let expected_versions: Vec<i64> = sqlx::migrate!("./migrations").iter().map(|migration| migration.version).collect();
            ensure!(actual_versions == expected_versions,
                "concurrent startup did not apply the exact embedded migration versions: actual={actual_versions:?} expected={expected_versions:?}");
            let old: (String, String, String, String) = sqlx::query_as("SELECT status,name,entry_type,runtime_mode FROM applications WHERE id=$1")
                .bind(legacy).fetch_one(&database.pool).await?;
            ensure!(old == ("DISABLED".into(), "Preserve business".into(), "HTML".into(), "IFRAME".into()), "legacy application was deleted or silently promoted");
            let present: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM menus WHERE id=$1 AND application_id=$2)")
                .bind(menu).bind(legacy).fetch_one(&database.pool).await?;
            ensure!(present, "legacy menu was deleted");
            let builtin_status: String = sqlx::query_scalar("SELECT status FROM applications WHERE id=$1")
                .bind(builtin).fetch_one(&database.pool).await?;
            ensure!(builtin_status == "ENABLED", "platform builtin was disabled");
            crate::database::run_migrations(&database.pool).await?;
            ensure!(after == history(&database.pool).await?, "second startup rewrote history");
            assert_lock_available(&database.pool).await?;
            Ok(())
        }.await;
        database.finish(result).await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn baseline_compatibility_rejects_corrupt_missing_and_dirty_history_without_leaking_lock(
) -> Result<()> {
    let database = TestDatabase::create_through(8).await?;
    let result = async {
        let originals: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&database.pool).await?;
        for case in ["unknown_initial", "changed_later", "unknown_version", "dirty_initial", "dirty_later"] {
            for (version, checksum) in &originals {
                sqlx::query("UPDATE _sqlx_migrations SET checksum=$2,success=true WHERE version=$1")
                    .bind(version).bind(checksum).execute(&database.pool).await?;
            }
            sqlx::query("DELETE FROM _sqlx_migrations WHERE version=999").execute(&database.pool).await?;
            sqlx::query("UPDATE _sqlx_migrations SET checksum=decode($1,'hex') WHERE version=1")
                .bind(HISTORICAL_INITIAL[0]).execute(&database.pool).await?;
            match case {
                "unknown_initial" => { sqlx::query("UPDATE _sqlx_migrations SET checksum=decode('00','hex') WHERE version=1").execute(&database.pool).await?; }
                "changed_later" => { sqlx::query("UPDATE _sqlx_migrations SET checksum=decode('00','hex') WHERE version=2").execute(&database.pool).await?; }
                "unknown_version" => { sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES(999,'Unknown version',true,decode('00','hex'),0)").execute(&database.pool).await?; }
                "dirty_initial" => { sqlx::query("UPDATE _sqlx_migrations SET success=false WHERE version=1").execute(&database.pool).await?; }
                "dirty_later" => { sqlx::query("UPDATE _sqlx_migrations SET success=false WHERE version=8").execute(&database.pool).await?; }
                _ => unreachable!(),
            }
            let before = history(&database.pool).await?;
            ensure!(crate::database::run_migrations(&database.pool).await.is_err(), "accepted invalid migration history: {case}");
            ensure!(before == history(&database.pool).await?, "failure rewrote migration history: {case}");
            assert_lock_available(&database.pool).await?;
        }
        // Cancel a migration while another connection owns the same SQLx lock.
        // The cancelled dedicated connection must not retain a session lock.
        let mut holder = database.pool.acquire().await?.detach();
        holder.lock().await?;
        let waiting_pool = database.pool.clone();
        let waiting = tokio::spawn(async move { crate::database::run_migrations(&waiting_pool).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        waiting.abort();
        let _ = waiting.await;
        holder.unlock().await?;
        sqlx::Connection::close(holder).await?;
        assert_lock_available(&database.pool).await?;
        Ok(())
    }.await;
    database.finish(result).await
}
