//! Actual ACK durability regression for the former dropped transaction. Only
//! the explicit task-owned cluster is used; no ambient DATABASE_URL is read.
use super::*;
use anyhow::{anyhow, ensure, Result};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Owned {
    pool: PgPool,
    admin: PgPool,
    name: String,
}
impl Owned {
    async fn new() -> Result<Self> {
        let raw = std::env::var("IDENTITY_PHASE12_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE12_TEST_DATABASE_URL required"))?;
        let options = PgConnectOptions::from_str(&raw)
            .map_err(|_| anyhow!("invalid phase12 test database URL"))?;
        ensure!(
            options.get_database() == Some("postgres")
                && matches!(options.get_host(), "localhost" | "127.0.0.1" | "::1"),
            "task-owned loopback administration database required"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("owned test cluster unavailable"))?;
        let cluster: String =
            sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
                .fetch_one(&admin)
                .await?;
        ensure!(cluster == "7700230159816609830", "wrong task-owned cluster");
        let name = format!("identity_phase12_admin_{}", uuid7::new_uuid().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = PgPoolOptions::new()
            .max_connections(3)
            .connect_with(options.database(&name))
            .await
            .map_err(|_| anyhow!("owned test database unavailable"))?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool, admin, name })
    }
    async fn finish(self, result: Result<()>) -> Result<()> {
        self.pool.close().await;
        ensure!(
            self.name.starts_with("identity_phase12_admin_")
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "unsafe cleanup database"
        );
        sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await?;
        self.admin.close().await;
        result
    }
}
#[tokio::test]
#[ignore = "requires explicit task-owned Phase12 PostgreSQL"]
async fn phase12_remote_admin_pg_ack_commits_and_suppressed_audit_is_an_error() -> Result<()> {
    let db = Owned::new().await?;
    let result=async {
        let config=crate::config::Config::from_values(&HashMap::from([
            ("APP_ENV".into(),"test".into()),("DATABASE_URL".into(),"postgres://unused/owned-phase12".into()),
            ("JWT_SECRET".into(),"owned-phase12-tests-only-not-production".into())]))?;
        let state=AppState::new(db.pool.clone(),Arc::new(config));
        let user=CurrentUser {user_id:uuid7::new_uuid(),username:"owned-ack-admin".into(),real_name:"Owned fixture".into(),is_super_admin:false,tenant_id:uuid7::new_uuid(),org_id:uuid7::new_uuid(),session_id:uuid7::new_uuid()};
        let headers=HeaderMap::new();let boundary=PlatformBoundary {state:&state,user:&user,headers:&headers};let operation=uuid7::new_uuid();
        boundary.acknowledge(operation).await?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='OIDC_REMOTE_ACK' AND detail::jsonb->>'operationId'=$1")
            .bind(operation.to_string()).fetch_one(&db.pool).await?;
        ensure!(count==1,"ACK audit transaction was not durably committed");
        sqlx::query("CREATE FUNCTION public.phase12_suppress_ack() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='OIDC_REMOTE_ACK' THEN RETURN NULL; END IF; RETURN NEW; END $$").execute(&db.pool).await?;
        sqlx::query("CREATE TRIGGER phase12_suppress_ack BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION public.phase12_suppress_ack()").execute(&db.pool).await?;
        let missing=uuid7::new_uuid();ensure!(boundary.acknowledge(missing).await.is_err(),"silent audit suppression was accepted as ACK");
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='OIDC_REMOTE_ACK'").fetch_one(&db.pool).await?;
        ensure!(count==1,"failed ACK changed durable audit records");drop(state);Ok(())
    }.await;
    db.finish(result).await
}

