//! qiankun-only enforcement and a preserving upgrade of historical metadata.
use super::*;

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn qiankun_only_apis_require_explicit_reconfiguration_and_archive_esm_releases() -> Result<()>
{
    let database = TestDatabase::create().await?;
    let result:Result<()>=async {
        let h=Harness::create(&database.pool,"https://apps.example.test").await?;
        for (kind,mode) in [("ESM","TRUSTED"),("HTML","IFRAME")] {
            let (status,_)=call(&h.app,Method::POST,"/api/applications",&h.platform,
                Some(json!({"code":"rejected-micro","name":"Rejected micro","entryType":kind,"runtimeMode":mode,"basePath":"/micro/rejected-micro","entryUrl":"/legacy/index.html"}))).await?;
            ensure!(status == StatusCode::BAD_REQUEST,"new legacy micro configuration was accepted");
        }
        let created=successful_call(&h.app,Method::POST,"/api/applications",&h.platform,
            Some(json!({"code":"new-qiankun","name":"New qiankun"}))).await?;
        ensure!(created["entryType"] == "HTML" && created["runtimeMode"] == "TRUSTED","API defaults are not qiankun");
        let builtin=successful_call(&h.app,Method::POST,"/api/applications",&h.platform,
            Some(json!({"code":"builtin-page","name":"Built in page","basePath":"/system-extra"}))).await?;
        ensure!(builtin["entryUrl"] == "","built-in route inherited a fake micro entry");
        successful_call(&h.app,Method::PUT,&format!("/api/applications/{}",builtin["id"].as_str().unwrap()),&h.platform,
            Some(json!({"name":"Renamed built in page"}))).await?;
        sqlx::query("UPDATE applications SET runtime_mode='IFRAME',status='DISABLED' WHERE id=$1")
            .bind(h.main_app).execute(&database.pool).await?;
        let url_path=format!("/api/applications/{}/url-entry?environment=development",h.main_app);
        let (status,_)=call(&h.app,Method::PUT,&url_path,&h.platform,Some(json!({"entryUrl":"https://apps.example.test/"}))).await?;
        ensure!(status == StatusCode::BAD_REQUEST,"URL save silently granted trust to historical iframe");
        let app_path=format!("/api/applications/{}",h.main_app);
        let (status,_)=call(&h.app,Method::PUT,&app_path,&h.platform,Some(json!({"status":"ENABLED"}))).await?;
        ensure!(status == StatusCode::BAD_REQUEST,"legacy loader was reenabled");
        let setup=successful_call(&h.app,Method::GET,&format!("/api/applications/{}/setup?environment=development",h.main_app),&h.platform,None).await?;
        ensure!(setup["releaseValidated"] == false,"legacy iframe reported ready");
        let repaired=successful_call(&h.app,Method::PUT,&app_path,&h.platform,
            Some(json!({"entryType":"HTML","runtimeMode":"TRUSTED","entryUrl":"https://apps.example.test/","status":"DISABLED"}))).await?;
        ensure!(repaired["status"] == "DISABLED","reconfiguration implicitly enabled application");
        successful_call(&h.app,Method::PUT,&url_path,&h.platform,Some(json!({"entryUrl":"https://apps.example.test/"}))).await?;
        // Reconstruct a pre-upgrade archive without permitting it in the
        // current schema or API. The new check remains enforced for new rows.
        sqlx::query("ALTER TABLE application_releases DROP CONSTRAINT application_releases_entry_type_check").execute(&database.pool).await?;
        let old:Uuid=sqlx::query_scalar("INSERT INTO application_releases(application_id,environment,version,entry_type,entry_url,validation_status,checked_at,id) \
            VALUES($1,'development','archive-v1','ESM','/legacy/archive-v1/main.js','PASSED',now(),$2) RETURNING id")
            .bind(h.main_app).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        sqlx::query("ALTER TABLE application_releases ADD CONSTRAINT application_releases_entry_type_check CHECK(entry_type='HTML') NOT VALID").execute(&database.pool).await?;
        for action in ["preflight","activate"] {
            let (status,_)=call(&h.app,Method::POST,&format!("/api/applications/{}/releases/{old}/{action}",h.main_app),&h.platform,None).await?;
            ensure!(status == StatusCode::CONFLICT,"archived ESM release accepted {action}");
        }
        let archived=successful_call(&h.app,Method::GET,&format!("/api/applications/{}/releases",h.main_app),&h.platform,None).await?;
        ensure!(archived.as_array().is_some_and(|rows|rows.iter().any(|row|row["id"] == old.to_string())),"historical release disappeared from inventory");
        let (status,_)=call(&h.app,Method::POST,&format!("/api/applications/{}/releases",h.main_app),&h.platform,
            Some(json!({"environment":"development","version":"new-esm","entryType":"ESM","entryUrl":"https://apps.example.test/new-esm/main.js"}))).await?;
        ensure!(status == StatusCode::BAD_REQUEST,"new ESM release was registered");
        let still:(String,Option<chrono::DateTime<chrono::Utc>>)=sqlx::query_as("SELECT validation_status,checked_at FROM application_releases WHERE id=$1")
            .bind(old).fetch_one(&database.pool).await?;
        ensure!(still.0 == "PASSED" && still.1.is_some(),"archived release was mutated by rejected preflight");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn qiankun_only_unifies_builtin_metadata_and_ignores_archived_builtin_release() -> Result<()>
{
    let database = TestDatabase::create_through(9).await?;
    let result:Result<()>=async {
        let mut h=Harness::create(&database.pool,"https://apps.example.test").await?;
        let builtin:Uuid=sqlx::query_scalar("INSERT INTO applications(code,name,base_path,entry_type,runtime_mode,entry_url,id) VALUES('old-builtin','Preserved built in','/system-old','ESM','IFRAME','/src/old-builtin.tsx',$1) RETURNING id")
            .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let null_builtin:Uuid=sqlx::query_scalar("INSERT INTO applications(code,name,base_path,entry_type,runtime_mode,entry_url,id) VALUES('null-builtin','Null path builtin',NULL,'ESM','IFRAME','/src/null-builtin.tsx',$1) RETURNING id")
            .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let html_builtin:Uuid=sqlx::query_scalar("INSERT INTO applications(code,name,base_path,entry_type,runtime_mode,entry_url,id) VALUES('html-builtin','HTML metadata','/system-html','HTML','TRUSTED','/preserved/index.html',$1) RETURNING id")
            .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let menu:Uuid=sqlx::query_scalar("INSERT INTO menus(application_id,title,path,id) VALUES($1,'Preserved menu','/system-old',$2) RETURNING id")
            .bind(builtin).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let historical_micro:Uuid=sqlx::query_scalar("INSERT INTO applications(code,name,base_path,entry_type,runtime_mode,entry_url,status,id) VALUES('disabled-history','Disabled history','/micro/disabled-history','ESM','IFRAME','/src/history.tsx','DISABLED',$1) RETURNING id")
            .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        // Reconstruct one already-existing archive, then restore v9's exact
        // new-write constraint before exercising the real incremental upgrade.
        sqlx::query("ALTER TABLE application_releases DROP CONSTRAINT application_releases_entry_type_check").execute(&database.pool).await?;
        let archived:Uuid=sqlx::query_scalar("INSERT INTO application_releases(application_id,environment,version,entry_type,entry_url,validation_status,checked_at,id) VALUES($1,'development','old-v1','ESM','/old/old-v1/entry.js','PASSED',now(),$2) RETURNING id")
            .bind(builtin).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let null_archived:Uuid=sqlx::query_scalar("INSERT INTO application_releases(application_id,environment,version,entry_type,entry_url,validation_status,checked_at,id) VALUES($1,'development','old-v1','ESM','/old/old-v1/null-entry.js','PASSED',now(),$2) RETURNING id")
            .bind(null_builtin).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        sqlx::query("ALTER TABLE application_releases ADD CONSTRAINT application_releases_entry_type_check CHECK(entry_type='HTML') NOT VALID").execute(&database.pool).await?;
        sqlx::query("INSERT INTO application_deployments(application_id,environment,active_release_id) VALUES($1,'development',$2)")
            .bind(builtin).bind(archived).execute(&database.pool).await?;
        sqlx::query("INSERT INTO application_deployments(application_id,environment,active_release_id) VALUES($1,'development',$2)")
            .bind(null_builtin).bind(null_archived).execute(&database.pool).await?;
        let (tenant,org):(Uuid,Uuid)=sqlx::query_as("SELECT o.tenant_id,o.id FROM organizations o JOIN memberships m ON m.organization_id=o.id JOIN users u ON u.id=m.user_id WHERE u.username='release-admin-a'")
            .fetch_one(&database.pool).await?;
        grant_application(&database.pool,tenant,org,builtin).await?;
        grant_application(&database.pool,tenant,org,null_builtin).await?;
        // Exercise exactly v10 first, with its real migration record. Current
        // HTTP adapters need the later identity columns, so upgrade to the full
        // schema only after asserting v10's historical-data contract below.
        let v10 = sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                sqlx::migrate!("./migrations").iter()
                    .filter(|migration| migration.version <= 10).cloned().collect(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        v10.run(&database.pool).await?;
        let actual:(String,String,String,String,String,String)=sqlx::query_as("SELECT name,base_path,status,entry_type,runtime_mode,entry_url FROM applications WHERE id=$1")
            .bind(builtin).fetch_one(&database.pool).await?;
        ensure!(actual == ("Preserved built in".into(),"/system-old".into(),"ENABLED".into(),"HTML".into(),"TRUSTED".into(),"".into()),"built-in identity/status was lost or metadata remained legacy");
        let null_actual:(Option<String>,String,String,String,String)=sqlx::query_as("SELECT base_path,status,entry_type,runtime_mode,entry_url FROM applications WHERE id=$1")
            .bind(null_builtin).fetch_one(&database.pool).await?;
        ensure!(null_actual == (None,"ENABLED".into(),"HTML".into(),"TRUSTED".into(),"".into()),"NULL-base builtin was skipped or its raw path changed");
        let preserved_html:String=sqlx::query_scalar("SELECT entry_url FROM applications WHERE id=$1")
            .bind(html_builtin).fetch_one(&database.pool).await?;
        ensure!(preserved_html == "/preserved/index.html","upgrade erased non-ESM HTML metadata");
        let retained:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM menus WHERE id=$1) AND EXISTS(SELECT 1 FROM application_releases WHERE id=$2 AND entry_type='ESM')")
            .bind(menu).bind(archived).fetch_one(&database.pool).await?;
        ensure!(retained,"upgrade deleted built-in menus or archive");
        let legacy:(String,String,String,String)=sqlx::query_as("SELECT status,entry_type,runtime_mode,entry_url FROM applications WHERE id=$1")
            .bind(historical_micro).fetch_one(&database.pool).await?;
        ensure!(legacy == ("DISABLED".into(),"ESM".into(),"IFRAME".into(),"/src/history.tsx".into()),"upgrade promoted or erased disabled micro history");
        sqlx::migrate!("./migrations").run(&database.pool).await?;
        ensure!(call(&h.app,Method::GET,"/api/auth/profile",&h.a,None).await?.0 == StatusCode::UNAUTHORIZED,
            "upgrade accepted a historical session without signing-time identity proof");
        // The data contract below uses newly issued fixture sessions. The old
        // credentials stay revoked; the fixture never adds proof to those rows.
        let account: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username='release-admin-a'")
            .fetch_one(&database.pool).await?;
        h.a=session(&database.pool,account,tenant,org).await?;
        let (account,platform_tenant,platform_org): (Uuid,Uuid,Uuid) = sqlx::query_as(
            "SELECT u.id,o.tenant_id,o.id FROM users u JOIN memberships m ON m.user_id=u.id JOIN organizations o ON o.id=m.organization_id WHERE u.username='release-platform' AND m.member_type='EMPLOYEE'",
        ).fetch_one(&database.pool).await?;
        h.platform=session(&database.pool,account,platform_tenant,platform_org).await?;
        let profile=h.profile(&h.a).await?;
        let app=profile_application(&profile,builtin)?;
        ensure!(app["entryType"] == "HTML" && app["runtimeMode"] == "TRUSTED" && app["entryUrl"] == "" && app["entryVersionId"].is_null()
            && app["deploymentVersion"].is_null(),"archived built-in ESM release overrode unified metadata");
        let null_app=profile_application(&profile,null_builtin)?;
        ensure!(null_app["entryType"] == "HTML" && null_app["runtimeMode"] == "TRUSTED" && null_app["entryUrl"] == "" && null_app["entryVersionId"].is_null(),"NULL-base builtin resolved an archived ESM entry");
        for status in ["ENABLED","DISABLED"] {
            for (kind,mode) in [("ESM","TRUSTED"),("HTML","IFRAME")] {
                let (response,_)=call(&h.app,Method::POST,"/api/applications",&h.platform,
                    Some(json!({"code":"unsupported-built","name":"Unsupported built in","basePath":"/native-reject","entryType":kind,"runtimeMode":mode,"status":status}))).await?;
                ensure!(response == StatusCode::BAD_REQUEST,"API accepted legacy built-in configuration");
            }
        }
        let path=format!("/api/applications/{builtin}");
        for patch in [json!({"entryType":"ESM"}),json!({"runtimeMode":"IFRAME"})] {
            ensure!(call(&h.app,Method::PUT,&path,&h.platform,Some(patch)).await?.0 == StatusCode::BAD_REQUEST,"update accepted a built-in legacy exception");
        }
        let updated=successful_call(&h.app,Method::PUT,&path,&h.platform,Some(json!({"name":"Updated native page","entryUrl":""}))).await?;
        ensure!(updated["entryType"] == "HTML" && updated["runtimeMode"] == "TRUSTED" && updated["entryUrl"] == "","ordinary built-in metadata could not keep an empty URL");
        expect_sqlstate(sqlx::query("INSERT INTO applications(code,name,base_path,entry_type,runtime_mode,id) VALUES('db-old-built','DB old builtin','/native-old','ESM','TRUSTED',$1)")
            .bind(crate::util::uuid7::new_uuid()).execute(&database.pool).await,"23514","enabled legacy built-in database insert")?;
        Ok(())
    }.await;
    database.finish(result).await
}
