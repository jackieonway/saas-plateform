//! Owned three-database capacity experiment; no production SLA assertion.
use super::*;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};
const METRICS: &str = "phase32-owned-operational-canary";

fn resources(pid: u32) -> Value {
    let read =
        |name: &str| std::fs::read_to_string(format!("/proc/{pid}/{name}")).unwrap_or_default();
    let status = read("status");
    json!({"stat":read("stat").split_whitespace().skip(13).take(2).collect::<Vec<_>>(),"memory":status.lines().filter(|s|s.starts_with("VmRSS:")||s.starts_with("VmHWM:")).collect::<Vec<_>>()})
}
async fn scrape(c: &Client, origin: &str, prefix: &str) -> Result<String> {
    let url = format!("{origin}{prefix}/internal/metrics");
    ensure!(c.get(&url).send().await?.status() == 401);
    let response = c.get(url).bearer_auth(METRICS).send().await?;
    ensure!(
        response.status() == 200,
        "private scrape at {prefix} returned {}",
        response.status().as_u16()
    );
    let body = response.text().await?;
    ensure!(!body.contains(METRICS) && !body.contains("user_id=") && !body.contains("token="));
    Ok(body)
}
async fn seed(d: &[Database], f: &Fixture) -> Result<()> {
    for (index, m) in [&f.actor, &f.child, &f.sibling, &f.foreign]
        .into_iter()
        .enumerate()
    {
        sqlx::query("INSERT INTO work_orders(id,number,tenant_id,organization_id,organization_name,owner_user_id,owner_display_name,title,description,priority,status,created_at,deleted_at) SELECT gen_random_uuid(),$4||g::text,$1,$2,'owned',$3,'owned','Representative ticket '||g,repeat('bounded workload ',8),CASE WHEN g%3=0 THEN 'HIGH' WHEN g%3=1 THEN 'LOW' ELSE 'NORMAL' END,CASE WHEN g%3=0 THEN 'RESOLVED' WHEN g%3=1 THEN 'IN_PROGRESS' ELSE 'OPEN' END,now()-g*interval '1 minute',CASE WHEN g%20=0 THEN now() ELSE NULL END FROM generate_series(1,12500) g")
            .bind(m.tenant).bind(m.org).bind(m.user).bind(format!("P32-{index}-")).execute(&d[2].pool).await?;
        let store = Uuid::new_v4();
        sqlx::query("INSERT INTO stores(id,tenant_id,organization_id,owner_user_id,name,business_status,business_contact) VALUES($1,$2,$3,$4,'Owned benchmark store','OPEN','local')").bind(store).bind(m.tenant).bind(m.org).bind(m.user).execute(&d[1].pool).await?;
        sqlx::query("INSERT INTO store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount,note) SELECT gen_random_uuid(),$1,$2,$3,$4,DATE '1990-01-01'+g,(g%100000)::numeric/100,repeat('bounded workload ',8) FROM generate_series(1,12500) g").bind(m.tenant).bind(store).bind(m.org).bind(m.user).execute(&d[1].pool).await?;
    }
    for (db, table) in [(&d[1], "store_daily_reports"), (&d[2], "work_orders")] {
        sqlx::query(&format!("ANALYZE {table}"))
            .execute(&db.pool)
            .await?;
        ensure!(
            sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&db.pool)
                .await?
                == 50000
        );
    }
    Ok(())
}

