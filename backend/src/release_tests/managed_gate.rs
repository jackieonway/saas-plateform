//! Managed delivery ownership survives configuration removal and alternate entry files.
use super::*;

#[tokio::test]
#[ignore = "requires owned disposable PostgreSQL"]
async fn failed_deployment_retry_cannot_reopen_a_cleanup_fence() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result = async {
        let origin = "https://cleanup-fence.test";
        let h = Harness::create(&database.pool, origin).await?;
        let system_app: Uuid = sqlx::query_scalar("SELECT application_id FROM menus WHERE permission='system:application'")
            .fetch_one(&database.pool).await?;
        let (tenant, org): (Uuid, Uuid) = sqlx::query_as(
            "SELECT t.id,o.id FROM tenants t JOIN organizations o ON o.tenant_id=t.id WHERE t.is_platform AND o.parent_id IS NULL"
        ).fetch_one(&database.pool).await?;
        let management = menu(&database.pool, system_app, "system:deployment:write").await?;
        grant_menu(&database.pool, tenant, org, None, management).await?;
        let actor: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username='release-platform'")
            .fetch_one(&database.pool).await?;
        let public = format!("{origin}/sub-apps/release-app/versions/v1/");
        let mut body = release_body("v1", "HTML");
        body["entryUrl"] = json!(&public);
        let release = release_id(&h.register(body).await?)?;
        let artifact: Uuid = sqlx::query_scalar(
            "INSERT INTO managed_image_artifacts(application_id,version,target_id,template_id,image_ref,policy_revision,public_url,state,id) \
             VALUES($1,'v1','edge','static','registry.test/demo@sha256:unused','policy',$2,'CLEANING',$3) RETURNING id"
        ).bind(h.main_app).bind(public).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        sqlx::query("INSERT INTO managed_deployment_bindings(release_id,artifact_id) VALUES($1,$2)")
            .bind(release).bind(artifact).execute(&database.pool).await?;
        let mut jobs = Vec::new();
        for operation in ["DEPLOY", "CLEANUP"] {
            let job: Uuid = sqlx::query_scalar(
                "INSERT INTO managed_deployment_jobs(artifact_id,release_id,request_id,payload_sha256,operation,state,attempt,error_code,created_by,id) \
                 VALUES($1,$2,$5,'intent',$3,'FAILED',1,'ENGINE_FAILED',$4,$6) RETURNING id"
            ).bind(artifact).bind(release).bind(operation).bind(actor).bind(crate::util::uuid7::new_uuid()).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
            jobs.push(job);
        }
        let (status, response) = call(
            &h.app, Method::POST,
            &format!("/api/applications/{}/managed-deployments/{}/retry", h.main_app, jobs[0]),
            &h.platform, None
        ).await?;
        ensure!(status == StatusCode::CONFLICT, "old deployment escaped cleanup fence: {response}");
        let states: (String, String) = sqlx::query_as(
            "SELECT j.state,a.state FROM managed_deployment_jobs j JOIN managed_image_artifacts a ON a.id=j.artifact_id WHERE j.id=$1"
        ).bind(jobs[0]).fetch_one(&database.pool).await?;
        ensure!(states == ("FAILED".into(), "CLEANING".into()), "rejected retry mutated persistent state");
        // The original cleanup remains recoverable; the fence must not block its retry.
        let (status, response) = call(
            &h.app, Method::POST,
            &format!("/api/applications/{}/managed-deployments/{}/retry", h.main_app, jobs[1]),
            &h.platform, None
        ).await?;
        ensure!(status == StatusCode::OK, "cleanup retry was blocked: {response}");
        let states: (String, String) = sqlx::query_as(
            "SELECT j.state,a.state FROM managed_deployment_jobs j JOIN managed_image_artifacts a ON a.id=j.artifact_id WHERE j.id=$1"
        ).bind(jobs[1]).fetch_one(&database.pool).await?;
        ensure!(states == ("QUEUED".into(), "CLEANING".into()), "cleanup retry lost its fence");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires owned disposable PostgreSQL"]
async fn managed_version_files_cannot_bypass_historical_readiness_or_cleanup() -> Result<()> {
    ensure!(
        std::env::var_os("DEPLOYMENT_CONFIG_FILE").is_none(),
        "run this historical-ownership regression without deployment target configuration"
    );
    let database = TestDatabase::create().await?;
    let result = async {
        let origin = "https://historical-delivery.test";
        let h = Harness::create(&database.pool, origin).await?;
        let root = format!("{origin}/sub-apps/release-app/versions/v1/");
        let artifact: Uuid = sqlx::query_scalar(
            "INSERT INTO managed_image_artifacts(application_id,version,target_id,template_id,image_ref,policy_revision,public_url,id) \
             VALUES($1,'v1','retired-target','static','registry.test/demo@sha256:unused','retired-policy',$2,$3) RETURNING id",
        ).bind(h.main_app).bind(&root).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;

        let mut body = release_body("v1", "HTML");
        body["entryUrl"] = json!(format!("{root}index.html"));
        let release_id = release_id(&h.register(body).await?)?;
        let mut row: crate::releases::ReleaseRow = sqlx::query_as(&format!(
            "{} FROM application_releases r WHERE r.id=$1",
            crate::releases::RELEASE_SELECT
        )).bind(release_id).fetch_one(&database.pool).await?;
        let mut config = Config::from_values(&Default::default())?;
        config.micro_gateway_origin = origin.into();
        config.micro_allowed_origins = vec![origin.into()];
        let state = AppState::new(database.pool.clone(), Arc::new(config));

        for suffix in ["", "index.html", "%69ndex.html", "assets/app.js"] {
            row.entry_url = format!("{root}{suffix}");
            let mut tx = database.pool.begin().await?;
            let outcome = crate::managed_deployments::require_ready(&state, &mut tx, &row).await;
            ensure!(
                matches!(outcome, Err(crate::error::AppError::Conflict(_))),
                "historical managed entry bypassed readiness: {}", row.entry_url
            );
            tx.rollback().await?;
        }
        // An unrelated version remains eligible for ordinary URL delivery.
        row.entry_url = format!("{origin}/sub-apps/release-app/versions/v10/index.html");
        let mut tx = database.pool.begin().await?;
        crate::managed_deployments::require_ready(&state, &mut tx, &row).await?;
        tx.rollback().await?;

        sqlx::query("UPDATE managed_image_artifacts SET state='CLEANING' WHERE id=$1")
            .bind(artifact).execute(&database.pool).await?;
        for (index, suffix) in ["", "index.html", "%69ndex.html", "assets/app.js"].iter().enumerate() {
            let mut body = release_body("v1", "HTML");
            body["entryUrl"] = json!(format!("{root}{suffix}"));
            body["environment"] = json!(format!("cleanup-{index}"));
            let (status, response) = call(
                &h.app, Method::POST, &format!("/api/applications/{}/releases",h.main_app),
                &h.platform, Some(body)
            ).await?;
            ensure!(status == StatusCode::CONFLICT, "cleaning entry accepted: {response}");
        }
        let references: i64 = sqlx::query_scalar("SELECT count(*) FROM application_releases WHERE application_id=$1")
            .bind(h.main_app).fetch_one(&database.pool).await?;
        ensure!(references == 1, "cleanup fence allowed new release references");
        Ok(())
    }.await;
    database.finish(result).await
}
