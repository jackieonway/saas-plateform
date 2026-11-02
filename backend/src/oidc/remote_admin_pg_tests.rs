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
        let expected_cluster = std::env::var("IDENTITY_PHASE12_TEST_CLUSTER_ID")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE12_TEST_CLUSTER_ID required"))?;
        ensure!(cluster == expected_cluster, "wrong task-owned cluster");
        let name = format!("identity_phase12_admin_{}", uuid7::new_uuid().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = match PgPoolOptions::new()
            .max_connections(3)
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await?;
                admin.close().await;
                return Err(anyhow!("owned test database unavailable"));
            }
        };
        if let Err(error) = sqlx::migrate!("./migrations").run(&pool).await {
            pool.close().await;
            sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                .execute(&admin)
                .await?;
            admin.close().await;
            return Err(error.into());
        }
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
        let headers=HeaderMap::new();let boundary=PlatformBoundary {state:&state,user:&user,headers:&headers,metadata:None};let operation=uuid7::new_uuid();
        boundary.acknowledge(operation, false, false).await?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='IDENTITY_KEY_COMMAND_ACK' AND detail::jsonb->>'operationId'=$1")
            .bind(operation.to_string()).fetch_one(&db.pool).await?;
        ensure!(count==1,"ACK audit transaction was not durably committed");
        sqlx::query("CREATE FUNCTION public.phase12_suppress_ack() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='IDENTITY_KEY_COMMAND_ACK' THEN RETURN NULL; END IF; RETURN NEW; END $$").execute(&db.pool).await?;
        sqlx::query("CREATE TRIGGER phase12_suppress_ack BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION public.phase12_suppress_ack()").execute(&db.pool).await?;
        let missing=uuid7::new_uuid();ensure!(boundary.acknowledge(missing, false, false).await.is_err(),"silent audit suppression was accepted as ACK");
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='IDENTITY_KEY_COMMAND_ACK'").fetch_one(&db.pool).await?;
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
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin,real_name,session_version) VALUES($1,$2,'fixture-not-a-login-credential',true,'Owned root',3)").bind(user.user_id).bind(&user.username).execute(pool).await?;
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
    for permission in [
        "system:oauth:view",
        "system:oauth:register",
        "system:oauth:review",
        "system:oauth:publish",
        "system:oauth:rotate",
        "system:oauth:disable",
        "system:oauth:scopes",
        "system:oauth:invite",
        "system:oauth:keys",
        "system:oauth:assign",
    ] {
        let button = uuid7::new_uuid();
        sqlx::query("INSERT INTO menus(id,application_id,parent_id,type,title,permission) VALUES($1,$2,$3,'BUTTON',$4,$4)")
            .bind(button).bind(application).bind(menu).bind(permission).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(user.tenant_id)
            .bind(button)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
            .bind(user.org_id)
            .bind(button)
            .execute(pool)
            .await?;
    }
    let issuer = "https://auth.saas.test";
    sqlx::query("INSERT INTO auth_provider_settings(singleton,issuer,created_at) VALUES(true,$1,clock_timestamp())")
        .bind(issuer).execute(pool).await?;
    let (client, grant, sid, auth_time, expires) =
        crate::identity_deployment_tests::canonical_employee_source(pool, user.user_id).await?;
    sqlx::query(
        "UPDATE sso_sessions SET mfa_method='totp',mfa_verified_at=clock_timestamp() WHERE id=$1",
    )
    .bind(sid)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO auth_totp_factors(user_id,secret_ciphertext) VALUES($1,decode(repeat('ab',32),'hex'))")
        .bind(user.user_id).execute(pool).await?;
    let browser = uuid7::new_uuid();
    let bff = uuid7::new_uuid();
    let digest = crate::util::hash::sha256_hex(&bff.to_string());
    sqlx::query("INSERT INTO console_browsers(id,token_hash,generation,current_session_id,created_at,expires_at) VALUES($1,$2,1,$3,clock_timestamp(),$4)")
        .bind(browser).bind(&digest).bind(bff).bind(expires).execute(pool).await?;
    sqlx::query("INSERT INTO bff_sessions(id,browser_id,token_hash,csrf_hash,user_id,account_version,global_session_id,oidc_client_id,oidc_grant_id,oidc_client_policy_version,issuer,auth_time,nonce_hash,access_ciphertext,refresh_ciphertext,access_expires_at,version,selected_session_id,created_at,expires_at,oidc_public_sid,oidc_public_sub) VALUES($1,$2,$3,$3,$4,3,$5,$6,$7,1,$8,$9,$3,'owned-admin-domain-fixture','owned-admin-domain-fixture',$10,1,$11,clock_timestamp(),$10,$3,'owned-pairwise-'||$4::text)")
        .bind(bff).bind(browser).bind(&digest).bind(user.user_id).bind(sid).bind(client).bind(grant).bind(issuer).bind(auth_time).bind(expires).bind(user.session_id).execute(pool).await?;
    sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version,bff_session_id,bff_context_version) VALUES($1,$2,$3,$4,$5,3,$6,'EMPLOYEE',1,$7,$8,1,$9,1)")
        .bind(user.session_id).bind(user.user_id).bind(user.tenant_id).bind(user.org_id).bind(expires).bind(membership).bind(issuer).bind(client.to_string()).bind(bff).execute(pool).await?;
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
    config.identity_key_admin_service_client_id = Some(uuid7::new_uuid().to_string());
    Ok((state, user))
}
struct VerifyCommitted {
    pool: PgPool,
    calls: AtomicUsize,
}

