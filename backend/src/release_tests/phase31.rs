//! Phase31 additive upgrades and read-only retention against disposable PostgreSQL.
use super::*;

async fn snapshot(pool: &PgPool) -> Result<BTreeMap<String, Value>> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename::text FROM pg_tables WHERE schemaname='public' ORDER BY tablename",
    )
    .fetch_all(pool)
    .await?;
    let mut result = BTreeMap::new();
    for table in tables {
        ensure!(table
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'));
        let value: Value = sqlx::query_scalar(&format!("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM \"{table}\" t"))
            .fetch_one(pool).await?;
        result.insert(table, value);
    }
    Ok(result)
}

#[tokio::test]
#[ignore = "requires owned disposable PostgreSQL"]
async fn phase31_platform_upgrade_preserves_every_old_row_and_checksum() -> Result<()> {
    let database = TestDatabase::create_through(3).await?;
    let result = async {
        let _h = Harness::create(&database.pool, "http://127.0.0.1:7777").await?;
        let before = snapshot(&database.pool).await?;
        ensure!(before["membership_data_grants"].as_array().is_some_and(|v| !v.is_empty()));
        sqlx::migrate!("./migrations").run(&database.pool).await?;
        sqlx::migrate!("./migrations").run(&database.pool).await?;
        let after = snapshot(&database.pool).await?;
        for (table, rows) in &before {
            if table == "_sqlx_migrations" {
                let old = rows.as_array().unwrap();
                let current = after[table].as_array().unwrap();
                ensure!(old.len() == 3 && current.len() == sqlx::migrate!("./migrations").iter().count());
                for row in old { ensure!(current.contains(row), "historical migration record changed"); }
            } else { ensure!(after.get(table) == Some(rows), "upgrade changed {table}"); }
        }
        ensure!(after["application_release_operations"] == json!([]));
        // A corrupt checksum is rejected, never repaired or adopted.
        sqlx::query("UPDATE _sqlx_migrations SET checksum=decode('00','hex') WHERE version=1").execute(&database.pool).await?;
        let corrupt = snapshot(&database.pool).await?;
        ensure!(sqlx::migrate!("./migrations").run(&database.pool).await.is_err());
        ensure!(corrupt == snapshot(&database.pool).await?);
        println!("PHASE31_PLATFORM_UPGRADE PASS old=3 current={} all_old_rows_and_checksums_preserved corruption_rejected", sqlx::migrate!("./migrations").iter().count());
        Ok(())
    }.await;
    database.finish(result).await
}

