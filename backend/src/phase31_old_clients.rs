//! Frozen consumers exercise current platform and independent business services.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires owned PostgreSQL and built independent business binaries"]
async fn phase31_old_business_clients() -> Result<()> {
    let mut databases = Vec::new();
    for kind in ["PLATFORM", "STORES", "WORK_ORDERS"] {
        match Database::create(kind).await {
            Ok(db) => databases.push(db),
            Err(error) => {
                for db in databases.into_iter().rev() {
                    let _ = db.cleanup(false).await;
                }
                return Err(error);
            }
        }
    }
    let result = async {
        sqlx::migrate!("./migrations").run(&databases[0].pool).await?;
        let f=Fixture::prepare(&databases[0].pool).await?;
        let state=AppState::new(databases[0].pool.clone(),Arc::new(config(&f.policy)?));
        let mut server=Server::start(crate::routes::api_router().nest("/oauth",crate::routes::oauth2::oauth_router()).with_state(state)).await?;
        let secrets=vec![SECRET.to_owned(),PASSWORD.to_owned(),f.client_secret.clone(),f.work_client_secret.clone()];
        let mut stores=match Process::start("STORES",&databases[1],&server.origin,&f.client_id,&f.client_secret).await {Ok(p)=>p,Err(e)=>{server.stop().await;return Err(e);}};
        let mut work=match Process::start("WORK_ORDERS",&databases[2],&server.origin,&f.work_client_id,&f.work_client_secret).await {Ok(p)=>p,Err(e)=>{stores.stop(&secrets).await?;server.stop().await;return Err(e);}};
        let assertions=async {
            let client=Client::builder().no_proxy().timeout(Duration::from_secs(15)).build()?;
            stores.ready(&client,"/stores-api").await?;work.ready(&client,"/work-orders-api").await?;
            let admin=login(&client,&server.origin,"admin","admin123",f.platform_root.org).await?;
            grants(&client,&server.origin,&admin,&f.actor,f.app,&PAIRS,Some("ORG")).await?;
            grants(&client,&server.origin,&admin,&f.actor,f.work_app,&[("ticket","read"),("ticket","create"),("ticket","update")],Some("ORG")).await?;
            let mut setup=json!({});
            for (code,origin) in [("stores",&stores.origin),("work-orders",&work.origin)] {
                let bearer=token(&client,&server.origin,&f.actor,code).await?;
                let reply=request(&client,&server.origin,&format!("/api/apps/{code}/context"),&bearer,Method::GET,None).await?;
                setup[code]=json!({"origin":origin,"token":bearer,"context":reply.require(200)?});
            }
            let output=tokio::time::timeout(Duration::from_secs(40),tokio::process::Command::new("node").arg("../tests/phase31/old-business.mjs").current_dir(env!("CARGO_MANIFEST_DIR")).env("PHASE31_CLIENTS",setup.to_string()).kill_on_drop(true).output()).await.map_err(|_|anyhow!("old business client deadline"))??;
            let stages=String::from_utf8_lossy(&output.stdout).lines().filter(|line|matches!(*line,"PHASE31_CLIENT_STAGE STORES_CONTEXT"|"PHASE31_CLIENT_STAGE STORES_WRITE"|"PHASE31_CLIENT_STAGE WORK_CONTEXT"|"PHASE31_CLIENT_STAGE WORK_WRITE")).collect::<Vec<_>>().join(",");
            ensure!(output.status.success(),"old business adapter failed after {stages} (remote diagnostics suppressed)");
            ensure!(String::from_utf8_lossy(&output.stdout).contains("PHASE31_OLD_BUSINESS PASS"));
            println!("PHASE31_OLD_BUSINESS PASS frozen_Phase30_consumers real_three_databases real_independent_services");
            Ok::<_,anyhow::Error>(())
        }.await;
        let a=stores.stop(&secrets).await;let b=work.stop(&secrets).await;let c=server.stop().await;
        assertions?;ensure!(a? && b? && c,"old consumer process cleanup failed");
        Ok(())
    }.await;
    let mut cleanup = true;
    for db in databases.into_iter().rev() {
        cleanup &= db.cleanup(result.is_ok()).await.is_ok();
    }
    result?;
    ensure!(cleanup, "old consumer database cleanup failed");
    Ok(())
}