// The private metadata HTTP port is a fixture here. Production always uses
// authenticated HTTPS; human authorization and application scope remain the
// actual canonical database functions in these tests.
struct CanonicalMetadata {
    pool: PgPool,
}
impl Metadata for CanonicalMetadata {
    fn client<'a>(&'a self, client: Uuid) -> AdminFuture<'a, Value> {
        Box::pin(async move {
            let row:Value=sqlx::query_scalar("SELECT to_jsonb(c)||jsonb_build_object('applicationName',a.display_name,'ownership',a.ownership,'partnerId',a.partner_id) FROM auth_clients c JOIN auth_applications a ON a.application_id=c.application_id WHERE c.id=$1")
                .bind(client).fetch_optional(&self.pool).await.map_err(AppError::from)?.ok_or_else(||AppError::not_found("fixture client missing"))?;
            let mut object = serde_json::Map::new();
            for (output, input) in [
                ("id", "id"),
                ("clientId", "client_id"),
                ("displayName", "display_name"),
                ("applicationId", "application_id"),
                ("applicationName", "applicationName"),
                ("ownership", "ownership"),
                ("partnerId", "partnerId"),
                ("environment", "environment"),
                ("clientType", "client_type"),
                ("runtimeKind", "runtime_kind"),
                ("subjectMode", "subject_mode"),
                ("contextMode", "context_mode"),
                ("allowedMemberTypes", "allowed_member_types"),
                ("grantTypes", "grant_types"),
                ("tokenEndpointAuthMethod", "token_endpoint_auth_method"),
                ("redirectUris", "redirect_uris"),
                ("postLogoutRedirectUris", "post_logout_redirect_uris"),
                ("logoutMode", "logout_mode"),
                ("backchannelLogoutUri", "backchannel_logout_uri"),
                ("backchannelTargetId", "backchannel_target_id"),
                ("status", "status"),
                ("version", "version"),
                ("policyVersion", "policy_version"),
                ("createdAt", "created_at"),
                ("updatedAt", "updated_at"),
            ] {
                object.insert(output.into(), row[input].clone());
            }
            let group: Option<Uuid> = sqlx::query_scalar(
                "SELECT group_id FROM auth_consent_group_clients WHERE client_id=$1",
            )
            .bind(client)
            .fetch_optional(&self.pool)
            .await?;
            object.insert("consentGroupId".into(), json!(group));
            let permissions:Value=sqlx::query_scalar("SELECT COALESCE(jsonb_agg(jsonb_build_object('resourceKey',resource_key,'audience',audience,'scopes',scopes) ORDER BY resource_key),'[]'::jsonb) FROM (SELECT r.resource_key,r.audience,array_agg(s.scope_name ORDER BY s.scope_name) scopes FROM auth_client_permissions p JOIN auth_resources r ON r.id=p.resource_id JOIN auth_scopes s ON s.id=p.scope_id AND s.resource_id=p.resource_id WHERE p.client_id=$1 GROUP BY r.resource_key,r.audience) registered")
                .bind(client).fetch_one(&self.pool).await?;
            object.insert("permissions".into(), permissions);
            Ok(Value::Object(object))
        })
    }
}

