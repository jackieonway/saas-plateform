//! Shared real dual-application fixture. The before run is explicit and frozen.
use super::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

// Existing tree::build_tree iterates HashMaps, so sibling wire order is not a
// contract. Preserve every node/field/hierarchy while comparing canonical order.
fn canonical_menus(value: &mut Value) {
    if let Some(items) = value.as_array_mut() {
        for item in items.iter_mut() {
            canonical_menus(&mut item["children"]);
        }
        items.sort_by_key(|item| item["id"].as_str().unwrap_or_default().to_owned());
    }
}

// The runner binds each executable to its actual source snapshot and ELF hash.
async fn platform_process(
    database: &Database,
    fixture: &Fixture,
    client: &Client,
    executable_env: &str,
) -> Result<(tokio::process::Child, String)> {
    let binary = std::env::var_os(executable_env)
        .ok_or_else(|| anyhow!("verified platform binary required"))?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    drop(listener);
    let mut process = tokio::process::Command::new(binary)
        .env_clear()
        .env("APP_ENV", "test")
        .env("APPLICATION_ENVIRONMENT", "development")
        .env("DATABASE_URL", &database.url)
        .env("JWT_SECRET", SECRET)
        .env("DB_MAX_CONNECTIONS", "2")
        .env("DB_ACQUIRE_TIMEOUT_MS", "1000")
        .env("DB_STATEMENT_TIMEOUT_MS", "1000")
        .env("DB_LOCK_TIMEOUT_MS", "100")
        .env("BIND_ADDR", address.to_string())
        .env("DATA_GRANT_POLICY", fixture.policy.to_string())
        .env("OAUTH_INTROSPECTION_CLIENT_REQUESTS_PER_MINUTE", "20000")
        .env("OAUTH_INTROSPECTION_ATTEMPTS_PER_MINUTE", "30000")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let origin = format!("http://{address}");
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            ensure!(
                process.try_wait()?.is_none(),
                "original executable exited before readiness"
            );
            if client
                .get(format!("{origin}/api/health"))
                .send()
                .await
                .is_ok_and(|v| v.status().is_success())
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| anyhow!("original platform readiness timeout"))??;
    Ok((process, origin))
}

