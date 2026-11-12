use super::*;

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn release_removal_checks_references_permissions_and_audit_rollback() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let h = Harness::create(&database.pool, "http://127.0.0.1:7777").await?;
        let release = release_id(&h.register(release_body("remove-v1", "HTML")).await?)?;
        let path = format!("/api/applications/{}/releases/{release}", h.main_app);
        let (status, _) = call(&h.app, Method::DELETE, &path, &h.a, None).await?;
        ensure!(status == StatusCode::FORBIDDEN, "tenant removed a release");
        let (status, _) = call(&h.app, Method::DELETE,
            &format!("/api/applications/{}/releases/{release}", h.other_app), &h.platform, None).await?;
        ensure!(status == StatusCode::NOT_FOUND, "release crossed application boundary");

        sqlx::query("UPDATE application_releases SET validation_status='FAILED', checked_at=now() WHERE id=$1")
            .bind(release).execute(&database.pool).await?;
        let operation = crate::util::uuid7::new_uuid();
        sqlx::query("INSERT INTO application_release_operations(id,application_id,kind) VALUES($1,$2,'PREFLIGHT')")
            .bind(operation).bind(h.main_app).execute(&database.pool).await?;
        let (status, _) = call(&h.app, Method::DELETE, &path, &h.platform, None).await?;
        ensure!(status == StatusCode::CONFLICT, "pending operation was ignored");
        sqlx::query("DELETE FROM application_release_operations WHERE id=$1")
            .bind(operation).execute(&database.pool).await?;

        sqlx::raw_sql("CREATE FUNCTION reject_release_delete_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
            IF NEW.action='DELETE_APPLICATION_RELEASE' THEN RAISE EXCEPTION 'test audit failure'; END IF; RETURN NEW; END $$; \
            CREATE TRIGGER reject_release_delete_audit BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION reject_release_delete_audit();")
            .execute(&database.pool).await?;
        let (status, _) = call(&h.app, Method::DELETE, &path, &h.platform, None).await?;
        ensure!(status.is_server_error(), "audit failure did not reject deletion");
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM application_releases WHERE id=$1)")
            .bind(release).fetch_one(&database.pool).await?;
        ensure!(exists, "audit failure did not roll back deletion");
        sqlx::query("DROP TRIGGER reject_release_delete_audit ON audit_logs").execute(&database.pool).await?;

        successful_call(&h.app, Method::DELETE, &path, &h.platform, None).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM application_releases WHERE id=$1")
            .bind(release).fetch_one(&database.pool).await?;
        ensure!(count == 0, "failed release was not removed");
        let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE application_id=$1 AND action='DELETE_APPLICATION_RELEASE'")
            .bind(h.main_app).fetch_one(&database.pool).await?;
        ensure!(audits == 1, "deletion audit missing or duplicated");
        let (status, _) = call(&h.app, Method::DELETE, &path, &h.platform, None).await?;
        ensure!(status == StatusCode::NOT_FOUND, "duplicate removal was not reported");

        let current = release_id(&h.register(release_body("remove-current", "HTML")).await?)?;
        let previous = release_id(&h.register(release_body("remove-previous", "HTML")).await?)?;
        sqlx::query("INSERT INTO application_deployments(application_id,environment,active_release_id,previous_release_id) VALUES($1,$2,$3,$4)")
            .bind(h.main_app).bind(ENVIRONMENT).bind(current).bind(previous).execute(&database.pool).await?;
        for protected in [current, previous] {
            let (status, _) = call(&h.app, Method::DELETE,
                &format!("/api/applications/{}/releases/{protected}",h.main_app), &h.platform, None).await?;
            ensure!(status == StatusCode::CONFLICT, "active/previous release was removed");
        }
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM application_releases WHERE id=ANY($1)")
            .bind(vec![current,previous]).fetch_one(&database.pool).await?;
        ensure!(rows == 2, "protected records changed");

        let bound = release_id(&h.register(release_body("remove-bound", "HTML")).await?)?;
        let artifact = crate::util::uuid7::new_uuid();
        sqlx::query("INSERT INTO managed_image_artifacts(id,application_id,version,target_id,template_id,image_ref,policy_revision,public_url) \
            VALUES($1,$2,'remove-bound','fixture','fixture','example.invalid/app:fixture','fixture','https://example.invalid/')")
            .bind(artifact).bind(h.main_app).execute(&database.pool).await?;
        sqlx::query("INSERT INTO managed_deployment_bindings(release_id,artifact_id) VALUES($1,$2)")
            .bind(bound).bind(artifact).execute(&database.pool).await?;
        let (status, _) = call(&h.app, Method::DELETE,
            &format!("/api/applications/{}/releases/{bound}",h.main_app), &h.platform, None).await?;
        ensure!(status == StatusCode::CONFLICT, "managed binding was ignored");
        Ok(())
    }.await;
    database.finish(result).await
}
