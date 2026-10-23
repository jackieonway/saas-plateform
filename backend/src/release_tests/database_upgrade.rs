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
        let defaults: (String, String) = sqlx::query_as("INSERT INTO applications(code,name) VALUES('fresh-default','Fresh default') RETURNING entry_type,runtime_mode")
            .fetch_one(&database.pool).await?;
        ensure!(defaults == ("HTML".into(), "TRUSTED".into()), "fresh defaults are not qiankun");
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
            let legacy: Uuid = sqlx::query_scalar("INSERT INTO applications(code,name,base_path,entry_type,runtime_mode,entry_url) VALUES('old-isolated','Preserve business','/micro/old-isolated','HTML','IFRAME','https://old.example/app/') RETURNING id")
                .fetch_one(&database.pool).await?;
            let builtin: Uuid = sqlx::query_scalar("INSERT INTO applications(code,name,base_path) VALUES('system','Platform builtin','/system') RETURNING id")
                .fetch_one(&database.pool).await?;
            let menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,type,title,path) VALUES($1,'MENU','Preserved menu','/micro/old-isolated') RETURNING id")
                .bind(legacy).fetch_one(&database.pool).await?;
            let before = history(&database.pool).await?;
            let (first, second) = tokio::join!(crate::database::run_migrations(&database.pool), crate::database::run_migrations(&database.pool));
            first?; second?;
            let after = history(&database.pool).await?;
            for row in &before { ensure!(after.contains(row), "historical migration metadata changed"); }
            ensure!(after.len() == sqlx::migrate!("./migrations").iter().count(), "concurrent startup applied a migration twice");
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