async fn combined(
    client: &Client,
    origin: &str,
    code: &str,
    token: &str,
    id: &str,
    secret: &str,
    resource: &str,
    action: &str,
) -> Result<(u16, Value)> {
    let response = client
        .post(format!("{origin}/api/apps/{code}/authorize"))
        .basic_auth(id, Some(secret))
        .form(&[("token", token), ("resource", resource), ("action", action)])
        .send()
        .await?;
    let status = response.status().as_u16();
    let body = serde_json::from_slice(&response.bytes().await?)?;
    Ok((status, body))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires managed disposable PostgreSQL; Phase30 live protocol"]
async fn phase30_live_protocol() -> Result<()> {
    live_protocol(true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires managed disposable PostgreSQL; current application protocol"]
async fn phase30_current_protocol() -> Result<()> {
    live_protocol(false).await
}

async fn live_protocol(original_executable: bool) -> Result<()> {
    let database = Database::create("PLATFORM").await?;
    let outcome=async {
        if original_executable {sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements").execute(&database.admin).await?;}
        sqlx::migrate!("./migrations").run(&database.pool).await?;
        let f=Fixture::prepare(&database.pool).await?;
        let state=AppState::new(database.pool.clone(),Arc::new(config(&f.policy)?));
        let mut server=Server::start(crate::routes::api_router().nest("/oauth",crate::routes::oauth2::oauth_router()).with_state(state.clone())).await?;
        let result=async {
            let client=Client::builder().timeout(Duration::from_secs(15)).no_proxy().build()?;
            let mut legacy=if original_executable {Some(platform_process(&database,&f,&client,"PHASE30_LEGACY_PLATFORM").await?)} else {None};
            let legacy_origin=legacy.as_ref().map(|(_,origin)|origin.clone()).unwrap_or_else(||server.origin.clone());
            let root:Uuid=sqlx::query_scalar("SELECT o.id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL").fetch_one(&database.pool).await?;
            let admin=login(&client,&server.origin,"admin","admin123",root).await?;
            grants(&client,&server.origin,&admin,&f.actor,f.app,&PAIRS,Some("ORG_SUBTREE")).await?;
            grants(&client,&server.origin,&admin,&f.actor,f.work_app,&[("ticket","read"),("ticket","create"),("ticket","update")],Some("ORG_SUBTREE")).await?;
            let mut tokens=Vec::new();
            for (code,id,secret,resource,app) in [("stores",&f.client_id,&f.client_secret,"store",f.app),("work-orders",&f.work_client_id,&f.work_client_secret,"ticket",f.work_app)] {
                let mut token=super::token(&client,&server.origin,&f.actor,code).await?;
                let mut old=request(&client,&legacy_origin,&format!("/api/apps/{code}/context"),&token,Method::GET,None).await?.require(200)?.clone();
                let (status,mut reply)=combined(&client,&server.origin,code,&token,id,secret,resource,"create").await?;
                ensure!(status==200,"combined {code} status {status}: {reply}");
                old.as_object_mut().unwrap().remove("expiresAt");
                reply["context"].as_object_mut().unwrap().remove("expiresAt");
                canonical_menus(&mut old["menus"]);canonical_menus(&mut reply["context"]["menus"]);
                ensure!(reply["contractVersion"]=="1.0" && reply["introspection"]["active"]==true && reply["context"]==old,"context parity {code}: differing keys {:?}; wrapper {}",old.as_object().unwrap().iter().filter(|(key,value)|reply["context"][key.as_str()]!=**value).map(|(key,_)|key).collect::<Vec<_>>(),reply["contractVersion"]);
                ensure!(combined(&client,&server.origin,code,&token,id,secret,"unregistered","read").await?.0==400,"unregistered accepted");
                ensure!(combined(&client,&server.origin,code,&token,id,"wrong",resource,"create").await?.0==401,"wrong client accepted");
                let version=old["contextVersion"].as_str().ok_or_else(||anyhow!("missing contextVersion"))?;
                ensure!(combined(&client,&server.origin,code,version,id,secret,resource,"create").await?.0==401,"context hash authenticated");
                for target in [&f.sibling,&f.foreign] {
                    let reply=request(&client,&server.origin,&format!("/api/apps/{code}/members?resource={resource}&action=create&organizationId={}&page=1&size=20",target.org),&token,Method::GET,None).await?;
                    ensure!(reply.status==404,"organization ceiling {code}");
                }
                // A committed change between first snapshot and fresh revalidation
                // must be visible; this uses separate autocommit SQL statements.
                let mut headers=axum::http::HeaderMap::new();
                headers.insert(axum::http::header::AUTHORIZATION,format!("Bearer {token}").parse()?);
                let snapshot=crate::app_authorization::request_context(&state,code,&headers).await?;
                grants(&client,&server.origin,&admin,&f.actor,app,&[(resource,"create")],None).await?;
                let changed=crate::app_authorization::revalidate_context(&state,code,&headers,snapshot.revision).await?;
                ensure!(changed.is_some(),"fresh check missed committed grant removal");
                ensure!(combined(&client,&server.origin,code,&token,id,secret,resource,"create").await?.0==403,"grant removal ignored");
                grants(&client,&server.origin,&admin,&f.actor,app,&[(resource,"create")],Some("ORG_SUBTREE")).await?;
                for scope in ["SELF","ORG","ORG_SUBTREE","TENANT"] {
                    grants(&client,&server.origin,&admin,&f.actor,app,&[(resource,"create")],Some(scope)).await?;
                    let mut before=request(&client,&legacy_origin,&format!("/api/apps/{code}/context"),&token,Method::GET,None).await?.require(200)?.clone();
                    let (status,mut after)=combined(&client,&server.origin,code,&token,id,secret,resource,"create").await?;
                    ensure!(status==200,"scope {scope} unexpectedly denied");
                    before.as_object_mut().unwrap().remove("expiresAt");after["context"].as_object_mut().unwrap().remove("expiresAt");
                    canonical_menus(&mut before["menus"]);canonical_menus(&mut after["context"]["menus"]);
                    ensure!(before==after["context"],"original/new scope {scope} differs for {code}");
                }
                grants(&client,&server.origin,&admin,&f.actor,app,&[(resource,"create")],Some("ORG_SUBTREE")).await?;
                sqlx::query("UPDATE roles SET status='DISABLED' WHERE id=$1").bind(f.actor.role).execute(&database.pool).await?;
                let context=request(&client,&legacy_origin,&format!("/api/apps/{code}/context"),&token,Method::GET,None).await?.require(200)?.clone();
                ensure!(context["permissions"].as_array().is_some_and(|p|p.is_empty()),"original role revocation ignored");
                let (denied,error)=combined(&client,&server.origin,code,&token,id,secret,resource,"create").await?;
                ensure!(denied==403 && error["code"]=="OPERATION_DENIED","role revocation/error semantic differs");
                sqlx::query("UPDATE roles SET status='ENABLED' WHERE id=$1").bind(f.actor.role).execute(&database.pool).await?;
                for (table,identity) in [("users",f.actor.user),("organizations",f.actor.org),("tenants",f.tenant),("applications",app)] {
                    sqlx::query(&format!("UPDATE {table} SET status='DISABLED' WHERE id=$1")).bind(identity).execute(&database.pool).await?;
                    let old_denial=request(&client,&legacy_origin,&format!("/api/apps/{code}/context"),&token,Method::GET,None).await?;
                    ensure!(old_denial.status==401 && combined(&client,&server.origin,code,&token,id,secret,resource,"create").await?.0==401,"live {table} revocation differs for {code}");
                    sqlx::query(&format!("UPDATE {table} SET status='ENABLED' WHERE id=$1")).bind(identity).execute(&database.pool).await?;
                    token=super::token(&client,&server.origin,&f.actor,code).await?;
                }
                for field in ["revoked","session"] {
                    let sql=if field=="revoked" {"UPDATE oauth_tokens SET revoked=true WHERE token_hash=$1"} else {"UPDATE platform_sessions SET revoked_at=now() WHERE id=(SELECT platform_session_id FROM oauth_tokens WHERE token_hash=$1)"};
                    sqlx::query(sql).bind(crate::util::hash::sha256_hex(&token)).execute(&database.pool).await?;
                    ensure!(request(&client,&legacy_origin,&format!("/api/apps/{code}/context"),&token,Method::GET,None).await?.status==401 && combined(&client,&server.origin,code,&token,id,secret,resource,"create").await?.0==401,"token/session revocation differs for {code}");
                    token=super::token(&client,&server.origin,&f.actor,code).await?;
                }
                for update in ["client_secret_hash='rotated'","status='DISABLED'","client_type='PUBLIC'"] {
                    sqlx::query(&format!("UPDATE oauth_clients SET {update} WHERE client_id=$1")).bind(id).execute(&database.pool).await?;
                    ensure!(combined(&client,&server.origin,code,&token,id,secret,resource,"create").await?.0==401,"client rotation/disable/public accepted");
                    sqlx::query("UPDATE oauth_clients SET client_secret_hash=$1,status='ENABLED',client_type='CONFIDENTIAL' WHERE client_id=$2").bind(crate::util::hash::sha256_hex(secret)).bind(id).execute(&database.pool).await?;
                    token=super::token(&client,&server.origin,&f.actor,code).await?;
                }
                let page=request(&client,&server.origin,"/api/users?page=9223372036854775807&size=200",&admin,Method::GET,None).await?;
                ensure!(page.status==200,"extreme platform page failed");

                tokens.push(token);
            }
            // Revocation triggers intentionally invalidate all sessions for the actor.
            // Fresh credentials are required; restoring status never restores old tokens.
            tokens.clear();
            for code in ["stores","work-orders"] {tokens.push(super::token(&client,&server.origin,&f.actor,code).await?);}
            for (code,id,secret,resource,wrong) in [("stores",&f.client_id,&f.client_secret,"store",&tokens[1]),("work-orders",&f.work_client_id,&f.work_client_secret,"ticket",&tokens[0])] {
                ensure!(combined(&client,&server.origin,code,wrong,id,secret,resource,"create").await?.0==401,"cross app token accepted");
            }
            let permits=state.app_authorization_slots.acquire_many(state.app_authorization_slots.available_permits() as u32).await?;
            ensure!(combined(&client,&server.origin,"stores",&tokens[0],&f.client_id,&f.client_secret,"store","create").await?.0==503,"saturated authorization accepted");
            drop(permits);
            let bounded=PgPoolOptions::new().max_connections(1).acquire_timeout(Duration::from_millis(100))
                .after_connect(|connection,_|Box::pin(async move {sqlx::query("SET statement_timeout=100").execute(&mut *connection).await?;sqlx::query("SET lock_timeout=50").execute(connection).await?;Ok(())})).connect(&database.url).await?;
            let timeout=sqlx::query("SELECT pg_sleep(1)").execute(&bounded).await.expect_err("SQL deadline missing");
            ensure!(timeout.as_database_error().and_then(|e|e.code()).as_deref()==Some("57014"),"unexpected SQL deadline error");
            let mut lock=database.pool.begin().await?;
            sqlx::query("LOCK TABLE memberships IN ACCESS EXCLUSIVE MODE").execute(&mut *lock).await?;
            let timeout=sqlx::query("SELECT 1 FROM memberships LIMIT 1").execute(&bounded).await.expect_err("lock deadline missing");
            ensure!(timeout.as_database_error().and_then(|e|e.code()).as_deref()==Some("55P03"),"unexpected lock deadline error");lock.rollback().await?;
            let lease=bounded.acquire().await?;
            ensure!(matches!(bounded.acquire().await,Err(sqlx::Error::PoolTimedOut)),"pool acquisition unbounded");drop(lease);
            let mut bounded_config=config(&f.policy)?;bounded_config.app_authorization_timeout_ms=1000;
            let bounded_state=AppState::new(bounded.clone(),Arc::new(bounded_config));
            let lease=bounded.acquire().await?;
            let mut headers=axum::http::HeaderMap::new();headers.insert(axum::http::header::AUTHORIZATION,format!("Bearer {}",tokens[0]).parse()?);
            let failure=crate::app_authorization::current_context(&bounded_state,"stores",&headers).await.expect_err("authorization should exhaust pool budget");
            ensure!(axum::response::IntoResponse::into_response(failure).status()==503,"pool exhaustion leaked 500");drop(lease);
            ensure!(sqlx::query_scalar::<_,i32>("SELECT 1").fetch_one(&bounded).await?==1,"pool failed to recover");bounded.close().await;
            println!("PHASE30_BUDGET_RESULT {{\"saturation503\":true,\"statementTimeout57014\":true,\"lockTimeout55P03\":true,\"poolTimeout\":true,\"poolRecovery\":true}}");
            if let Some((process,_))=&mut legacy {process.start_kill()?;process.wait().await?;}
            if original_executable {
                let (mut current,origin)=platform_process(&database,&f,&client,"PHASE30_CURRENT_PLATFORM").await?;
                let mut lock=database.pool.begin().await?;
                sqlx::query("LOCK TABLE memberships IN ACCESS EXCLUSIVE MODE").execute(&mut *lock).await?;
                let locked=combined(&client,&origin,"stores",&tokens[0],&f.client_id,&f.client_secret,"store","create").await?;
                ensure!(locked.0==503 && locked.1["code"]=="SERVICE_UNAVAILABLE","actual executable lock budget did not fail closed");
                lock.rollback().await?;
                for (index,(code,id,secret,resource)) in [("stores",&f.client_id,&f.client_secret,"store"),("work-orders",&f.work_client_id,&f.work_client_secret,"ticket")].into_iter().enumerate() {
                    ensure!(combined(&client,&origin,code,&tokens[index],id,secret,resource,"create").await?.0==200,"actual executable failed to recover");
                }
                current.start_kill()?;current.wait().await?;
                println!("PHASE30_EXECUTABLE_RESULT {{\"configuredLockBudget503\":true,\"dualAppRecovery\":true,\"processStopped\":true}}");
            }
            println!("PHASE30_LIVE_RESULT {}",json!({"applications":2,"protocol":"PASS","freshCommittedGrantRemoval":true,"hashIsNotCredential":true,"extremePaging":true,"originalExecutableCompared":original_executable}));

            Ok::<_,anyhow::Error>(())
        }.await;
        let stopped=server.stop().await; result?; ensure!(stopped,"server cleanup failed"); Ok::<_,anyhow::Error>(())
    }.await;
    let cleanup = database.cleanup(outcome.is_ok()).await;
    outcome?;
    cleanup?;
    Ok(())
}

async fn authorization_round(
    client: &Client,
    origin: &str,
    code: &str,
    token: &str,
    id: &str,
    secret: &str,
    merged: bool,
) -> Result<()> {
    if merged {
        let resource = if code == "stores" { "store" } else { "ticket" };
        let (status, body) =
            combined(client, origin, code, token, id, secret, resource, "create").await?;
        ensure!(
            status == 200 && body["introspection"]["active"] == true,
            "merged workload denied: {status}"
        );
    } else {
        let intro = client
            .post(format!("{origin}/oauth/introspect"))
            .basic_auth(id, Some(secret))
            .form(&[("token", token)])
            .send()
            .await?;
        ensure!(intro.status().is_success(), "legacy introspection failed");
        let body: Value = serde_json::from_slice(&intro.bytes().await?)?;
        ensure!(body["active"] == true, "inactive workload token");
        request(
            client,
            origin,
            &format!("/api/apps/{code}/context"),
            token,
            Method::GET,
            None,
        )
        .await?
        .require(200)?;
    }
    Ok(())
}

async fn sql_cost(database: &Database) -> Result<(i64, f64)> {
    Ok(sqlx::query_as("SELECT COALESCE(sum(calls),0)::bigint, COALESCE(sum(total_exec_time),0)::float8 FROM pg_stat_statements WHERE userid=(SELECT oid FROM pg_roles WHERE rolname=$1)")
        .bind(&database.role).fetch_one(&database.admin).await?)
}

async fn measure_old(
    database: &Database,
    server: &Server,
    client: &Client,
    code: &str,
    token: &str,
    client_id: &str,
    secret: &str,
    member_org: Uuid,
    combined_mode: bool,
) -> Result<Value> {
    for _ in 0..5 {
        authorization_round(
            client,
            &server.origin,
            code,
            token,
            client_id,
            secret,
            combined_mode,
        )
        .await?;
    }
    let resource = if code == "stores" { "store" } else { "ticket" };
    for _ in 0..3 {
        request(client,&server.origin,&format!("/api/apps/{code}/members?resource={resource}&action=create&organizationId={member_org}&page=1&size=20"),token,Method::GET,None).await?.require(200)?;
    }
    let before = sql_cost(database).await?;
    let busy = AtomicU32::new(0);
    let locks = AtomicU32::new(0);
    let started = Instant::now();
    let workload = async {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..4 {
            let client = client.clone();
            let origin = server.origin.clone();
            let code = code.to_owned();
            let token = token.to_owned();
            let cid = client_id.to_owned();
            let secret = secret.to_owned();
            tasks.spawn(async move {
                let mut timings = Vec::new();
                for _ in 0..20 {
                    let start = Instant::now();
                    authorization_round(
                        &client,
                        &origin,
                        &code,
                        &token,
                        &cid,
                        &secret,
                        combined_mode,
                    )
                    .await?;
                    timings.push(start.elapsed().as_secs_f64() * 1000.0);
                }
                Ok::<_, anyhow::Error>(timings)
            });
        }
        let mut timings = Vec::new();
        while let Some(result) = tasks.join_next().await {
            timings.extend(result??)
        }
        Ok::<_, anyhow::Error>(timings)
    };
    let monitor = async {
        loop {
            busy.fetch_max(
                database
                    .pool
                    .size()
                    .saturating_sub(database.pool.num_idle() as u32),
                Ordering::Relaxed,
            );
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname=$1 AND wait_event_type='Lock'",
            )
            .bind(&database.name)
            .fetch_one(&database.admin)
            .await?;
            locks.fetch_max(count as u32, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    let mut timings =
        tokio::select! {r=workload=>r?,r=monitor=>{r?;return Err(anyhow!("monitor stopped"))}};
    let duration = started.elapsed().as_secs_f64() * 1000.0;
    let after = sql_cost(database).await?;
    timings.sort_by(f64::total_cmp);
    ensure!(
        timings.len() == 80 && after.0 > before.0,
        "baseline workload incomplete"
    );
    let auth_sql = after.0 - before.0;
    let member_before = sql_cost(database).await?;
    let resource = if code == "stores" { "store" } else { "ticket" };
    let mut members = Vec::new();
    for _ in 0..20 {
        let start = Instant::now();
        request(client,&server.origin,&format!("/api/apps/{code}/members?resource={resource}&action=create&organizationId={member_org}&page=1&size=20"),token,Method::GET,None).await?.require(200)?;
        members.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let member_after = sql_cost(database).await?;
    Ok(
        json!({"app":code,"mode":if combined_mode {"combined"} else {"legacy-two-http"},"warmupAuthorizations":5,"warmupMemberRequests":3,"concurrency":4,"authorizations":80,"httpRequests":if combined_mode {80} else {160},
        "sqlCalls":auth_sql,"sqlExecMs":after.1-before.1,"elapsedMs":duration,"latencyMs":timings,
        "p95Ms":timings[75],"p99Ms":timings[79],"peakBusyPoolConnections":busy.load(Ordering::Relaxed),"peakDatabaseLockWaiters":locks.load(Ordering::Relaxed),
        "memberRequests":20,"memberSqlCalls":member_after.0-member_before.0,"memberLatencyMs":members}),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires managed disposable PostgreSQL with pg_stat_statements; explicit Phase30 baseline"]
async fn phase30_before_authorization_budget() -> Result<()> {
    ensure!(
        std::env::var("PHASE30_BASELINE").as_deref() == Ok("1"),
        "explicit baseline mode required"
    );
    authorization_budget(false, "PHASE30_DIAGNOSTIC_TWO_HTTP_RESULT").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires managed disposable PostgreSQL with pg_stat_statements; explicit Phase30 after budget"]
async fn phase30_after_authorization_budget() -> Result<()> {
    authorization_budget(true, "PHASE30_AFTER_RESULT").await?;
    authorization_budget(false, "PHASE30_AFTER_LEGACY_RESULT").await
}

async fn authorization_budget(combined_mode: bool, marker: &str) -> Result<()> {
    let database = Database::create("PLATFORM").await?;
    let outcome=async {
        sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements").execute(&database.admin).await?;
        sqlx::migrate!("./migrations").run(&database.pool).await?;
        let fixture=Fixture::prepare(&database.pool).await?;
        let state=AppState::new(database.pool.clone(),Arc::new(config(&fixture.policy)?));
        let mut server=Server::start(crate::routes::api_router().nest("/oauth",crate::routes::oauth2::oauth_router()).with_state(state)).await?;
        let result=async {
            let client=Client::builder().timeout(Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none()).no_proxy().build()?;
            let root:Uuid=sqlx::query_scalar("SELECT o.id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL").fetch_one(&database.pool).await?;
            let admin=login(&client,&server.origin,"admin","admin123",root).await?;
            grants(&client,&server.origin,&admin,&fixture.actor,fixture.app,&PAIRS,Some("ORG_SUBTREE")).await?;
            grants(&client,&server.origin,&admin,&fixture.actor,fixture.work_app,&[("ticket","read"),("ticket","create"),("ticket","update")],Some("ORG_SUBTREE")).await?;
            let mut observations=Vec::new();
            for (code,cid,secret) in [("stores",&fixture.client_id,&fixture.client_secret),("work-orders",&fixture.work_client_id,&fixture.work_client_secret)] {
                let app_token=token(&client,&server.origin,&fixture.actor,code).await?;
                observations.push(measure_old(&database,&server,&client,code,&app_token,cid,secret,fixture.actor.org,combined_mode).await?);
            }
            println!("{} {}",marker,json!({"poolMaxConnections":4,"samples":observations}));
            Ok::<_,anyhow::Error>(())
        }.await;
        let stopped=server.stop().await;result?;ensure!(stopped,"baseline server cleanup failed");Ok::<_,anyhow::Error>(())
    }.await;
    let cleanup = database.cleanup(outcome.is_ok()).await;
    outcome?;
    cleanup?;
    Ok(())
}
