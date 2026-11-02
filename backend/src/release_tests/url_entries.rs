//! Real router + owned database regressions for direct URL configuration.
use super::*;

struct NodeFixture {
    child: std::process::Child,
    origin: String,
}

impl Drop for NodeFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl NodeFixture {
    async fn start() -> Result<Self> {
        // Windows may block inbound connections to newly linked Rust test
        // executables. Node is the existing development HTTP runtime; the test
        // never changes firewall rules or uses the shared running backend.
        let script = r#"
const http = require('node:http');
const server = http.createServer((req, res) => {
  const path = new URL(req.url, 'http://localhost').pathname;
  res.setHeader('Access-Control-Allow-Origin', '*');
  if (path.endsWith('.html')) {
    res.setHeader('Content-Type', 'text/html');
    res.end('<html><body><script src="./main.js"></script></body></html>');
  } else if (path.endsWith('.js')) {
    res.setHeader('Content-Type', 'application/javascript');
    res.end('window.orders={bootstrap:async function(){},mount:async function(){},unmount:async function(){}};');
  } else if (path.endsWith('/health')) {
    res.setHeader('Content-Type', 'application/json'); res.end('{"status":"UP"}');
  } else { res.statusCode=404; res.end('missing'); }
});
server.on('error', () => process.exit(1));
server.listen(0, '127.0.0.1', () => process.stdout.write('http://127.0.0.1:' + server.address().port + '\n'));
"#;
        let node = std::env::var_os("MICRO_PROXY_TEST_NODE").unwrap_or_else(|| "node".into());
        let mut command = std::process::Command::new(node);
        command
            .args(["-e", script])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut fixture = Self {
            child: command.spawn()?,
            origin: String::new(),
        };
        let output = fixture
            .child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Node fixture stdout unavailable"))?;
        let line = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                use std::io::BufRead;
                let mut reader = std::io::BufReader::new(output);
                let mut line = String::new();
                reader.read_line(&mut line)?;
                Ok::<_, std::io::Error>(line)
            }),
        )
        .await
        .map_err(|_| anyhow!("Node HTTP fixture startup timed out"))???;
        fixture.origin = line.trim().to_owned();
        ensure!(
            fixture.origin.starts_with("http://127.0.0.1:"),
            "Node HTTP fixture failed to start"
        );
        Ok(fixture)
    }
}

fn entry_path(h: &Harness, environment: &str) -> String {
    format!(
        "/api/applications/{}/url-entry?environment={environment}",
        h.main_app
    )
}

async fn save(h: &Harness, url: &str) -> Result<Value> {
    successful_call(
        &h.app,
        Method::PUT,
        &entry_path(h, ENVIRONMENT),
        &h.platform,
        Some(json!({"entryUrl":url})),
    )
    .await
}