struct RegistryPageTransport {
    pool: PgPool,
    clients: Vec<Uuid>,
    malformed_public_id: Option<Value>,
    calls: AtomicUsize,
}
impl Transport for RegistryPageTransport {
    fn send<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, Reply> {
        Box::pin(async move {
            if command.target.method != Method::GET
                || command.target.path != format!("{AUTH_CLIENTS_PREFIX}?limit=10&offset=0")
            {
                return Err(AppError::Unavailable);
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            let metadata = CanonicalMetadata {
                pool: self.pool.clone(),
            };
            let mut records = Vec::new();
            for client in &self.clients {
                records.push(metadata.client(*client).await?);
            }
            if let Some(public_id) = &self.malformed_public_id {
                records[1]["clientId"] = public_id.clone();
            }
            Ok(Reply {
                status: 200,
                content_type: Some("application/json".into()),
                bytes: serde_json::to_vec(&json!({"records":records,"page":{"limit":10,"offset":0,"total":self.clients.len()}}))
                    .map_err(|_| AppError::Unavailable)?,
            })
        })
    }
}

#[tokio::test]
#[ignore = "requires explicit task-owned Phase12 PostgreSQL"]
async fn phase25_registry_reads_default_and_opaque_user_machine_ids_without_weakening_validation(
) -> Result<()> {
    let db = Owned::new().await?;
    let result = async {
        let (state, user) = authorized_fixture(&db.pool).await?;
        let (application, default_client): (Uuid, Uuid) = sqlx::query_as(
            "SELECT application_id,id FROM auth_clients ORDER BY created_at,id LIMIT 1")
            .fetch_one(&db.pool).await?;
        let rp = uuid7::new_uuid();
        let machine = uuid7::new_uuid();
        let rp_public = "p21-standard-rp-owned";
        let machine_public = "p21-machine-owned";
        sqlx::query("INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,allowed_member_types,grant_types,token_endpoint_auth_method,redirect_uris,status) VALUES($1,$2,'Owned standard RP',$3,'test','CONFIDENTIAL','WEB_BFF','USER','ACCOUNT_ONLY',ARRAY[]::text[],ARRAY['authorization_code','refresh_token'],'client_secret_basic',ARRAY['https://client.example.test/callback'],'ENABLED')")
            .bind(rp).bind(rp_public).bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,allowed_member_types,grant_types,token_endpoint_auth_method,redirect_uris,status) VALUES($1,$2,'Owned machine',$3,'test','CONFIDENTIAL','SERVICE','MACHINE','NONE',ARRAY[]::text[],ARRAY['client_credentials'],'client_secret_basic',ARRAY[]::text[],'ENABLED')")
            .bind(machine).bind(machine_public).bind(application).execute(&db.pool).await?;
        let headers = HeaderMap::new();
        let boundary = PlatformBoundary { state: &state, user: &user, headers: &headers, metadata: None };
        let transport = RegistryPageTransport { pool: db.pool.clone(), clients: vec![default_client, rp, machine],
            malformed_public_id: None, calls: AtomicUsize::new(0) };
        let query = validate_client_list_query(ClientListQuery { application_id: None, partner_id: None,
            ownership: None, environment: None, client_type: None, subject_mode: None, status: None,
            limit: Some(10), offset: Some(0) })?;
        let command = Command { target: Target::auth_client_list(&query)?, body: None,
            operation: uuid7::new_uuid(), digest: String::new() };
        let page = execute(&boundary, &transport, &command, fresh_deadline()).await
            .map_err(|_| anyhow!("current canonical RP/MACHINE registry page was rejected"))?;
        ensure!(page["records"].as_array().is_some_and(|records| records.len() == 3)
            && page["records"][0]["id"] == json!(default_client)
            && page["records"][0]["clientId"] == json!(default_client)
            && page["records"][1]["id"] == json!(rp) && page["records"][1]["clientId"] == rp_public
            && page["records"][2]["id"] == json!(machine) && page["records"][2]["clientId"] == machine_public
            && page["records"][2]["subjectMode"] == "MACHINE", "public and internal identifiers were normalized or mixed");
        ensure!(transport.calls.load(Ordering::SeqCst) == 1, "valid registry page was not dispatched exactly once");
        for public_id in ["界".repeat(100), " RP with spaces ".into()] {
            sqlx::query("UPDATE auth_clients SET client_id=$1 WHERE id=$2")
                .bind(&public_id).bind(rp).execute(&db.pool).await?;
            let page = execute(&boundary, &transport, &command, fresh_deadline()).await
                .map_err(|_| anyhow!("valid registered public identifier was restricted beyond identity protocol"))?;
            ensure!(page["records"][1]["clientId"] == public_id, "opaque public identifier was modified");
        }
        for public_id in [Value::Null, json!(10), json!(""), json!("x".repeat(101)),
            json!("界".repeat(101)), json!("bad\nclient"), json!("bad\u{0085}client")] {
            let malformed = RegistryPageTransport { pool: db.pool.clone(), clients: transport.clients.clone(),
                malformed_public_id: Some(public_id), calls: AtomicUsize::new(0) };
            ensure!(matches!(execute(&boundary, &malformed, &command, fresh_deadline()).await,
                Err(Failure { error: AppError::Unavailable, operation: None })), "one malformed remote row was silently accepted or dropped");
            ensure!(malformed.calls.load(Ordering::SeqCst) == 1, "malformed metadata did not reach the production decoder");
        }
        sqlx::query("UPDATE menus SET status='DISABLED' WHERE permission='system:oauth:view'")
            .execute(&db.pool).await?;
        let before = transport.calls.load(Ordering::SeqCst);
        ensure!(matches!(execute(&boundary, &transport, &command, fresh_deadline()).await,
            Err(Failure { error: AppError::Forbidden(_), .. })), "metadata compatibility bypassed VIEW authorization");
        ensure!(transport.calls.load(Ordering::SeqCst) == before, "denied caller dispatched the private metadata request");
        ensure!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM platform_auth_command_intents")
            .fetch_one(&db.pool).await? == 0, "read-only metadata created a mutation intent");
        drop(state);
        Ok(())
    }.await;
    db.finish(result).await
}