async fn authorized_fixture(pool: &PgPool) -> Result<(AppState, CurrentUser)> {
    let user = CurrentUser {
        user_id: uuid7::new_uuid(),
        username: "owned-root-admin".into(),
        real_name: "Owned root".into(),
        is_super_admin: true,
        tenant_id: uuid7::new_uuid(),
        org_id: uuid7::new_uuid(),
        session_id: uuid7::new_uuid(),
    };
    let membership = uuid7::new_uuid();
    let application = uuid7::new_uuid();
    let menu = uuid7::new_uuid();
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin,real_name) VALUES($1,$2,'fixture-not-a-login-credential',true,'Owned root')").bind(user.user_id).bind(&user.username).execute(pool).await?;
    sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned admin tenant','owned-phase12-root',true)").bind(user.tenant_id).execute(pool).await?;
    sqlx::query("INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned admin root')")
        .bind(user.org_id)
        .bind(user.tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,'EMPLOYEE','ENABLED',1)").bind(membership).bind(user.user_id).bind(user.org_id).execute(pool).await?;
    sqlx::query("INSERT INTO applications(id,name,code,allowed_member_types) VALUES($1,'Owned OIDC administration','owned-phase12-oidc-admin',ARRAY['EMPLOYEE'])").bind(application).execute(pool).await?;
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
        .bind(user.tenant_id)
        .bind(application)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
    )
    .bind(user.org_id)
    .bind(application)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,path) VALUES($1,$2,'MENU','Owned OIDC management','system:oauth','/system/oauth/clients')").bind(menu).bind(application).execute(pool).await?;
    sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
        .bind(user.tenant_id)
        .bind(menu)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
        .bind(user.org_id)
        .bind(menu)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) VALUES($1,$2,$3,$4,clock_timestamp()+interval '1 hour',0,$5,'EMPLOYEE',1,'urn:saas:legacy-platform','platform-console-legacy',1)")
        .bind(user.session_id).bind(user.user_id).bind(user.tenant_id).bind(user.org_id).bind(membership).execute(pool).await?;
    let config = crate::config::Config::from_values(&HashMap::from([
        ("APP_ENV".into(), "test".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/owned-phase12".into(),
        ),
        (
            "JWT_SECRET".into(),
            "owned-phase12-tests-only-not-production".into(),
        ),
    ]))?;
    let mut state = AppState::new(pool.clone(), Arc::new(config));
    // This regression calls the actual production acceptance transaction and
    // injects only the outbound transport. No remote startup/HTTP is needed.
    let config = Arc::make_mut(&mut state.config);
    config.identity_remote_enabled = true;
    config.identity_admin_service_client_id = Some(uuid7::new_uuid().to_string());
    Ok((state, user))
}
struct VerifyCommitted {
    pool: PgPool,
    calls: AtomicUsize,
}
impl Transport for VerifyCommitted {
    fn send<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, Reply> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let committed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_oidc_command_intents WHERE id=$1) AND EXISTS(SELECT 1 FROM audit_logs WHERE action='OIDC_REMOTE_ACCEPT' AND detail::jsonb->>'operationId'=$2)")
            .bind(command.operation).bind(command.operation.to_string()).fetch_one(&self.pool).await.map_err(|_|AppError::Unavailable)?;
            if !committed {
                return Err(AppError::Unavailable);
            }
            Ok(Reply {
                status: 200,
                content_type: Some("application/json".into()),
                bytes: serde_json::to_vec(&json!({"kid":"owned-a","status":"PUBLISHED"}))
                    .map_err(|_| AppError::Unavailable)?,
            })
        })
    }
}
fn publish_command() -> Result<Command> {
    let target = Target::new(Method::POST, "/keys/publish")?;
    let body = Some(json!({"kid":"owned-a"}));
    Ok(Command {
        digest: digest(&target, &body)?,
        target,
        body,
        operation: uuid7::new_uuid(),
    })
}
async fn operation_counts(pool: &PgPool, operation: Uuid) -> Result<(i64, i64, i64)> {
    Ok(sqlx::query_as("SELECT (SELECT count(*) FROM platform_oidc_command_intents WHERE id=$1),(SELECT count(*) FROM audit_logs WHERE action='OIDC_REMOTE_ACCEPT' AND detail::jsonb->>'operationId'=$2),(SELECT count(*) FROM audit_logs WHERE action='OIDC_REMOTE_ACK' AND detail::jsonb->>'operationId'=$2)")
        .bind(operation).bind(operation.to_string()).fetch_one(pool).await?)
}
#[tokio::test]
#[ignore = "requires explicit task-owned Phase12 PostgreSQL"]
async fn phase12_remote_admin_pg_suppressed_intent_or_accept_audit_rolls_back_without_dispatch(
) -> Result<()> {
    let db = Owned::new().await?;
    let result=async {
        let (state,user)=authorized_fixture(&db.pool).await?;let headers=HeaderMap::new();let boundary=PlatformBoundary {state:&state,user:&user,headers:&headers};
        let transport=VerifyCommitted {pool:db.pool.clone(),calls:AtomicUsize::new(0)};
        let deadline=||Deadline {at:tokio::time::Instant::now()+Duration::from_secs(3),dispatched:Arc::new(std::sync::Mutex::new(None))};
        let successful=publish_command()?;
        ensure!(execute(&boundary,&transport,&successful,deadline()).await.is_ok(),"authorized control did not commit intent/audit before network and ACK after it");
        ensure!(operation_counts(&db.pool,successful.operation).await?==(1,1,1),"control operation is missing durable intent or audits");
        ensure!(transport.calls.load(Ordering::SeqCst)==1,"control did not dispatch exactly once");
        sqlx::query("CREATE FUNCTION public.phase12_suppress_intent() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$").execute(&db.pool).await?;
        sqlx::query("CREATE TRIGGER phase12_suppress_intent BEFORE INSERT ON platform_oidc_command_intents FOR EACH ROW EXECUTE FUNCTION public.phase12_suppress_intent()").execute(&db.pool).await?;
        let absent=publish_command()?;ensure!(execute(&boundary,&transport,&absent,deadline()).await.is_err(),"suppressed intent was accepted");
        ensure!(transport.calls.load(Ordering::SeqCst)==1,"intent suppression dispatched a remote command");
        ensure!(operation_counts(&db.pool,absent.operation).await?==(0,0,0),"intent suppression committed partial accept state");
        sqlx::query("DROP TRIGGER phase12_suppress_intent ON platform_oidc_command_intents").execute(&db.pool).await?;
        sqlx::query("CREATE FUNCTION public.phase12_suppress_accept() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='OIDC_REMOTE_ACCEPT' THEN RETURN NULL; END IF; RETURN NEW; END $$").execute(&db.pool).await?;
        sqlx::query("CREATE TRIGGER phase12_suppress_accept BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION public.phase12_suppress_accept()").execute(&db.pool).await?;
        let unaudited=publish_command()?;ensure!(execute(&boundary,&transport,&unaudited,deadline()).await.is_err(),"suppressed acceptance audit was accepted");
        ensure!(transport.calls.load(Ordering::SeqCst)==1,"acceptance-audit suppression dispatched a remote command");
        ensure!(operation_counts(&db.pool,unaudited.operation).await?==(0,0,0),"acceptance-audit suppression did not roll back its intent");
        drop(state);Ok(())
    }.await;
    db.finish(result).await
}