async fn preview(h: &Harness) -> Result<Value> {
    successful_call(
        &h.app,
        Method::GET,
        &format!(
            "/api/applications/{}/releases/retention-preview",
            h.main_app
        ),
        &h.platform,
        None,
    )
    .await
}
fn record(value: &Value, id: Uuid) -> Result<&Value> {
    value["records"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["releaseId"] == id.to_string()))
        .ok_or_else(|| anyhow!("missing retention target"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires owned disposable PostgreSQL and Node24"]
async fn phase31_preview_preserves_database_zip_and_online_urls() -> Result<()> {
    let database = TestDatabase::create().await?;
    let root = zip_registration::owned_root()?;
    let result = async {
        let http = HttpFixture::start_at("127.0.0.1:0").await?;
        let old_origin = std::env::var("PHASE31_OLD_STATIC_ORIGIN").unwrap_or_else(|_|http.origin.clone());
        let h = Harness::create_with_upload_root(&database.pool, &http.origin, vec![http.origin.clone(),old_origin.clone()], root.to_str()).await?;
        let mut ids = Vec::new();
        for n in 0..9 {
            let version = format!("zip-{n}");
            let (status, row) = zip_registration::upload(&h, ENVIRONMENT, &version, zip_registration::archive(format!("immutable-{n}").as_bytes(), false, "zip")?, "zip").await?;
            ensure!(status == StatusCode::OK, "ZIP upload failed");
            let id = release_id(&row)?; ids.push(id);
            // Fixture-only time travel, in the uniquely owned database. Restore
            // the immutable-definition trigger in the same transaction.
            let mut tx=database.pool.begin().await?;
            sqlx::query("ALTER TABLE application_releases DISABLE TRIGGER application_release_definition_immutable").execute(&mut *tx).await?;
            sqlx::query("UPDATE application_releases SET created_at=now()-make_interval(days => $2) WHERE id=$1").bind(id).bind(100+n).execute(&mut *tx).await?;
            sqlx::query("ALTER TABLE application_releases ENABLE TRIGGER application_release_definition_immutable").execute(&mut *tx).await?;
            tx.commit().await?;
        }
        // Both a pointer and a same-version reference protect otherwise old releases.
        sqlx::query("INSERT INTO application_deployments(application_id,environment,active_release_id,previous_release_id) VALUES($1,$2,$3,$4)")
            .bind(h.main_app).bind(ENVIRONMENT).bind(ids[8]).bind(ids[7]).execute(&database.pool).await?;
        let (status, _) = zip_registration::upload(&h, "staging", "zip-6", zip_registration::archive(b"immutable-6", false, "zip")?, "zip").await?;
        ensure!(status == StatusCode::OK);
        let client = reqwest::Client::builder().no_proxy().build()?;
        let url = format!("{old_origin}/micro/old-image/index.html");
        let old_bytes = client.get(&url).send().await?.error_for_status()?.bytes().await?;
        let mut body = release_body("old-image", "HTML");
        body["entryUrl"] = json!(url);
        let external = release_id(&h.register(body).await?)?;
        let before = snapshot(&database.pool).await?;
        let result = preview(&h).await?;
        ensure!(result["applyAvailable"] == false && result["deletionPermitted"] == false);
        ensure!(record(&result, ids[5])?["disposition"] == "MANUAL_REVIEW");
        for (id, reason) in [(ids[8], "ACTIVE"), (ids[7], "PREVIOUS"), (ids[6], "CROSS_ENVIRONMENT_REFERENCE"), (ids[0], "LATEST_VERSIONS"), (external, "EXTERNAL_OWNERSHIP_REQUIRES_REVIEW")] {
            ensure!(record(&result,id)?["reasons"].as_array().unwrap().contains(&json!(reason)), "missing {reason}");
        }
        ensure!(before == snapshot(&database.pool).await?, "preview modified a database table");
        let path = format!("/api/applications/{}/releases/retention-preview",h.main_app);
        for (method, suffix, token, expected) in [(Method::GET,"?minimumAgeDays=29",h.platform.as_str(),StatusCode::BAD_REQUEST),(Method::GET,"?apply=true",h.platform.as_str(),StatusCode::BAD_REQUEST),(Method::POST,"",h.platform.as_str(),StatusCode::METHOD_NOT_ALLOWED),(Method::GET,"",h.a.as_str(),StatusCode::FORBIDDEN)] {
            ensure!(call(&h.app,method,&format!("{path}{suffix}"),token,None).await?.0 == expected);
        }
        // Hold a real tracked operation on a deterministic barrier; cancellation
        // must leave durable protection, even while all pool connections are free.
        let config = Arc::new(Config::from_env());
        let state = AppState::new(database.pool.clone(),config);
        let (ready_tx,ready_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move { crate::release_operations::track(&state,h.main_app,"PREFLIGHT",async { let _=ready_tx.send(()); std::future::pending::<Result<(),crate::error::AppError>>().await }).await });
        ready_rx.await?;
        let pending_before = snapshot(&database.pool).await?;
        let pending = preview(&h).await?;
        ensure!(pending["manualReview"] == 0);
        ensure!(pending_before == snapshot(&database.pool).await?);
        task.abort(); let _=task.await;
        ensure!(preview(&h).await?["manualReview"] == 0, "cancellation lost protection");
        let uncertain_state=AppState::new(database.pool.clone(),Arc::new(Config::from_env()));
        let unknown=crate::release_operations::track(&uncertain_state,h.main_app,"ACTIVATE",async {Err::<(),_>(crate::error::AppError::ReleaseResultUnknown("owned fixture unknown result".into()))}).await;
        ensure!(matches!(unknown,Err(crate::error::AppError::ReleaseResultUnknown(_))));
        // Only this owned fixture reconciles its synthetic abandoned operation.
        let abandoned:Uuid=sqlx::query_scalar("SELECT id FROM application_release_operations WHERE application_id=$1 AND kind='PREFLIGHT'").bind(h.main_app).fetch_one(&database.pool).await?;
        sqlx::query("DELETE FROM application_release_operations WHERE id=$1").bind(abandoned).execute(&database.pool).await?;
        ensure!(preview(&h).await?["manualReview"] == 0,"unknown result lost independent protection");
        let unknown_id:Uuid=sqlx::query_scalar("SELECT id FROM application_release_operations WHERE application_id=$1 AND kind='ACTIVATE'").bind(h.main_app).fetch_one(&database.pool).await?;
        sqlx::query("DELETE FROM application_release_operations WHERE id=$1").bind(unknown_id).execute(&database.pool).await?;
        ensure!(preview(&h).await?["manualReview"].as_u64().is_some_and(|count|count>0));
        old_client(&h).await?;
        for n in 0..9 { ensure!(std::fs::read(root.join(format!("release-app/versions/zip-{n}/app.js")))? == format!("immutable-{n}").as_bytes()); }
        ensure!(client.get(&url).send().await?.error_for_status()?.bytes().await? == old_bytes);
        println!("PHASE31_RETENTION PASS all_tables_unchanged zip_bytes_unchanged old_url_200_bytes_unchanged pending_cancellation_protected");
        Ok(())
    }.await;
    ensure!(
        root.parent() == Some(std::env::temp_dir().as_path())
            && root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("zip-registration-")
    );
    let cleanup = std::fs::remove_dir_all(root);
    database
        .finish(result.and(cleanup.map_err(Into::into)))
        .await
}

async fn old_client(h: &Harness) -> Result<()> {
    let (origin, native_server, node_server) =
        if std::env::var_os("MICRO_PROXY_TEST_NODE").is_some() {
            let server = node_fixture::start_router(h.app.clone()).await?;
            (server.origin.clone(), None, Some(server))
        } else {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let origin = format!("http://{}", listener.local_addr()?);
            let app = h.app.clone();
            let server = tokio::spawn(async move { axum::serve(listener, app).await });
            (origin, Some(server), None)
        };
    let executable = std::env::var_os("MICRO_PROXY_TEST_NODE").unwrap_or_else(|| "node".into());
    // Exact historical adapter and its provenance live beside this test rather
    // than in the removed pre-split repository-level tests directory.
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/release_tests/fixtures/phase31/old-host.mjs");
    let mut command = tokio::process::Command::new(executable);
    command
        .arg(script)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("PHASE31_ORIGIN", origin)
        .env("PHASE31_TOKEN", &h.platform)
        .env("PHASE31_APP", h.main_app.to_string())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output()).await;
    if let Some(server) = native_server {
        server.abort();
        let _ = server.await;
    }
    if let Some(server) = node_server {
        server.close().await?;
    }
    let output = output.map_err(|_| anyhow!("old client timeout"))??;
    ensure!(
        output.status.success(),
        "old host adapter failed (child diagnostics suppressed)"
    );
    ensure!(String::from_utf8_lossy(&output.stdout).contains("PHASE31_OLD_HOST PASS"));
    println!("PHASE31_OLD_HOST PASS frozen_Phase30_adapter_real_register_list_deployments");
    Ok(())
}