impl Transport for VerifyCommitted {
    fn send<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, Reply> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let committed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_auth_command_intents WHERE id=$1) AND EXISTS(SELECT 1 FROM audit_logs WHERE action='IDENTITY_KEY_COMMAND_ACCEPT' AND detail::jsonb->>'operationId'=$2)")
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
    Ok(sqlx::query_as("SELECT (SELECT count(*) FROM platform_auth_command_intents WHERE id=$1),(SELECT count(*) FROM audit_logs WHERE action='IDENTITY_KEY_COMMAND_ACCEPT' AND detail::jsonb->>'operationId'=$2),(SELECT count(*) FROM audit_logs WHERE action='IDENTITY_KEY_COMMAND_ACK' AND detail::jsonb->>'operationId'=$2)")
        .bind(operation).bind(operation.to_string()).fetch_one(pool).await?)
}
#[tokio::test]
#[ignore = "requires explicit task-owned Phase12 PostgreSQL"]
async fn phase12_remote_admin_pg_suppressed_intent_or_accept_audit_rolls_back_without_dispatch(
) -> Result<()> {
    let db = Owned::new().await?;
    let result=async {
        let (state,user)=authorized_fixture(&db.pool).await?;let headers=HeaderMap::new();let boundary=PlatformBoundary {state:&state,user:&user,headers:&headers,metadata:None};
        let transport=VerifyCommitted {pool:db.pool.clone(),calls:AtomicUsize::new(0)};
        let deadline=||Deadline {at:tokio::time::Instant::now()+Duration::from_secs(3),dispatched:Arc::new(std::sync::Mutex::new(None))};
        let successful=publish_command()?;
        ensure!(execute(&boundary,&transport,&successful,deadline()).await.is_ok(),"authorized control did not commit intent/audit before network and ACK after it");
        ensure!(operation_counts(&db.pool,successful.operation).await?==(1,1,1),"control operation is missing durable intent or audits");
        ensure!(transport.calls.load(Ordering::SeqCst)==1,"control did not dispatch exactly once");
        sqlx::query("CREATE FUNCTION public.phase12_suppress_intent() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$").execute(&db.pool).await?;
        sqlx::query("CREATE TRIGGER phase12_suppress_intent BEFORE INSERT ON platform_auth_command_intents FOR EACH ROW EXECUTE FUNCTION public.phase12_suppress_intent()").execute(&db.pool).await?;
        let absent=publish_command()?;ensure!(execute(&boundary,&transport,&absent,deadline()).await.is_err(),"suppressed intent was accepted");
        ensure!(transport.calls.load(Ordering::SeqCst)==1,"intent suppression dispatched a remote command");
        ensure!(operation_counts(&db.pool,absent.operation).await?==(0,0,0),"intent suppression committed partial accept state");
        sqlx::query("DROP TRIGGER phase12_suppress_intent ON platform_auth_command_intents").execute(&db.pool).await?;
        sqlx::query("CREATE FUNCTION public.phase12_suppress_accept() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='IDENTITY_KEY_COMMAND_ACCEPT' THEN RETURN NULL; END IF; RETURN NEW; END $$").execute(&db.pool).await?;
        sqlx::query("CREATE TRIGGER phase12_suppress_accept BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION public.phase12_suppress_accept()").execute(&db.pool).await?;
        let unaudited=publish_command()?;ensure!(execute(&boundary,&transport,&unaudited,deadline()).await.is_err(),"suppressed acceptance audit was accepted");
        ensure!(transport.calls.load(Ordering::SeqCst)==1,"acceptance-audit suppression dispatched a remote command");
        ensure!(operation_counts(&db.pool,unaudited.operation).await?==(0,0,0),"acceptance-audit suppression did not roll back its intent");
        drop(state);Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit task-owned Phase12 PostgreSQL"]
async fn phase05_browser_operation_intent_is_idempotent_and_absence_cannot_race_late_accept(
) -> Result<()> {
    let db = Owned::new().await?;
    let result = async {
        let (state, user) = authorized_fixture(&db.pool).await?;
        let headers = HeaderMap::new();
        let boundary = PlatformBoundary {
            state: &state,
            user: &user,
            headers: &headers,
            metadata: None,
        };
        let mut command = publish_command()?;
        boundary.accept(&command).await?;
        boundary.accept(&command).await?;
        ensure!(
            operation_counts(&db.pool, command.operation).await? == (1, 1, 0),
            "same browser operation duplicated intent or audit"
        );
        command.digest = "f".repeat(64);
        ensure!(
            boundary.accept(&command).await.is_err(),
            "operation accepted a different payload"
        );
        let now = chrono::Utc::now().timestamp_millis() - 120_000;
        let mut bytes = *uuid7::new_uuid().as_bytes();
        bytes[..6].copy_from_slice(&now.to_be_bytes()[2..]);
        command.operation = Uuid::from_bytes(bytes);
        ensure!(
            boundary.accept(&command).await.is_err(),
            "expired browser operation was newly admitted"
        );
        ensure!(
            operation_counts(&db.pool, command.operation).await? == (0, 0, 0),
            "expired admission wrote partial state"
        );
        let absent = operation(
            State(state.clone()),
            user,
            headers,
            Path(command.operation.to_string()),
            Extension(Deadline {
                at: tokio::time::Instant::now() + Duration::from_secs(3),
                dispatched: Arc::new(Mutex::new(None)),
            }),
        )
        .await;
        let absent = absent.map_err(|_| anyhow!("closed absent operation lookup failed"))?;
        ensure!(
            absent.0["state"] == "NOT_APPLIED",
            "closed admission plus locked absence did not prove non-application"
        );
        drop(state);
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit task-owned Phase12 PostgreSQL"]
async fn phase05_operation_lookup_rejects_another_active_session_before_remote_configuration(
) -> Result<()> {
    let db = Owned::new().await?;
    let result = async {
        let (mut state, user) = authorized_fixture(&db.pool).await?;
        let service = uuid7::new_uuid();
        Arc::make_mut(&mut state.config).identity_auth_admin_service_client_id =
            Some(service.to_string());
        let mut other_session = user.clone();
        other_session.session_id = uuid7::new_uuid();
        let browser = uuid7::new_uuid();
        let bff = uuid7::new_uuid();
        let token_digest = crate::util::hash::sha256_hex(&bff.to_string());
        sqlx::query("INSERT INTO console_browsers(id,token_hash,generation,current_session_id,created_at,expires_at) SELECT $1,$2,1,$3,clock_timestamp(),expires_at FROM platform_sessions WHERE id=$4")
            .bind(browser).bind(&token_digest).bind(bff).bind(user.session_id).execute(&db.pool).await?;
        sqlx::query("INSERT INTO bff_sessions(id,browser_id,token_hash,csrf_hash,user_id,account_version,global_session_id,oidc_client_id,oidc_grant_id,oidc_client_policy_version,issuer,auth_time,nonce_hash,access_ciphertext,refresh_ciphertext,access_expires_at,version,selected_session_id,created_at,expires_at,oidc_public_sid,oidc_public_sub) SELECT $1,$2,$3,$3,b.user_id,b.account_version,b.global_session_id,b.oidc_client_id,b.oidc_grant_id,b.oidc_client_policy_version,b.issuer,b.auth_time,$3,b.access_ciphertext,b.refresh_ciphertext,b.access_expires_at,b.version,$4,clock_timestamp(),b.expires_at,b.oidc_public_sid,b.oidc_public_sub FROM bff_sessions b JOIN platform_sessions s ON s.bff_session_id=b.id WHERE s.id=$5")
            .bind(bff).bind(browser).bind(&token_digest).bind(other_session.session_id).bind(user.session_id).execute(&db.pool).await?;
        sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version,bff_session_id,bff_context_version) SELECT $1,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version,$2,bff_context_version FROM platform_sessions WHERE id=$3")
            .bind(other_session.session_id).bind(bff).bind(user.session_id).execute(&db.pool).await?;
        let active: (bool, bool) = sqlx::query_as(
            "SELECT platform_session_active($1),platform_session_active($2)",
        )
        .bind(user.session_id)
        .bind(other_session.session_id)
        .fetch_one(&db.pool)
        .await?;
        ensure!(active == (true, true), "both employee sessions must be genuinely active");
        let client: Uuid = sqlx::query_scalar("SELECT id FROM auth_clients LIMIT 1")
            .fetch_one(&db.pool).await?;
        let target = Target::auth_client(Method::POST, &format!("/{client}/credentials"))?;
        let body = Some(json!({"expectedVersion":1}));
        let command = Command {
            digest: digest(&target, &body)?,
            target,
            body,
            operation: uuid7::new_uuid(),
        };
        let headers = HeaderMap::new();
        let metadata=CanonicalMetadata{pool:db.pool.clone()};
        PlatformBoundary { state: &state, user: &user, headers: &headers,metadata:Some(&metadata) }
            .accept(&command).await?;
        let dispatched = Arc::new(Mutex::new(None));
        let lookup = operation(
            State(state.clone()),
            other_session,
            headers,
            Path(command.operation.to_string()),
            Extension(Deadline {
                at: tokio::time::Instant::now() + Duration::from_secs(3),
                dispatched: dispatched.clone(),
            }),
        ).await;
        let failure = lookup.err().ok_or_else(|| anyhow!("another session recovered an operation result"))?;
        ensure!(matches!(failure.error, AppError::Forbidden(_)),
            "another active session was not rejected by ownership before missing HTTPS/service-secret configuration");
        ensure!(failure.operation.is_none() && dispatched.lock().map_err(|_| anyhow!("lookup dispatch lock poisoned"))?.is_none(),
            "foreign-session lookup reached the remote result boundary");
        let lookups: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='IDENTITY_AUTH_CLIENT_COMMAND_LOOKUP'")
            .fetch_one(&db.pool).await?;
        ensure!(lookups == 0, "foreign-session lookup was audited as an authorized result recovery");
        drop(state);
        Ok(())
    }.await;
    db.finish(result).await
}

struct LostGroupReply(AtomicUsize);
impl Transport for LostGroupReply {
    fn send<'a>(&'a self, _: &'a Command) -> AdminFuture<'a, Reply> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(AppError::Unavailable)
        })
    }
}