async fn monitor_faults(c: &Client, db: &Database, origin: &str) -> Result<Value> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    drop(listener);
    let file = std::env::temp_dir().join(format!("phase32-monitor-{}.json", Uuid::new_v4()));
    std::fs::write(&file,json!({"bind":address.to_string(),"interval_ms":100,"timeout_ms":500,"failures":2,"recoveries":2,"metrics_token":METRICS,"event_token":"phase32-owned-event-canary","targets":[{"component":"stores","url":format!("{origin}/stores-api/health/ready")}]}).to_string())?;
    let mut monitor = tokio::process::Command::new(
        std::env::var_os("MONITOR_BINARY").context("monitor binary required")?,
    )
    .arg(&file)
    .env_clear()
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .kill_on_drop(true)
    .spawn()?;
    let stream = monitor.stdout.take().context("monitor stdout")?;
    let logs = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stream
            .take(65537)
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    });
    let monitor_origin = format!("http://{address}");
    async fn wait_metric(c: &Client, origin: &str, needle: &str) -> Result<String> {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(r) = c
                    .get(format!("{origin}/internal/metrics"))
                    .bearer_auth(METRICS)
                    .send()
                    .await
                {
                    let text = r.text().await?;
                    if text.contains(needle) {
                        return Ok::<_, anyhow::Error>(text);
                    }
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("monitor state deadline"))?
    }
    let result = async {
        wait_metric(c, &monitor_origin, "component=\"stores\",state=\"up\"} 1").await?;
        ensure!(
            c.get(format!("{monitor_origin}/internal/metrics"))
                .send()
                .await?
                .status()
                == 401
        );
        let mut snapshots = Vec::new();
        for count in 1..=2 {
            // A real missing business relation invalidates schema readiness. Restore
            // the same relation, with no migrations or production data involved.
            sqlx::query("ALTER TABLE stores RENAME TO phase32_fault_stores")
                .execute(&db.pool)
                .await?;
            let down = wait_metric(
                c,
                &monitor_origin,
                &format!("component=\"stores\",event=\"DOWN\"}} {count}"),
            )
            .await;
            // Restore even if observation failed.
            sqlx::query("ALTER TABLE phase32_fault_stores RENAME TO stores")
                .execute(&db.pool)
                .await?;
            snapshots.push(down?);
            snapshots.push(
                wait_metric(
                    c,
                    &monitor_origin,
                    &format!("component=\"stores\",event=\"RECOVERED\"}} {count}"),
                )
                .await?,
            );
        }
        ensure!(
            c.post(format!("{monitor_origin}/internal/events"))
                .bearer_auth(METRICS)
                .json(&json!({"component":"backup_run","healthy":false,"sequence":1}))
                .send()
                .await?
                .status()
                == 401
        );
        for component in ["backup_age", "backup_run", "backup_verify"] {
            for sequence in 1..=4 {
                ensure!(c
                    .post(format!("{monitor_origin}/internal/events"))
                    .bearer_auth("phase32-owned-event-canary")
                    .json(&json!({"component":component,"healthy":sequence>2,"sequence":sequence}))
                    .send()
                    .await?
                    .status()
                    .is_success());
            }
            ensure!(
                c.post(format!("{monitor_origin}/internal/events"))
                    .bearer_auth("phase32-owned-event-canary")
                    .json(&json!({"component":component,"healthy":true,"sequence":1}))
                    .send()
                    .await?
                    .status()
                    == 409
            );
        }
        Ok::<_, anyhow::Error>(snapshots)
    }
    .await;
    let _ = monitor.kill().await;
    let _ = monitor.wait().await;
    let _ = std::fs::remove_file(&file);
    let bytes = tokio::time::timeout(Duration::from_secs(5), logs).await???;
    ensure!(bytes.len() <= 65536);
    let events = String::from_utf8(bytes)?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let snapshots = result?;
    ensure!(
        events
            .iter()
            .filter(|e| e["component"] == "stores" && e["event"] == "DOWN")
            .count()
            == 2
    );
    ensure!(
        events
            .iter()
            .filter(|e| e["component"] == "stores" && e["event"] == "RECOVERED")
            .count()
            == 2
    );
    Ok(
        json!({"events":events,"snapshots":snapshots,"fault":"rename owned business table; readiness schema check fails; restore same table","externalDelivery":false,"backupImplementation":false}),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires owned PostgreSQL, explicit binaries and Phase32 opt-in"]
async fn phase32_capacity_and_locks() -> Result<()> {
    ensure!(std::env::var("PHASE32_BASELINE").as_deref() == Ok("1"));
    let mut dbs = Vec::new();
    for kind in ["PLATFORM", "STORES", "WORK_ORDERS"] {
        match Database::create(kind).await {
            Ok(d) => dbs.push(d),
            Err(e) => {
                for d in dbs.into_iter().rev() {
                    let _ = d.cleanup(false).await;
                }
                return Err(e);
            }
        }
    }
    let outcome=async {
        sqlx::migrate!("./migrations").run(&dbs[0].pool).await?;
        let f=Fixture::prepare(&dbs[0].pool).await?;
        // Bootstrap the real 665,923-row reference catalog before measured
        // process startup; runtime statement timeouts remain at production defaults.
        crate::region_seed::initialize(&dbs[0].pool).await?;
        let c=Client::builder().no_proxy().timeout(Duration::from_secs(20)).build()?;
        let listener=TcpListener::bind("127.0.0.1:0").await?;let addr=listener.local_addr()?;drop(listener);
        let mut platform=tokio::process::Command::new(std::env::var_os("PLATFORM_BINARY").context("platform binary required")?)
            .env_clear().env("APP_ENV","test").env("APPLICATION_ENVIRONMENT","development").env("DATABASE_URL",&dbs[0].url)
            .env("JWT_SECRET",SECRET).env("METRICS_TOKEN",METRICS).env("DB_MAX_CONNECTIONS","10")
            .env("BIND_ADDR",addr.to_string()).env("DATA_GRANT_POLICY",f.policy.to_string())
            .env("OAUTH_INTROSPECTION_CLIENT_REQUESTS_PER_MINUTE","20000").env("OAUTH_INTROSPECTION_ATTEMPTS_PER_MINUTE","30000")
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true).spawn()?;
        let stderr=platform.stderr.take().context("platform diagnostic pipe")?;
        let diagnostics=tokio::spawn(async move{let mut bytes=Vec::new();let _=stderr.take(65536).read_to_end(&mut bytes).await;String::from_utf8_lossy(&bytes).into_owned()});
        let origin=format!("http://{addr}");
        tokio::time::timeout(Duration::from_secs(60),async{loop{if platform.try_wait()?.is_some(){let text=diagnostics.await?;let code=["DATABASE_MIGRATION_FAILED","DATABASE_INITIALIZATION_FAILED","REGION_INITIALIZATION_FAILED","DATABASE_CONNECTION_FAILED"].into_iter().find(|code|text.contains(code)).unwrap_or("PLATFORM_STARTUP_FAILED");return Err(anyhow!(code));}if c.get(format!("{origin}/api/health")).send().await.is_ok_and(|r|r.status().is_success()){return Ok::<_,anyhow::Error>(());}tokio::time::sleep(Duration::from_millis(25)).await;}}).await??;
        let mut stores=Process::start("STORES",&dbs[1],&origin,&f.client_id,&f.client_secret).await?;
        let mut work=Process::start("WORK_ORDERS",&dbs[2],&origin,&f.work_client_id,&f.work_client_secret).await?;
        let result=async {
            stores.ready(&c,"/stores-api").await?;work.ready(&c,"/work-orders-api").await?;seed(&dbs,&f).await?;
            let admin=login(&c,&origin,"admin","admin123",f.platform_root.org).await?;
            grants(&c,&origin,&admin,&f.actor,f.app,&PAIRS,Some("ORG_SUBTREE")).await?;
            grants(&c,&origin,&admin,&f.actor,f.work_app,&[("ticket","read")],Some("ORG_SUBTREE")).await?;
            let session=login(&c,&origin,&f.actor.username,PASSWORD,f.actor.org).await?;
            let store_id:Uuid=sqlx::query_scalar("SELECT id FROM stores WHERE organization_id=$1").bind(f.actor.org).fetch_one(&dbs[1].pool).await?;
            let report_path=format!("/stores/{store_id}/daily-reports");
            let mut sessions=Vec::new();
            for i in 0..200 {let app=if i%2==0{"stores"}else{"work-orders"};sessions.push(issue(&c,&origin,&session,app).await?);}
            let pids=[platform.id().context("platform pid")?,stores.child.id().context("stores pid")?,work.child.id().context("work pid")?];
            let mut rounds=Vec::new();
            for active in [100usize,150,200] {
                let before=pids.map(resources);let generator_before=resources(std::process::id());let started=Instant::now();let inflight=Arc::new(AtomicUsize::new(0));let peak=Arc::new(AtomicUsize::new(0));let barrier=Arc::new(tokio::sync::Barrier::new(active));let mut jobs=Vec::new();
                for (i,bearer) in sessions.iter().take(active).enumerate(){let c=c.clone();let bearer=bearer.clone();let url=if i%2==0{format!("{}/stores-api{report_path}",stores.origin)}else{format!("{}/work-orders-api/tickets",work.origin)};let inflight=inflight.clone();let peak=peak.clone();let barrier=barrier.clone();
                    jobs.push(tokio::spawn(async move{barrier.wait().await;let mut samples=Vec::new();for _ in 0..5 {let n=inflight.fetch_add(1,Ordering::SeqCst)+1;peak.fetch_max(n,Ordering::SeqCst);let start=Instant::now();let status=match c.get(&url).bearer_auth(&bearer).send().await{Ok(r)=>{let status=r.status().as_u16();if r.bytes().await.is_ok(){status}else{0}},Err(_)=>0};samples.push((i%2,start.elapsed().as_secs_f64()*1000.0,status));inflight.fetch_sub(1,Ordering::SeqCst);}samples}));
                }
                let mut samples=Vec::new();for job in jobs{samples.extend(job.await?);}
                let elapsed=started.elapsed().as_secs_f64();let mut cohorts=Vec::new();
                for app in 0..2 {let mut times=samples.iter().filter(|s|s.0==app).map(|s|s.1).collect::<Vec<_>>();times.sort_by(f64::total_cmp);let mut statuses=BTreeMap::new();for (_,_,status) in samples.iter().filter(|s|s.0==app){*statuses.entry(*status).or_insert(0usize)+=1;}
                    ensure!(statuses.keys().all(|s|*s==200||*s==429||*s>=500||*s==0),"unexpected workload contract status: {statuses:?}");
                    let percentile=|q:f64|times[((times.len() as f64*q).ceil() as usize).saturating_sub(1)];let errors=samples.iter().filter(|s|s.0==app&&(s.2>=500||s.2==0)).count();
                    let mut success=samples.iter().filter(|s|s.0==app&&s.2==200).map(|s|s.1).collect::<Vec<_>>();success.sort_by(f64::total_cmp);
                    let success_percentile=|q:f64|if success.is_empty(){None}else{Some(success[((success.len() as f64*q).ceil() as usize).saturating_sub(1)])};
                    cohorts.push(json!({"app":if app==0{"stores"}else{"work-orders"},"samples":times.len(),"statusCounts":statuses,"p50Ms":percentile(0.5),"p95Ms":percentile(0.95),"p99Ms":percentile(0.99),"successfulSamples":success.len(),"successP95Ms":success_percentile(0.95),"successP99Ms":success_percentile(0.99),"unexpectedErrorRatio":errors as f64/times.len() as f64,"candidatePass":success.len()==times.len()&&percentile(0.95)<=500.0&&percentile(0.99)<=1000.0&&errors as f64/(times.len() as f64)<0.001}));
                }
                rounds.push(json!({"onlineAppSessions":200,"activeSessions":active,"distinctPrincipals":1,"maxConcurrentRequests":peak.load(Ordering::SeqCst),"elapsedSeconds":elapsed,"requestsPerSecond":samples.len() as f64/elapsed,"resourcesBefore":before,"resourcesAfter":pids.map(resources),"generatorBefore":generator_before,"generatorAfter":resources(std::process::id()),"cohorts":cohorts}));
            }
            let mut metrics=Vec::new();
            for (db,process,prefix,table,bearer,path) in [(&dbs[1],&stores,"/stores-api","store_daily_reports",&sessions[0],report_path.as_str()),(&dbs[2],&work,"/work-orders-api","work_orders",&sessions[1],"/tickets")] {
                let mut lock=db.pool.begin().await?;sqlx::query(&format!("LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE")).execute(&mut *lock).await?;
                let cc=c.clone();let url=format!("{}{prefix}{path}",process.origin);let bearer=bearer.clone();let pending=tokio::spawn(async move{cc.get(url).bearer_auth(bearer).send().await});
                tokio::time::timeout(Duration::from_secs(3),async{loop{let n=sqlx::query_scalar::<_,i64>("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock'").fetch_one(&db.pool).await?;if n>0{return Ok::<_,anyhow::Error>(());}tokio::task::yield_now().await;}}).await??;
                let body=scrape(&c,&process.origin,prefix).await?;ensure!(body.lines().any(|s|s.starts_with("saas_db_lock_waiters{")&&!s.ends_with(" 0")),"lock metric missing");metrics.push(body);lock.rollback().await?;ensure!(pending.await??.status()==200);
            }
            for sample in 0..31 {
                let reply=request(&c,&origin,"/api/apps/stores/runtime-events",&sessions[0],Method::POST,Some(json!({"kind":"TIMING","stage":"entry","duration_ms":1}))).await?;
                ensure!(reply.status==if sample<30{204}else{429},"bounded timing quota changed");
            }
            let failure=request(&c,&origin,"/api/apps/stores/runtime-events",&sessions[0],Method::POST,Some(json!({"kind":"LOAD_FAILED","stage":"mount"}))).await?;
            ensure!(failure.status==204,"timing samples must not starve failure diagnostics");
            ensure!(request(&c,&origin,"/api/apps/stores/runtime-events",&sessions[0],Method::POST,Some(json!({"kind":"TIMING","stage":"entry","duration_ms":60001}))).await?.status==400);
            sqlx::query("INSERT INTO notifications(user_id,title,content) VALUES($1,'Owned observation','bounded')").bind(f.actor.user).execute(&dbs[0].pool).await?;
            let platform_metrics=scrape(&c,&origin,"/api").await?;
            ensure!(platform_metrics.contains("platform_notification_observation_up 1\n")&&platform_metrics.contains("platform_notification_committed_rows 1\n"),"committed inbox observation must count the actual durable row");
            metrics.push(platform_metrics);
            let faults=monitor_faults(&c,&dbs[1],&stores.origin).await?;
            grants(&c,&origin,&admin,&f.actor,f.work_app,&[("ticket","read")],None).await?;
            ensure!(c.get(format!("{}/work-orders-api/tickets",work.origin)).bearer_auth(&sessions[1]).send().await?.status()==403,"revocation must affect next request");
            let report=json!({"schemaVersion":1,"kind":"phase32-capacity","records":{"workOrders":50000,"dailyReports":50000,"parentStores":4,"workOrderSoftDeletedRatio":0.05,"scopeGroups":4},"resourceOrder":["platform","stores","work-orders"],"onlineDefinition":"200 distinct valid application tokens held by one generator; one underlying platform session and principal; no websocket-presence or browser-page count claim","databaseMaxConnectionsPerService":10,"logicalAppSessions":200,"distinctPrincipals":1,"sessionModel":"200 unique application tokens for one principal; alternating apps; one request per active session, five iterations; closed loop burst, no think time","authorizationModes":{"stores":"legacy","workOrders":"combined"},"network":"private container loopback HTTP including current platform authorization, business SQL and response body; PostgreSQL on owned Docker network; no WAN simulation","hardware":{"cpuInfo":std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default().lines().filter(|s|s.starts_with("model name")).take(1).collect::<Vec<_>>(),"memInfo":std::fs::read_to_string("/proc/meminfo").unwrap_or_default().lines().take(3).collect::<Vec<_>>(),"cpuMax":std::fs::read_to_string("/sys/fs/cgroup/cpu.max").unwrap_or_default(),"memoryMax":std::fs::read_to_string("/sys/fs/cgroup/memory.max").unwrap_or_default()},"rounds":rounds,"metrics":metrics,"revocationNextRequest":403});
            let mut report=report;report["faults"]=faults;
            let file=std::env::var("PHASE32_REPORT").context("explicit report destination required")?;std::fs::write(file,serde_json::to_vec_pretty(&report)?)?;
            println!("PHASE32_CAPACITY PASS real_100000_rows three_load_stages two_db_lock_faults fresh_revocation");Ok::<_,anyhow::Error>(())
        }.await;
        let secrets=vec![SECRET.into(),PASSWORD.into(),METRICS.into(),f.client_secret.clone(),f.work_client_secret.clone()];let a=stores.stop(&secrets).await;let b=work.stop(&secrets).await;let _=platform.kill().await;let _=platform.wait().await;
        result?;ensure!(a?&&b?,"process cleanup failed");Ok::<_,anyhow::Error>(())
    }.await;
    let mut clean = true;
    for d in dbs.into_iter().rev() {
        clean &= d.cleanup(outcome.is_ok()).await.is_ok();
    }
    outcome?;
    ensure!(clean, "database cleanup failed");
    Ok(())
}