async fn current_context(h: &Harness, app_token: &str) -> Result<Value> {
    successful_call(
        &h.app,
        Method::GET,
        "/api/apps/release-app/context",
        app_token,
        None,
    )
    .await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn url_entries_resolve_environment_and_revision_consistently() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let h = Harness::create(&database.pool,"https://apps.example.test").await?;

        let app_token = h.app_token(&h.a,"release-app").await?;
        ensure!(current_context(&h,&app_token).await?["application"]["entryUrl"] == "/legacy/release-app/index.html","legacy fallback changed before URL configuration");
        let prior_release:Uuid=sqlx::query_scalar("INSERT INTO application_releases(application_id,environment,version,entry_type,entry_url,validation_status,checked_at,id) \
            VALUES($1,'development','prior-v1','HTML','/micro/prior-v1/index.html','PASSED',now(),$2) RETURNING id")
            .bind(h.main_app).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        sqlx::query("INSERT INTO application_deployments(application_id,environment,active_release_id) VALUES($1,'development',$2)")
            .bind(h.main_app).bind(prior_release).execute(&database.pool).await?;
        let path = entry_path(&h,ENVIRONMENT);
        ensure!(successful_call(&h.app,Method::GET,&path,&h.platform,None).await?.is_null(),"missing URL entry was not null");
        let (status,_) = call(&h.app,Method::PUT,&path,&h.a,Some(json!({"entryUrl":"https://apps.example.test/"}))).await?;
        ensure!(status == StatusCode::FORBIDDEN,"tenant administrator configured a platform upstream");
        for invalid in ["https://denied.example.test/","https://user:secret@apps.example.test/","https://apps.example.test/#hash",
            "https://apps.example.test/?access_token=secret","https://apps.example.test/sub-apps/test/versions/v1/index.html",
            "https://apps.example.test/app"] {
            let (status,_) = call(&h.app,Method::PUT,&path,&h.platform,Some(json!({"entryUrl":invalid}))).await?;
            ensure!(status == StatusCode::BAD_REQUEST,"invalid direct entry accepted: {invalid}");
        }
        let first = save(&h,"https://apps.example.test/?theme=dark&next=a?b").await?;
        ensure!(first["proxyEnabled"] == true && first["revision"].is_string(),"default proxy/revision missing");
        let expected = format!("/micro-proxy/release-app/{}/?theme=dark&next=a?b",first["revision"].as_str().unwrap());
        ensure!(first["browserEntryUrl"] == expected,"wrong browser entry");
        let context = current_context(&h,&app_token).await?;
        let profile = h.profile(&h.a).await?;
        let app = profile_application(&profile,h.main_app)?;
        for field in ["entryUrl","entryVersionId","deploymentVersion","entryType","runtimeMode"] {
            ensure!(context["application"][field] == app[field],"profile/context differ in {field}");
        }
        ensure!(app["entryUrl"] == expected && app["entryVersionId"] == first["revision"] && app["deploymentVersion"].is_null(),"live entry was not authoritative");
        let setup = successful_call(&h.app,Method::GET,&format!("/api/applications/{}/setup?environment=development",h.main_app),&h.platform,None).await?;
        ensure!(setup["entrySource"] == "URL" && setup["urlEntryConfigured"] == true && setup["releaseValidated"] == true
            && setup["releaseRequired"] == false && setup["activeVersion"].is_null(),"live URL not ready in setup");
        let second = save(&h,"https://apps.example.test/other/").await?;
        ensure!(second["revision"] != first["revision"],"URL change did not invalidate entry identity");
        let repeat = save(&h,"https://apps.example.test/other/").await?;
        ensure!(repeat == second,"idempotent save changed revision or timestamps");
        for rejected in [format!("/micro-proxy/release-app/{}/",first["revision"].as_str().unwrap()),
            "/micro-proxy/release-app/".to_owned(),"/micro-proxy/release-app".to_owned()] {
            let response=h.app.clone().oneshot(Request::builder().uri(rejected).body(Body::empty())?).await?;
            ensure!(response.status() == StatusCode::NOT_FOUND,"stale or revisionless proxy route was accepted");
        }
        let stage = successful_call(&h.app,Method::PUT,&entry_path(&h,"staging"),&h.platform,
            Some(json!({"entryUrl":"https://apps.example.test/staging/","proxyEnabled":false}))).await?;
        ensure!(stage["browserEntryUrl"] == "https://apps.example.test/staging/","direct nonproxy URL changed");
        ensure!(current_context(&h,&app_token).await?["application"]["entryVersionId"] == second["revision"],"other environment leaked into context");
        let lookup = crate::application_entries::lookup_proxy_entry(&database.pool,"release-app",ENVIRONMENT).await?;
        ensure!(lookup.as_ref().is_some_and(|v|v.entry_url == "https://apps.example.test/other/"),"proxy did not use live origin");
        ensure!(crate::application_entries::lookup_proxy_entry(&database.pool,"release-app","staging").await?.is_none(),"nonproxy origin exposed via proxy");
        successful_call(&h.app,Method::DELETE,&path,&h.platform,None).await?;
        let response=h.app.clone().oneshot(Request::builder().uri(second["browserEntryUrl"].as_str().unwrap()).body(Body::empty())?).await?;
        ensure!(response.status() == StatusCode::NOT_FOUND,"deleted URL proxy remained available");
        let restored=current_context(&h,&app_token).await?;
        ensure!(restored["application"]["entryUrl"] == "/micro/prior-v1/index.html" && restored["application"]["entryVersionId"] == prior_release.to_string(),"delete did not restore active release");
        let (status,_) = call(&h.app,Method::PUT,&entry_path(&h,"production"),&h.platform,
            Some(json!({"entryUrl":"http://apps.example.test/"}))).await?;
        ensure!(status == StatusCode::BAD_REQUEST,"production accepted HTTP");
        let audits: (i64,i64) = sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE detail LIKE '%theme=dark%' OR detail LIKE '%apps.example.test%') \
            FROM audit_logs WHERE application_id=$1 AND action IN ('CONFIGURE_APPLICATION_URL_ENTRY','CLEAR_APPLICATION_URL_ENTRY')")
            .bind(h.main_app).fetch_one(&database.pool).await?;
        ensure!(audits == (4,0),"URL configuration audit missing, duplicated, or included raw URLs");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn url_entries_roll_back_when_audit_insert_is_rejected() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let h = Harness::create(&database.pool,"https://apps.example.test").await?;
        let original = save(&h,"https://apps.example.test/original/").await?;
        sqlx::raw_sql("CREATE FUNCTION reject_url_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
          IF NEW.action IN ('CONFIGURE_APPLICATION_URL_ENTRY','CLEAR_APPLICATION_URL_ENTRY') THEN RETURN NULL; END IF; RETURN NEW; END $$; \
          CREATE TRIGGER reject_url_audit BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION reject_url_audit();")
            .execute(&database.pool).await?;
        for (method,body) in [(Method::PUT,Some(json!({"entryUrl":"https://apps.example.test/rejected/"}))),(Method::DELETE,None)] {
            let (status,_) = call(&h.app,method,&entry_path(&h,ENVIRONMENT),&h.platform,body).await?;
            ensure!(status == StatusCode::INTERNAL_SERVER_ERROR,"audit rejection did not fail management request");
            let current = successful_call(&h.app,Method::GET,&entry_path(&h,ENVIRONMENT),&h.platform,None).await?;
            ensure!(current == original,"failed audit committed URL changes");
        }
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn url_entries_release_activation_and_rollback_clear_override_atomically() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let http = NodeFixture::start().await?;
        let h = Harness::create(&database.pool,&http.origin).await?;
        let v1 = release_id(&h.register(release_body("v1","HTML")).await?)?;
        let v2 = release_id(&h.register(release_body("v2","HTML")).await?)?;
        h.preflight(v1,"PASSED").await?;
        h.preflight(v2,"PASSED").await?;
        save(&h,&format!("{}/live/",http.origin)).await?;
        h.activate(v1).await?;
        ensure!(successful_call(&h.app,Method::GET,&entry_path(&h,ENVIRONMENT),&h.platform,None).await?.is_null(),"activation left live override active");
        save(&h,&format!("{}/live/",http.origin)).await?;
        h.activate(v2).await?;
        save(&h,&format!("{}/live/",http.origin)).await?;
        sqlx::raw_sql("CREATE FUNCTION reject_clear_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
          IF NEW.action='CLEAR_APPLICATION_URL_ENTRY' THEN RETURN NULL; END IF; RETURN NEW; END $$; \
          CREATE TRIGGER reject_clear_audit BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION reject_clear_audit();")
            .execute(&database.pool).await?;
        let rollback_path = format!("/api/applications/{}/deployments/development/rollback",h.main_app);
        let (status,_) = call(&h.app,Method::POST,&rollback_path,&h.platform,None).await?;
        ensure!(status == StatusCode::INTERNAL_SERVER_ERROR,"rollback ignored override audit failure");
        let active: Uuid = sqlx::query_scalar("SELECT active_release_id FROM application_deployments WHERE application_id=$1 AND environment='development'")
            .bind(h.main_app).fetch_one(&database.pool).await?;
        ensure!(active == v2 && !successful_call(&h.app,Method::GET,&entry_path(&h,ENVIRONMENT),&h.platform,None).await?.is_null(),"audit rejection committed partial rollback");
        sqlx::query("DROP TRIGGER reject_clear_audit ON audit_logs").execute(&database.pool).await?;
        let rolled = successful_call(&h.app,Method::POST,&rollback_path,&h.platform,None).await?;
        ensure!(rolled["activeReleaseId"] == v1.to_string(),"rollback selected wrong release");
        ensure!(successful_call(&h.app,Method::GET,&entry_path(&h,ENVIRONMENT),&h.platform,None).await?.is_null(),"rollback left override active");
        check_profile_entry(&h.profile(&h.a).await?,h.main_app,v1,"v1","HTML","/micro/v1/index.html")?;
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn url_entries_qiankun_trusted_html_upload_keeps_immutable_versions() -> Result<()> {
    let database = TestDatabase::create().await?;
    let root = zip_registration::owned_root()?;
    let result: Result<()> = async {
        let h = Harness::create_with_upload_root(
            &database.pool,
            "http://127.0.0.1:7777",
            vec!["http://127.0.0.1:7777".into()],
            root.to_str(),
        )
        .await?;
        successful_call(
            &h.app,
            Method::PUT,
            &format!("/api/applications/{}", h.main_app),
            &h.platform,
            Some(json!({"runtimeMode":"TRUSTED"})),
        )
        .await?;
        let (status, first) = zip_registration::upload(
            &h,
            "development",
            "qiankun-v1",
            zip_registration::archive(
                b"window.app={mount:function(){},unmount:function(){}}",
                false,
                "zip",
            )?,
            "zip",
        )
        .await?;
        ensure!(
            status == StatusCode::OK && first["artifactDigest"].is_string(),
            "TRUSTED HTML upload rejected: {first}"
        );
        let (status, _) = zip_registration::upload(
            &h,
            "staging",
            "qiankun-v1",
            zip_registration::archive(b"changed", false, "zip")?,
            "zip",
        )
        .await?;
        ensure!(
            status == StatusCode::CONFLICT,
            "trusted HTML upload replaced immutable bytes"
        );
        Ok(())
    }
    .await;
    // The root is a unique temp folder created above and owned only by this test.
    let _ = std::fs::remove_dir_all(&root);
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops isolated PostgreSQL databases using TEST_DATABASE_URL"]
async fn url_entries_recheck_revoked_management_permission_after_lock_wait() -> Result<()> {
    for method in [Method::PUT, Method::DELETE] {
        let database = TestDatabase::create().await?;
        let result:Result<()>=async {
            let h=Harness::create(&database.pool,"https://apps.example.test").await?;
            let original=save(&h,"https://apps.example.test/original/").await?;
            let mut lock=database.pool.begin().await?;
            sqlx::query_scalar::<_,Uuid>("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
                .bind(h.main_app).fetch_one(&mut *lock).await?;
            let app=h.app.clone();
            let token=h.platform.clone();
            let path=entry_path(&h,ENVIRONMENT);
            let request_method=method.clone();
            let pending=tokio::spawn(async move {
                let body=if request_method == Method::PUT {Some(json!({"entryUrl":"https://apps.example.test/rejected/"}))} else {None};
                call(&app,request_method,&path,&token,body).await
            });
            let waiting=tokio::time::timeout(std::time::Duration::from_secs(5),async {
                loop {
                    let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                        AND pid<>pg_backend_pid() AND wait_event_type='Lock' AND query LIKE '%FROM applications WHERE id=$1 FOR UPDATE%')")
                        .fetch_one(&database.pool).await?;
                    if blocked { break Ok::<_,anyhow::Error>(()); }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }).await;
            if !matches!(waiting,Ok(Ok(()))) {
                pending.abort();
                return Err(anyhow!("URL mutation did not reach its application lock"));
            }
            // Even a super admin must observe the platform organization cap.
            let removed=sqlx::query("DELETE FROM organization_menus om USING menus m WHERE om.menu_id=m.id AND m.permission='system:application'")
                .execute(&database.pool).await?;
            ensure!(removed.rows_affected()>0,"fixture permission was not revoked");
            lock.commit().await?;
            let (status,_)=pending.await??;
            ensure!(status == StatusCode::FORBIDDEN,"{method} ignored permission revocation during its lock wait");
            let stored:(String,Uuid)=sqlx::query_as("SELECT entry_url,revision FROM application_url_entries WHERE application_id=$1 AND environment=$2")
                .bind(h.main_app).bind(ENVIRONMENT).fetch_one(&database.pool).await?;
            ensure!(stored.0 == original["entryUrl"].as_str().unwrap() && stored.1.to_string() == original["revision"].as_str().unwrap(),"rejected request changed the URL entry");
            let audits:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE application_id=$1 AND action IN ('CONFIGURE_APPLICATION_URL_ENTRY','CLEAR_APPLICATION_URL_ENTRY')")
                .bind(h.main_app).fetch_one(&database.pool).await?;
            ensure!(audits == 1,"revoked request persisted an audit or changed entry state");
            Ok(())
        }.await;
        database.finish(result).await?;
    }
    Ok(())
}