#[tokio::test]
#[ignore = "requires explicit task-owned Phase12 PostgreSQL"]
async fn phase07_canonical_group_command_intents_survive_unknown_reply_without_new_operation(
) -> Result<()> {
    let db = Owned::new().await?;
    let result = async {
        let (mut state, user) = authorized_fixture(&db.pool).await?;
        let service = uuid7::new_uuid();
        Arc::make_mut(&mut state.config).identity_auth_admin_service_client_id = Some(service.to_string());
        sqlx::query("INSERT INTO identity_service_clients(id,secret_hash,purpose,status,created_at) VALUES($1,repeat('b',64),'AUTH_ADMIN','ENABLED',clock_timestamp())")
            .bind(service).execute(&db.pool).await?;
        let headers = HeaderMap::new();
        let metadata=CanonicalMetadata{pool:db.pool.clone()};
        let boundary = PlatformBoundary { state: &state, user: &user, headers: &headers,metadata:Some(&metadata) };
        let transport = LostGroupReply(AtomicUsize::new(0));
        let (application, client): (Uuid, Uuid) = sqlx::query_as("SELECT application_id,id FROM auth_clients LIMIT 1").fetch_one(&db.pool).await?;
        let group = uuid7::new_uuid();
        for (target, body) in [
            (Target::consent_group_create(), json!({"applicationId":application,"environment":"test","displayName":"Owned group","purpose":"Console access","contextMode":"PLATFORM_CONTEXT"})),
            (Target::consent_group_member(group), json!({"clientId":client,"expectedGroupVersion":1})),
        ] {
            let body = prepare(&target, Some(body))?;
            let command = Command { digest: digest(&target, &body)?, target, body, operation: uuid7::new_uuid() };
            let result = execute(&boundary, &transport, &command, Deadline { at: tokio::time::Instant::now() + Duration::from_secs(3), dispatched: Arc::new(Mutex::new(None)) }).await;
            let failure = result.err().ok_or_else(|| anyhow!("lost reply unexpectedly completed"))?;
            ensure!(failure.operation == Some(command.operation), "group unknown reply lost the browser operation");
            // Re-observing the accepted original operation must not make a new
            // intent or audit even if a caller sends the same request again.
            boundary.accept(&command).await?;
            let (intents, accepts): (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM platform_auth_command_intents WHERE id=$1),(SELECT count(*) FROM audit_logs WHERE action='IDENTITY_AUTH_CLIENT_COMMAND_ACCEPT' AND detail::jsonb->>'operationId'=$2)")
                .bind(command.operation).bind(command.operation.to_string()).fetch_one(&db.pool).await?;
            ensure!((intents, accepts) == (1, 1), "group acceptance was missing or duplicated after a lost reply");
            sqlx::query("INSERT INTO identity_auth_command_receipts(operation_id,service_id,request_digest,service_secret_hash,method,path,result_json,returns_secret,created_at) VALUES($1,$2,repeat('a',64),repeat('b',64),'POST',$3,'{}'::jsonb,false,clock_timestamp())")
                .bind(command.operation).bind(service).bind(&command.target.path).execute(&db.pool).await?;
        }
        ensure!(transport.0.load(Ordering::SeqCst) == 2, "group replay dispatched an extra outbound command");
        for path in [
            format!("{AUTH_CONSENT_GROUPS_PREFIX}/"),
            format!("{AUTH_CONSENT_GROUPS_PREFIX}/{}/clients", Uuid::new_v4()),
            format!("{AUTH_CONSENT_GROUPS_PREFIX}/{group}/clients/extra"),
            format!("{AUTH_CONSENT_GROUPS_PREFIX}/{group}/rotate"),
            format!("{AUTH_CONSENT_GROUPS_PREFIX}?unexpected=true"),
        ] {
            let receipt = sqlx::query("INSERT INTO identity_auth_command_receipts(operation_id,service_id,request_digest,service_secret_hash,method,path,result_json,returns_secret,created_at) VALUES($1,$2,repeat('a',64),repeat('b',64),'POST',$3,'{}'::jsonb,false,clock_timestamp())")
                .bind(uuid7::new_uuid()).bind(service).bind(&path).execute(&db.pool).await;
            ensure!(receipt.err().and_then(|error| error.as_database_error().and_then(|database| database.constraint()).map(str::to_owned)).as_deref() == Some("identity_auth_command_receipts_path_check"), "receipt accepted an unregistered group path: {path}");
            let intent = sqlx::query("INSERT INTO platform_auth_command_intents(id,actor_user_id,tenant_id,organization_id,session_id,service_client_id,request_digest,method,path,accepted_at) VALUES($1,$2,$3,$4,$5,$6,repeat('a',64),'POST',$7,clock_timestamp())")
                .bind(uuid7::new_uuid()).bind(user.user_id).bind(user.tenant_id).bind(user.org_id).bind(user.session_id).bind(uuid7::new_uuid()).bind(&path).execute(&db.pool).await;
            ensure!(intent.err().and_then(|error| error.as_database_error().and_then(|database| database.constraint()).map(str::to_owned)).as_deref() == Some("platform_auth_command_intents_path_check"), "intent accepted an unregistered group path: {path}");
        }
        drop(state);
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit task-owned Phase12 PostgreSQL"]
async fn phase19_remote_admin_requires_fine_permission_current_totp_and_production_factor(
) -> Result<()> {
    let db = Owned::new().await?;
    let result = async {
        let (mut state, user) = authorized_fixture(&db.pool).await?;
        let headers = HeaderMap::new();
        let source: Uuid = sqlx::query_scalar("SELECT b.global_session_id FROM bff_sessions b JOIN platform_sessions s ON s.bff_session_id=b.id WHERE s.id=$1")
            .bind(user.session_id).fetch_one(&db.pool).await?;
        let transport = VerifyCommitted { pool: db.pool.clone(), calls: AtomicUsize::new(0) };
        let cases = [
            ("totp", "clock_timestamp()-interval '6 minutes'"),
            ("recovery_code", "clock_timestamp()"),
            ("totp", "clock_timestamp()+interval '1 minute'"),
        ];
        for (method, timestamp) in cases {
            sqlx::query(&format!("UPDATE sso_sessions SET mfa_method=$2,mfa_verified_at={timestamp} WHERE id=$1"))
                .bind(source).bind(method).execute(&db.pool).await?;
            let command = publish_command()?;
            let boundary = PlatformBoundary { state: &state, user: &user, headers: &headers, metadata: None };
            let failure = execute(&boundary, &transport, &command, fresh_deadline()).await
                .err().ok_or_else(|| anyhow!("unusable MFA was allowed to dispatch"))?;
            ensure!(matches!(failure.error, AppError::Forbidden(_)), "unusable MFA did not produce a definite authorization denial");
            ensure!(operation_counts(&db.pool, command.operation).await? == (0,0,0), "MFA denial persisted partial intent/audit");
        }
        ensure!(transport.calls.load(Ordering::SeqCst) == 0, "MFA denial reached private transport");
        sqlx::query("UPDATE sso_sessions SET mfa_method='totp',mfa_verified_at=clock_timestamp() WHERE id=$1")
            .bind(source).execute(&db.pool).await?;
        sqlx::query("UPDATE menus SET status='DISABLED' WHERE permission LIKE 'system:oauth:%'")
            .execute(&db.pool).await?;
        let command = publish_command()?;
        let boundary = PlatformBoundary { state: &state, user: &user, headers: &headers, metadata: None };
        ensure!(matches!(boundary.accept(&command).await, Err(AppError::Forbidden(_))),
            "coarse system:oauth menu retained write authority");
        ensure!(operation_counts(&db.pool, command.operation).await? == (0,0,0), "coarse-menu denial persisted state");
        sqlx::query("UPDATE menus SET status='ENABLED' WHERE permission='system:oauth:view'")
            .execute(&db.pool).await?;
        sqlx::query("DELETE FROM auth_totp_factors WHERE user_id=$1")
            .bind(user.user_id).execute(&db.pool).await?;
        Arc::make_mut(&mut state.config).app_env = "production".into();
        let target = Target::auth_client(Method::GET, "")?;
        let read = Command { target, body: None, operation: uuid7::new_uuid(), digest: String::new() };
        let boundary = PlatformBoundary { state: &state, user: &user, headers: &headers, metadata: None };
        ensure!(matches!(boundary.accept(&read).await, Err(AppError::Forbidden(_))),
            "production read admitted an administrator without an enabled factor");
        Arc::make_mut(&mut state.config).app_env = "test".into();
        let boundary = PlatformBoundary { state: &state, user: &user, headers: &headers, metadata: None };
        boundary.accept(&read).await?;
        drop(state);
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit task-owned Phase12 PostgreSQL"]
async fn phase19_disable_role_uses_actual_private_owner_and_rejects_hidden_scope_changes(
) -> Result<()> {
    let db = Owned::new().await?;
    let result = async {
        let (mut state, user) = authorized_fixture(&db.pool).await?;
        Arc::make_mut(&mut state.config).identity_auth_admin_service_client_id = Some(uuid7::new_uuid().to_string());
        sqlx::query("UPDATE menus SET status='DISABLED' WHERE permission='system:oauth:scopes'")
            .execute(&db.pool).await?;
        let client: Uuid = sqlx::query_scalar("SELECT id FROM auth_clients LIMIT 1").fetch_one(&db.pool).await?;
        let metadata = CanonicalMetadata { pool: db.pool.clone() };
        let current = metadata.client(client).await?;
        let keys = ["applicationId","environment","displayName","clientType","runtimeKind",
            "subjectMode","contextMode","allowedMemberTypes","grantTypes","tokenEndpointAuthMethod",
            "redirectUris","postLogoutRedirectUris","logoutMode","backchannelLogoutUri","backchannelTargetId","consentGroupId","permissions"];
        let mut object = serde_json::Map::new();
        for key in keys { object.insert(key.into(), current[key].clone()); }
        object.insert("expectedVersion".into(), current["version"].clone());
        object.insert("status".into(), json!("DISABLED"));
        for permission in object["permissions"].as_array_mut().ok_or_else(|| anyhow!("missing metadata permissions"))? {
            permission.as_object_mut().ok_or_else(|| anyhow!("invalid metadata permission"))?.remove("audience");
        }
        let headers = HeaderMap::new();
        let boundary = PlatformBoundary { state: &state, user: &user, headers: &headers, metadata: Some(&metadata) };
        ensure!(Target::new(Method::PATCH, &format!("/clients/{}", uuid7::new_uuid())).is_err(),
            "retired client management target remained available");
        let target = Target::auth_client(Method::PATCH, &format!("/{client}"))?;
        let original = Value::Object(object);
        let body = prepare(&target, Some(original.clone()))?;
        let command = Command { digest: digest(&target,&body)?, target: target.clone(), body, operation: uuid7::new_uuid() };
        boundary.accept(&command).await?;
        let binding: (String,Option<Uuid>) = sqlx::query_as("SELECT management_permission,application_ref FROM platform_auth_command_intents WHERE id=$1")
            .bind(command.operation).fetch_one(&db.pool).await?;
        ensure!(binding == ("system:oauth:disable".into(), current["applicationId"].as_str().map(id).transpose()?),
            "pure disable did not persist exact authority and actual application owner");
        for (field,value) in [("displayName",json!("Hidden change")),("redirectUris",json!(["https://other.example.test/callback"]))] {
            let mut changed = original.clone(); changed[field] = value;
            let body = prepare(&target,Some(changed))?;
            let command = Command { digest: digest(&target,&body)?, target: target.clone(), body, operation: uuid7::new_uuid() };
            ensure!(matches!(boundary.accept(&command).await,Err(AppError::Forbidden(_))),
                "disable-only authority accepted hidden {field} change");
            ensure!(operation_counts(&db.pool,command.operation).await? == (0,0,0), "hidden change committed state");
        }
        let mut forged = original; forged["applicationId"] = json!(uuid7::new_uuid());
        let body = prepare(&target,Some(forged))?;
        let command = Command { digest: digest(&target,&body)?, target, body, operation: uuid7::new_uuid() };
        ensure!(matches!(boundary.accept(&command).await,Err(AppError::Forbidden(_))),
            "caller applicationId replaced the actual private client owner");
        drop(state);
        Ok(())
    }.await;
    db.finish(result).await
}

struct ReviewReadTransport(AtomicUsize);
impl Transport for ReviewReadTransport {
    fn send<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, Reply> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            let (application, environment) = command
                .target
                .path
                .strip_prefix(&format!("{REVIEW_PREFIX}/"))
                .and_then(|suffix| suffix.split_once('/'))
                .ok_or(AppError::Unavailable)?;
            Ok(Reply {
                status: 200,
                content_type: Some("application/json".into()),
                bytes: serde_json::to_vec(
                    &json!({"applicationId":application,"environment":environment,
                    "projectionVersion":null,"admissionVersion":null,"status":"UNREGISTERED"}),
                )
                .map_err(|_| AppError::Unavailable)?,
            })
        })
    }
}

#[tokio::test]
#[ignore = "requires explicit task-owned Phase12 PostgreSQL"]
async fn phase19_review_only_role_can_read_sync_metadata_without_general_view_and_requires_recent_totp(
) -> Result<()> {
    let db = Owned::new().await?;
    let result = async {
        let (state,user) = authorized_fixture(&db.pool).await?;
        sqlx::query("UPDATE menus SET status='DISABLED' WHERE permission LIKE 'system:oauth:%' AND permission<>'system:oauth:review'")
            .execute(&db.pool).await?;
        let application:Uuid=sqlx::query_scalar("SELECT application_id FROM auth_applications LIMIT 1")
            .fetch_one(&db.pool).await?;
        let headers=HeaderMap::new();
        let boundary=PlatformBoundary{state:&state,user:&user,headers:&headers,metadata:None};
        let transport=ReviewReadTransport(AtomicUsize::new(0));
        let target=Target::review_sync_read(application,"test","system:oauth:review")?;
        let command=Command{target,body:None,operation:uuid7::new_uuid(),digest:String::new()};
        let value=execute(&boundary,&transport,&command,fresh_deadline()).await
            .map_err(|_|anyhow!("review-only sync metadata read did not reach authenticated transport fixture"))?;
        ensure!(value["status"]=="UNREGISTERED" && transport.0.load(Ordering::SeqCst)==1,
            "authorized sync metadata read did not validate its fixed response");
        let ordinary=Command{target:Target::review(Method::GET,application,"test")?,body:None,
            operation:uuid7::new_uuid(),digest:String::new()};
        ensure!(matches!(execute(&boundary,&transport,&ordinary,fresh_deadline()).await,
            Err(Failure{error:AppError::Forbidden(_),..})),"write role silently gained general VIEW authority");
        sqlx::query("UPDATE sso_sessions SET mfa_verified_at=clock_timestamp()-interval '6 minutes' WHERE user_id=$1")
            .bind(user.user_id).execute(&db.pool).await?;
        ensure!(matches!(execute(&boundary,&transport,&command,fresh_deadline()).await,
            Err(Failure{error:AppError::Forbidden(_),..})),"sensitive metadata read accepted stale TOTP");
        ensure!(transport.0.load(Ordering::SeqCst)==1,"denied metadata read reached transport");
        let intents:i64=sqlx::query_scalar("SELECT count(*) FROM platform_auth_command_intents")
            .fetch_one(&db.pool).await?;
        ensure!(intents==0,"metadata read created a mutation intent");
        drop(state);Ok(())
    }.await;
    db.finish(result).await
}
