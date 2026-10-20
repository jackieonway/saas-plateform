//! Owned PostgreSQL regressions for immutable ZIP artifacts and registration recovery.
use super::*;
use crate::routes::application_releases::reconcile_upload_commit;

pub(super) fn archive(script: &[u8], reversed: bool, format: &str) -> Result<Vec<u8>> {
    let html = b"<html><script src='./app.js'></script></html>".as_slice();
    let mut entries = vec![("index.html", html), ("app.js", script)];
    if reversed {
        entries.reverse();
    }
    if format == "zip" {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer.start_file(name, zip::write::SimpleFileOptions::default())?;
            writer.write_all(bytes)?;
        }
        return Ok(writer.finish()?.into_inner());
    }
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut writer = tar::Builder::new(encoder);
    for (name, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        writer.append_data(&mut header, name, bytes)?;
    }
    Ok(writer.into_inner()?.finish()?)
}

fn upload_request(
    h: &Harness,
    environment: &str,
    version: &str,
    bytes: Vec<u8>,
    format: &str,
) -> Result<Request<Body>> {
    Ok(Request::builder().method(Method::POST).uri(format!(
        "/api/applications/{}/releases/upload?environment={environment}&version={version}&entryType=HTML&format={format}", h.main_app
    )).header(header::AUTHORIZATION, format!("Bearer {}", h.platform))
        .header(header::CONTENT_TYPE, "application/octet-stream").body(Body::from(bytes))?)
}

pub(super) async fn upload(
    h: &Harness,
    environment: &str,
    version: &str,
    bytes: Vec<u8>,
    format: &str,
) -> Result<(StatusCode, Value)> {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        h.app
            .clone()
            .oneshot(upload_request(h, environment, version, bytes, format)?),
    )
    .await??;
    let status = response.status();
    Ok((
        status,
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?,
    ))
}

async fn storage(h: &Harness, version: &str) -> Result<Value> {
    successful_call(
        &h.app,
        Method::GET,
        &format!(
            "/api/applications/{}/releases/storage?version={version}",
            h.main_app
        ),
        &h.platform,
        None,
    )
    .await
}

pub(super) fn owned_root() -> Result<std::path::PathBuf> {
    let root = std::env::temp_dir().join(format!("zip-registration-{}", Uuid::new_v4()));
    std::fs::create_dir(&root)?;
    Ok(root)
}

async fn harness(database: &TestDatabase, root: &Path) -> Result<Harness> {
    Harness::create_with_upload_root(
        &database.pool,
        "http://127.0.0.1:7777",
        vec!["http://127.0.0.1:7777".into()],
        root.to_str(),
    )
    .await
}

#[tokio::test]
#[ignore = "requires an owned temporary PostgreSQL database"]
async fn zip_retry_after_lost_response_and_cross_format_promotion_reuses_artifact() -> Result<()> {
    let database = TestDatabase::create().await?;
    let root = owned_root()?;
    let result: Result<()> = async {
        let h = harness(&database, &root).await?;
        // Discard the first completed response without reading its release ID.
        let response = h.app.clone().oneshot(upload_request(&h, "development", "retry", archive(b"original", false, "zip")?, "zip")?).await?;
        ensure!(response.status() == StatusCode::OK, "initial upload did not commit"); drop(response);
        let initial: Uuid = sqlx::query_scalar("SELECT id FROM application_releases WHERE application_id=$1 AND environment='development' AND version='retry'")
            .bind(h.main_app).fetch_one(&database.pool).await?;
        let (status, rejected) = call(&h.app, Method::POST, &format!("/api/applications/{}/releases", h.main_app), &h.platform,
            Some(json!({"environment":"staging","version":"retry","entryType":"HTML","entryUrl":"/micro/retry/index.html","resources":[]}))).await?;
        ensure!(status == StatusCode::CONFLICT, "URL registration bypassed the bound ZIP version: {status} {rejected}");
        let unbound: i64 = sqlx::query_scalar("SELECT count(*) FROM application_releases WHERE application_id=$1 AND version='retry' AND artifact_digest IS NULL")
            .bind(h.main_app).fetch_one(&database.pool).await?;
        ensure!(unbound == 0, "rejected URL registration created an unbound environment row");
        let (status, retried) = upload(&h, "development", "retry", archive(b"original", true, "zip")?, "zip").await?;
        ensure!(status == StatusCode::OK && release_id(&retried)? == initial, "same artifact retry changed the ID: {status} {retried}");
        let (status, promoted) = upload(&h, "staging", "retry", archive(b"original", true, "tar.gz")?, "tar.gz").await?;
        ensure!(status == StatusCode::OK && promoted["artifactDigest"] == retried["artifactDigest"], "tar.gz promotion did not reuse canonical payload: {status} {promoted}");
        let (status, _) = upload(&h, "production", "retry", archive(b"different", false, "zip")?, "zip").await?;
        ensure!(status == StatusCode::CONFLICT, "different digest in another environment was accepted");
        ensure!(std::fs::read(root.join("release-app/versions/retry/app.js"))? == b"original", "conflict changed installed bytes");
        let state = storage(&h, "retry").await?;
        ensure!(state["status"] == "REGISTERED" && state["references"].as_array().is_some_and(|rows| rows.len() == 2), "missing cross-environment references: {state}");
        let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE application_id=$1 AND action='REGISTER_APPLICATION_RELEASE'")
            .bind(h.main_app).fetch_one(&database.pool).await?;
        ensure!(audits == 2, "idempotent retry duplicated the registration audit");
        Ok(())
    }.await;
    let _ = std::fs::remove_dir_all(&root);
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires an owned temporary PostgreSQL database"]
async fn zip_concurrent_same_version_has_one_registration_and_conflicting_digest_cannot_win(
) -> Result<()> {
    let database = TestDatabase::create().await?;
    let root = owned_root()?;
    let result: Result<()> = async {
        let h = harness(&database, &root).await?;
        let bytes = archive(b"same", false, "zip")?;
        let (left, right) = tokio::join!(upload(&h, "development", "parallel", bytes.clone(), "zip"), upload(&h, "development", "parallel", bytes, "zip"));
        let (ls, left) = left?; let (rs, right) = right?;
        ensure!(ls == StatusCode::OK && rs == StatusCode::OK && left["id"] == right["id"], "parallel retries diverged: {ls} {rs} {left} {right}");
        let a = archive(b"winner-a", false, "zip")?; let b = archive(b"winner-b", true, "zip")?;
        let (a, b) = tokio::join!(upload(&h, "development", "race", a, "zip"), upload(&h, "staging", "race", b, "zip"));
        let a = a?; let b = b?;
        ensure!([a.0,b.0].iter().filter(|status| **status == StatusCode::OK).count() == 1 && [a.0,b.0].iter().filter(|status| **status == StatusCode::CONFLICT).count() == 1, "different artifacts did not resolve to one immutable winner: {} {}", a.0,b.0);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM application_releases WHERE application_id=$1 AND version='parallel'")
            .bind(h.main_app).fetch_one(&database.pool).await?;
        ensure!(count == 1, "parallel duplicate database rows");
        let state = storage(&h, "race").await?;
        ensure!(state["status"] == "REGISTERED", "concurrent winner is not consistent: {state}");
        Ok(())
    }.await;
    let _ = std::fs::remove_dir_all(&root);
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires an owned temporary PostgreSQL database"]
async fn zip_database_rejection_retains_orphan_and_other_environment_references() -> Result<()> {
    let database = TestDatabase::create().await?;
    let root = owned_root()?;
    let result: Result<()> = async {
        let h = harness(&database, &root).await?; let bytes = archive(b"retained", false, "zip")?;
        let (status, _) = upload(&h, "development", "shared", bytes.clone(), "zip").await?;
        ensure!(status == StatusCode::OK, "initial shared upload failed");
        sqlx::raw_sql("CREATE FUNCTION phase5_reject_registration() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic registration rejection'; END $$; CREATE TRIGGER phase5_reject_registration BEFORE INSERT ON application_releases FOR EACH ROW EXECUTE FUNCTION phase5_reject_registration();")
            .execute(&database.pool).await?;
        for version in ["orphan", "shared"] {
            let (status, _) = upload(&h, "staging", version, bytes.clone(), "zip").await?;
            ensure!(status == StatusCode::INTERNAL_SERVER_ERROR, "synthetic DB rejection did not fail");
            ensure!(std::fs::read(root.join(format!("release-app/versions/{version}/app.js")))? == b"retained", "registration failure removed installed bytes");
        }
        ensure!(storage(&h, "orphan").await?["status"] == "INSTALLED_UNREGISTERED", "orphan was not discoverable");
        ensure!(storage(&h, "shared").await?["status"] == "REGISTERED", "other environment lost its artifact");
        sqlx::raw_sql("DROP TRIGGER phase5_reject_registration ON application_releases; DROP FUNCTION phase5_reject_registration();").execute(&database.pool).await?;
        let (status, row) = upload(&h, "staging", "orphan", bytes, "zip").await?;
        ensure!(status == StatusCode::OK && row["artifactDigest"].is_string(), "same-package retry failed recovery: {status} {row}");
        Ok(())
    }.await;
    let _ = std::fs::remove_dir_all(&root);
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires an owned temporary PostgreSQL database"]
async fn zip_unknown_commit_and_post_commit_lookup_failure_preserve_registered_artifact(
) -> Result<()> {
    let database = TestDatabase::create().await?;
    let root = owned_root()?;
    let result: Result<()> = async {
        let h = harness(&database, &root).await?;
        let (status, row) = upload(&h, "development", "uncertain", archive(b"committed", false, "zip")?, "zip").await?;
        ensure!(status == StatusCode::OK, "initial upload failed");
        let digest = row["artifactDigest"].as_str().ok_or_else(|| anyhow!("missing digest"))?;
        let acknowledgement_lost = Err(sqlx::Error::Io(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "synthetic lost COMMIT acknowledgement")));
        let reconciled = reconcile_upload_commit(&database.pool, h.main_app, "development", "uncertain", digest, acknowledgement_lost).await?;
        ensure!(reconciled.id == release_id(&row)?, "unknown commit did not read the actual registered row");
        let unavailable = PgPoolOptions::new().connect_lazy("postgresql://unused@127.0.0.1:1/unused")?; unavailable.close().await;
        ensure!(reconcile_upload_commit(&unavailable, h.main_app, "development", "uncertain", digest, Ok(())).await.is_err(), "closed lookup pool was accepted");
        ensure!(std::fs::read(root.join("release-app/versions/uncertain/app.js"))? == b"committed", "post-commit lookup failure removed registered files");
        ensure!(storage(&h, "uncertain").await?["status"] == "REGISTERED", "reconciliation lost registered state");
        let mutate = sqlx::query("UPDATE application_releases SET artifact_digest=$2 WHERE id=$1").bind(reconciled.id).bind("b".repeat(64)).execute(&database.pool).await;
        ensure!(mutate.is_err(), "bound artifact digest was mutable");
        let mutate = sqlx::query("UPDATE application_release_artifacts SET manifest='{}' WHERE application_id=$1 AND version='uncertain'").bind(h.main_app).execute(&database.pool).await;
        ensure!(mutate.is_err(), "artifact manifest was mutable");
        Ok(())
    }.await;
    let _ = std::fs::remove_dir_all(&root);
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires an owned temporary PostgreSQL database"]
async fn zip_legacy_adoption_requires_matching_immutable_release_definition() -> Result<()> {
    let database = TestDatabase::create().await?;
    let root = owned_root()?;
    let result: Result<()> = async {
        let h = harness(&database, &root).await?;
        let legacy = h.register(json!({"environment":"development","version":"legacy","entryType":"HTML",
            "entryUrl":"/sub-apps/release-app/versions/legacy/index.html","resources":[{"url":"app.js","kind":"SCRIPT"}]})).await?;
        let (status, adopted) = upload(&h, "development", "legacy", archive(b"legacy", true, "zip")?, "zip").await?;
        ensure!(status == StatusCode::OK && adopted["id"] == legacy["id"] && adopted["artifactDigest"].is_string(), "matching legacy release was not adopted: {status} {adopted}");
        h.register(json!({"environment":"development","version":"external","entryType":"HTML","entryUrl":"/micro/external/index.html","resources":[]})).await?;
        let (status, _) = upload(&h, "staging", "external", archive(b"external", false, "zip")?, "zip").await?;
        ensure!(status == StatusCode::CONFLICT, "an unrelated URL registration was silently converted to ZIP");
        let old: Option<String> = sqlx::query_scalar("SELECT artifact_digest FROM application_releases WHERE application_id=$1 AND version='external'")
            .bind(h.main_app).fetch_one(&database.pool).await?;
        ensure!(old.is_none(), "legacy metadata changed after conflict");
        Ok(())
    }.await;
    let _ = std::fs::remove_dir_all(&root);
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires an owned temporary PostgreSQL database and loopback HTTP"]
async fn zip_storage_and_activation_recheck_real_files_even_when_http_keeps_returning_valid_content(
) -> Result<()> {
    let database = TestDatabase::create().await?;
    let root = owned_root()?;
    let result: Result<()> = async {
        let fixture = HttpFixture::start_at("127.0.0.1:0").await?;
        let h = Harness::create_with_upload_root(&database.pool, &fixture.origin, vec![fixture.origin.clone()], root.to_str()).await?;
        let path = format!("/api/applications/{}/releases/storage?version=v1", h.main_app);
        let (denied, _) = call(&h.app, Method::GET, &path, &h.a, None).await?;
        ensure!(denied == StatusCode::FORBIDDEN, "tenant user read storage reconciliation");
        ensure!(storage(&h, "v1").await?["status"] == "MISSING", "missing version was not reported");
        let mut ids = Vec::new();
        for version in ["v1", "v2"] {
            let (status, row) = upload(&h, "development", version, archive(version.as_bytes(), false, "zip")?, "zip").await?;
            ensure!(status == StatusCode::OK && row["validationStatus"] == "PASSED", "initial intact upload failed preflight: {status} {row}");
            let id = release_id(&row)?; h.activate(id).await?; ids.push(id);
        }
        std::fs::write(root.join("release-app/versions/v1/app.js"), b"tampered")?;
        ensure!(storage(&h, "v1").await?["status"] == "INCONSISTENT", "tampered file was not diagnosed");
        let (status, _) = call(&h.app, Method::POST, &format!("/api/applications/{}/deployments/development/rollback", h.main_app), &h.platform, None).await?;
        ensure!(status == StatusCode::CONFLICT, "rollback accepted corrupted previous artifact");
        let previous = h.preflight(ids[0], "FAILED").await?;
        ensure!(previous["validationResult"]["checks"][0]["kind"] == "ARTIFACT", "local corruption was hidden by valid HTTP fixture");
        std::fs::write(root.join("release-app/versions/v2/app.js"), b"tampered")?;
        let (status, _) = call(&h.app, Method::POST, &format!("/api/applications/{}/releases/{}/activate", h.main_app, ids[1]), &h.platform, None).await?;
        ensure!(status == StatusCode::CONFLICT, "activation accepted a stale PASSED artifact after file corruption");
        let active: Uuid = sqlx::query_scalar("SELECT active_release_id FROM application_deployments WHERE application_id=$1 AND environment='development'")
            .bind(h.main_app).fetch_one(&database.pool).await?;
        ensure!(active == ids[1], "failed rollback changed deployment pointer");
        std::fs::remove_dir_all(root.join("release-app/versions/v1"))?;
        let missing = storage(&h, "v1").await?;
        ensure!(missing["status"] == "MISSING" && missing["references"].as_array().is_some_and(|rows| rows.len() == 1), "missing artifact hid its database reference");
        Ok(())
    }.await;
    let _ = std::fs::remove_dir_all(&root);
    database.finish(result).await
}
