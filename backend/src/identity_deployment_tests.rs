//! Phase 12 real LOGIN-role and ownership tests. Only explicitly selected fresh
//! databases are migrated. Passwords stay in process memory and never in output.

use crate::util::{hash, uuid7};
use anyhow::{anyhow, ensure, Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use std::{collections::HashMap, str::FromStr, time::Duration};
use uuid::Uuid;

const ISSUER: &str = "https://auth.saas.test";
const CREATE_ROLES: &str = include_str!("../ops/identity-deployment/create-roles.sql");
const PROVISION: &str = include_str!("../ops/identity-deployment/provision.sql");
const VERIFY: &str = include_str!("../ops/identity-deployment/verify.sql");

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn render(script: &str, values: &HashMap<&str, String>) -> Result<String> {
    let mut sql = script
        .lines()
        .filter_map(|line| {
            if line.trim() == "\\gset" {
                Some(";".to_owned())
            } else if line.trim_start().starts_with('\\') {
                None
            } else {
                Some(line.to_owned())
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    for (key, value) in values {
        sql = sql.replace(&format!(":'{key}'"), &quote(value));
    }
    ensure!(!sql.contains(":'"), "unresolved deployment SQL input");
    Ok(sql)
}

struct Database {
    pool: PgPool,
    admin: PgPool,
    identity: Option<PgPool>,
    platform: Option<PgPool>,
    name: String,
    prefix: String,
    deployment: Uuid,
    options: PgConnectOptions,
}
impl Database {
    async fn create() -> Result<Self> {
        let url = std::env::var("IDENTITY_PHASE12_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE12_TEST_DATABASE_URL is required"))?;
        let options = PgConnectOptions::from_str(&url)
            .map_err(|_| anyhow!("invalid Phase12 administration configuration"))?;
        ensure!(
            options.get_database() == Some("postgres"),
            "Phase12 administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("Phase12 administration connection failed"))?;
        let deployment = uuid7::new_uuid();
        let name = format!("identity_phase12_{}", deployment.simple());
        let prefix = format!("id12_{}", &deployment.simple().to_string()[..20]);
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = match PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options.clone().database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH(FORCE)"))
                    .execute(&admin)
                    .await;
                return Err(anyhow!("Phase12 owned database connection failed"));
            }
        };
        let db = Self {
            pool,
            admin,
            identity: None,
            platform: None,
            name,
            prefix,
            deployment,
            options,
        };
        if let Err(error) = sqlx::migrate!("./migrations").run(&db.pool).await {
            let _ = db.finish(Ok(())).await;
            return Err(error.into());
        }
        Ok(db)
    }
    fn values(&self) -> HashMap<&str, String> {
        HashMap::from([
            ("expected_database", self.name.clone()),
            ("deployment_id", self.deployment.to_string()),
            ("issuer", ISSUER.into()),
            ("role_prefix", self.prefix.clone()),
        ])
    }
    fn role(&self, suffix: &str) -> String {
        format!("{}_{}", self.prefix, suffix)
    }
    async fn provision(&mut self) -> Result<()> {
        sqlx::query("INSERT INTO oidc_provider_settings(singleton,issuer,created_at) VALUES(true,$1,clock_timestamp())").bind(ISSUER).execute(&self.pool).await?;
        sqlx::query(&format!(
            "COMMENT ON DATABASE \"{}\" IS {}",
            self.name,
            quote(&format!("identity-deployment/1/{}", self.deployment))
        ))
        .execute(&self.pool)
        .await?;
        sqlx::raw_sql(&render(CREATE_ROLES, &self.values())?)
            .execute(&self.pool)
            .await?;
        sqlx::raw_sql(&render(PROVISION, &self.values())?)
            .execute(&self.pool)
            .await?;
        self.verify().await?;
        for suffix in ["identity_runtime", "platform_runtime"] {
            let role = self.role(suffix);
            let password = hash::sha256_hex(&uuid7::new_uuid().to_string());
            sqlx::query(&format!(
                "ALTER ROLE \"{role}\" PASSWORD {}",
                quote(&password)
            ))
            .execute(&self.pool)
            .await?;
            let runtime = PgPoolOptions::new()
                .max_connections(4)
                .acquire_timeout(Duration::from_secs(5))
                .connect_with(
                    self.options
                        .clone()
                        .database(&self.name)
                        .username(&role)
                        .password(&password),
                )
                .await
                .map_err(|_| anyhow!("Phase12 real runtime connection failed"))?;
            let principal: (String, String) =
                sqlx::query_as("SELECT current_user::text,session_user::text")
                    .fetch_one(&runtime)
                    .await?;
            ensure!(
                principal == (role.clone(), role),
                "runtime test used SET ROLE instead of the real LOGIN identity"
            );
            if suffix == "identity_runtime" {
                self.identity = Some(runtime);
            } else {
                self.platform = Some(runtime);
            }
        }
        Ok(())
    }
    async fn verify(&self) -> Result<()> {
        let mut connection = self.pool.acquire().await?;
        let result = sqlx::raw_sql(&render(VERIFY, &self.values())?)
            .execute(&mut *connection)
            .await;
        if result.is_err() {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
        }
        result?;
        Ok(())
    }
    fn identity(&self) -> Result<&PgPool> {
        self.identity
            .as_ref()
            .ok_or_else(|| anyhow!("identity runtime not provisioned"))
    }
    fn platform(&self) -> Result<&PgPool> {
        self.platform
            .as_ref()
            .ok_or_else(|| anyhow!("platform runtime not provisioned"))
    }
    async fn finish(mut self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("identity_phase12_")
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && self.prefix.starts_with("id12_")
                && self
                    .prefix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "refusing cleanup outside Phase12-owned database/roles"
        );
        if let Some(pool) = self.identity.take() {
            pool.close().await;
        }
        if let Some(pool) = self.platform.take() {
            pool.close().await;
        }
        let _ = sqlx::query("ROLLBACK").execute(&self.pool).await;
        let closed = tokio::time::timeout(Duration::from_secs(8), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH(FORCE)", self.name))
            .execute(&self.admin)
            .await;
        let mut roles_removed = true;
        for suffix in [
            "aux_permission",
            "identity_runtime",
            "platform_runtime",
            "bridge_owner",
            "platform_owner",
            "identity_owner",
            "migration_owner",
        ] {
            roles_removed &= sqlx::query(&format!("DROP ROLE IF EXISTS \"{}\"", self.role(suffix)))
                .execute(&self.admin)
                .await
                .is_ok();
        }
        self.admin.close().await;
        eprintln!("identity_phase12_cleanup database={} assertions_ok={} database_removed={} roles_removed={roles_removed} pool_closed={closed}",self.name,result.is_ok(),removed.is_ok());
        result?;
        removed?;
        ensure!(closed && roles_removed, "Phase12 owned cleanup incomplete");
        Ok(())
    }
}

async fn denied(pool: &PgPool, sql: &str, label: &str) -> Result<()> {
    match sqlx::raw_sql(sql).execute(pool).await {
        Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("42501") => Ok(()),
        Err(_) => Err(anyhow!(
            "{label}: failed for a reason other than PostgreSQL insufficient_privilege"
        )),
        Ok(_) => Err(anyhow!(
            "{label}: forbidden operation unexpectedly succeeded"
        )),
    }
}

async fn assert_definer_owner_privileges(
    pool: &PgPool,
    function: &str,
    owner: &str,
    table: &str,
) -> Result<()> {
    let actual: (String, bool, bool, bool) = sqlx::query_as(
        "SELECT pg_catalog.pg_get_userbyid(p.proowner)::text,p.prosecdef, \
         pg_catalog.has_table_privilege(p.proowner,$2,'SELECT'), \
         pg_catalog.has_table_privilege(p.proowner,$2,'UPDATE') \
         FROM pg_catalog.pg_proc p WHERE p.oid=pg_catalog.to_regprocedure($1)",
    )
    .bind(function)
    .bind(table)
    .fetch_one(pool)
    .await?;
    ensure!(
        actual == (owner.to_owned(), true, true, true),
        "reviewed definer lost its exact NOLOGIN owner or that owner's ordinary table privileges"
    );
    Ok(())
}

async fn nonowner_membership_escape_is_rejected(db: &Database) -> Result<()> {
    let aux = db.role("aux_permission");
    sqlx::raw_sql(&format!("CREATE ROLE \"{aux}\" NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; \
        GRANT USAGE ON SCHEMA public TO \"{aux}\"; \
        GRANT EXECUTE ON FUNCTION public.is_uuid_v7(uuid) TO \"{aux}\"; \
        GRANT SELECT(password_hash),UPDATE(is_super_admin) ON public.users TO \"{aux}\";"))
        .execute(&db.pool).await?;
    let result=async {
        for (suffix,pool,forbidden_column,privilege) in [
            ("platform_runtime",db.platform()?,"password_hash","SELECT"),
            ("identity_runtime",db.identity()?,"is_super_admin","UPDATE"),
        ] {
            let role=db.role(suffix);
            sqlx::query(&format!("GRANT \"{aux}\" TO \"{role}\"")).execute(&db.pool).await?;
            let mut connection=pool.acquire().await?;
            let unchanged:(String,String,bool,bool,bool)=sqlx::query_as(
                "SELECT current_user::text,session_user::text, \
                 pg_catalog.has_column_privilege(current_user,'public.users',$1,$2), \
                 EXISTS(SELECT 1 FROM public.identity_deployment_state d WHERE \
                    pg_catalog.pg_has_role(current_user,d.migration_owner,'MEMBER') \
                    OR pg_catalog.pg_has_role(current_user,d.identity_owner,'MEMBER') \
                    OR pg_catalog.pg_has_role(current_user,d.platform_owner,'MEMBER') \
                    OR pg_catalog.pg_has_role(current_user,d.revocation_bridge_owner,'MEMBER')), \
                 NOT EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members m JOIN pg_catalog.pg_roles r ON r.oid=m.member WHERE r.rolname=current_user)")
                .bind(forbidden_column).bind(privilege).fetch_one(&mut *connection).await?;
            ensure!(unchanged==(role.clone(),role.clone(),false,false,false),
                "NOINHERIT auxiliary-role fixture did not preserve the original principal, hidden forbidden privilege and owner nonmembership");
            let mut verifier=db.pool.acquire().await?;
            let rejected=sqlx::raw_sql(&render(VERIFY,&db.values())?).execute(&mut *verifier).await;
            let _=sqlx::query("ROLLBACK").execute(&mut *verifier).await;
            ensure!(matches!(rejected,Err(sqlx::Error::Database(ref error)) if error.message()=="IDENTITY_DEPLOYMENT_RUNTIME_MEMBERSHIP_INVALID"),
                "deployment verification accepted a permission-bearing nonowner membership");
            let switched=async {
                sqlx::query(&format!("SET ROLE \"{aux}\"")).execute(&mut *connection).await?;
                let changed:(String,String)=sqlx::query_as("SELECT current_user::text,session_user::text").fetch_one(&mut *connection).await?;
                ensure!(changed==(aux.clone(),role.clone()),"auxiliary SET ROLE bypass was not exercised through the actual LOGIN session");
                let credential:String=sqlx::query_scalar("SELECT password_hash FROM public.users LIMIT 1").fetch_one(&mut *connection).await?;
                ensure!(credential=="phase12-task-owned-phc-placeholder","auxiliary role could not demonstrate the forbidden credential read");
                // Common UUID CHECK execution/schema USAGE are the only
                // housekeeping grants; no additional business columns exist.
                sqlx::query("UPDATE public.users SET is_super_admin=$1").bind(suffix=="identity_runtime").execute(&mut *connection).await?;
                Ok::<(),anyhow::Error>(())
            }.await;
            let reset=sqlx::query("RESET ROLE").execute(&mut *connection).await;
            switched?;reset?;drop(connection);
            sqlx::query(&format!("REVOKE \"{aux}\" FROM \"{role}\"")).execute(&db.pool).await?;
            db.verify().await?;
        }
        Ok::<(),anyhow::Error>(())
    }.await;
    // Both the success and assertion-error paths remove only this fresh
    // task-owned role, its memberships and its grants in the owned database.
    let cleanup = sqlx::raw_sql(&format!(
        "REVOKE \"{aux}\" FROM \"{}\",\"{}\"; DROP OWNED BY \"{aux}\"; DROP ROLE \"{aux}\";",
        db.role("identity_runtime"),
        db.role("platform_runtime")
    ))
    .execute(&db.pool)
    .await;
    result?;
    cleanup?;
    db.verify().await?;
    Ok(())
}

async fn intent_lock_wait_expiry(
    db: &Database,
    fixture: &Fixture,
    admin_intent: bool,
) -> Result<()> {
    let operation = uuid7::new_uuid();
    let service = uuid7::new_uuid();
    let digest = "c".repeat(64);
    let (table, expiry, query) = if admin_intent {
        let expiry:DateTime<Utc>=sqlx::query_scalar("INSERT INTO platform_oidc_command_intents(id,actor_user_id,tenant_id,organization_id,session_id,service_client_id,request_digest,method,path,accepted_at) VALUES($1,$2,$3,$4,$5,$6,$7,'POST','/internal/identity/oidc/clients',clock_timestamp()-interval '59 seconds') RETURNING accepted_at+interval '60 seconds'")
            .bind(operation).bind(fixture.user).bind(fixture.tenant).bind(fixture.org).bind(fixture.sessions[0]).bind(service).bind(&digest).fetch_one(db.platform()?).await?;
        ("platform_oidc_command_intents",expiry,format!("SELECT public.identity_oidc_command_intent_active('{operation}','{service}','{digest}','POST','/internal/identity/oidc/clients')"))
    } else {
        let expiry:DateTime<Utc>=sqlx::query_scalar("INSERT INTO platform_account_command_intents(id,target_user_id,actor_user_id,actor_tenant_id,actor_organization_id,actor_session_id,expected_account_version,canonical_digest,command_kind,expires_at,state,followup,created_at) VALUES($1,$2,$2,$3,$4,$5,3,$6,'REVOKE_ALL',clock_timestamp()+interval '1 second','PENDING_SECRET','{}',clock_timestamp()) RETURNING expires_at")
            .bind(operation).bind(fixture.user).bind(fixture.tenant).bind(fixture.org).bind(fixture.sessions[0]).bind(&digest).fetch_one(db.platform()?).await?;
        ("platform_account_command_intents",expiry,format!("SELECT public.platform_account_command_intent_active('{operation}','{}',3,'{digest}','REVOKE_ALL')",fixture.user))
    };
    let mut owner = db.pool.begin().await?;
    sqlx::query(&format!(
        "SELECT id FROM public.{table} WHERE id=$1 FOR UPDATE"
    ))
    .bind(operation)
    .execute(&mut *owner)
    .await?;
    let mut connection = db.identity()?.acquire().await?;
    sqlx::query("SET statement_timeout='4s'")
        .execute(&mut *connection)
        .await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *connection)
        .await?;
    let pending = tokio::spawn(async move {
        sqlx::query_scalar::<_, bool>(&query)
            .fetch_one(&mut *connection)
            .await
    });
    let started = std::time::Instant::now();
    let mut blocked = false;
    while started.elapsed() < Duration::from_secs(2) {
        blocked = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')",
        )
        .bind(pid)
        .fetch_one(&db.pool)
        .await?;
        if blocked {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Wait for the server's actual deadline, not an assumed sleep duration. The
    // row remains unchanged, so only a fresh post-lock statement can reject it.
    while started.elapsed() < Duration::from_secs(3) {
        let expired: bool = sqlx::query_scalar("SELECT clock_timestamp()>$1::timestamptz")
            .bind(expiry)
            .fetch_one(&db.pool)
            .await?;
        if expired {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    owner.commit().await?;
    let accepted = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .map_err(|_| anyhow!("intent post-lock proof did not complete"))???;
    ensure!(
        blocked,
        "intent expiry fixture did not reach the real PostgreSQL lock barrier"
    );
    ensure!(
        !accepted,
        "unchanged intent that expired while waiting was accepted after lock acquisition"
    );
    Ok(())
}

struct Fixture {
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    member: Uuid,
    sessions: Vec<Uuid>,
    tokens: Vec<Uuid>,
}
impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        let user = uuid7::new_uuid();
        let tenant = uuid7::new_uuid();
        let org = uuid7::new_uuid();
        let member = uuid7::new_uuid();
        sqlx::query("INSERT INTO users(id,username,password_hash,session_version) VALUES($1,$2,'phase12-task-owned-phc-placeholder',3)").bind(user).bind(format!("phase12-{}",user.simple())).execute(pool).await?;
        sqlx::query("INSERT INTO tenants(id,name,code) VALUES($1,'Owned phase12 tenant',$2)")
            .bind(tenant)
            .bind(format!("phase12-{}", tenant.simple()))
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned phase12 organization')").bind(org).bind(tenant).execute(pool).await?;
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,identity_version) VALUES($1,$2,$3,'EMPLOYEE',5)").bind(member).bind(user).bind(org).execute(pool).await?;
        let mut sessions = Vec::new();
        let mut tokens = Vec::new();
        for kind in ["access", "refresh"] {
            let sid = uuid7::new_uuid();
            let token = uuid7::new_uuid();
            sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) VALUES($1,$2,$3,$4,clock_timestamp()+interval '1 hour',3,$5,'EMPLOYEE',5,'urn:saas:legacy-platform','platform-console-legacy',1)")
                .bind(sid).bind(user).bind(tenant).bind(org).bind(member).execute(pool).await?;
            sqlx::query("INSERT INTO platform_session_refresh_tokens(id,session_id,token_hash,session_version,expires_at) VALUES($1,$2,$3,3,clock_timestamp()+interval '30 minutes')")
                .bind(uuid7::new_uuid()).bind(sid).bind(hash::sha256_hex(&sid.to_string())).execute(pool).await?;
            sqlx::query("INSERT INTO oauth_tokens(id,token_hash,token_type,user_id,client_id,expires_at) VALUES($1,$2,$3,$4,'platform-console-legacy',clock_timestamp()+interval '20 minutes')")
                .bind(token).bind(hash::sha256_hex(&token.to_string())).bind(kind).bind(user).execute(pool).await?;
            sessions.push(sid);
            tokens.push(token);
        }
        sqlx::query("INSERT INTO oauth_authorization_codes(id,code_hash,client_id,user_id,redirect_uri,expires_at) VALUES($1,$2,'platform-console-legacy',$3,'https://phase12.example.invalid/callback',clock_timestamp()+interval '5 minutes')")
            .bind(uuid7::new_uuid()).bind(hash::sha256_hex(&user.to_string())).bind(user).execute(pool).await?;
        sqlx::query("INSERT INTO oauth_device_codes(id,device_code_hash,user_code,client_id,user_id,status,expires_at) VALUES($1,$2,'OWNED12','platform-console-legacy',$3,'PENDING',clock_timestamp()+interval '5 minutes')")
            .bind(uuid7::new_uuid()).bind(hash::sha256_hex(&member.to_string())).bind(user).execute(pool).await?;
        Ok(Self {
            user,
            tenant,
            org,
            member,
            sessions,
            tokens,
        })
    }
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase12_real_roles_enforce_mixed_user_columns_and_no_ddl_owner_escape() -> Result<()> {
    let mut db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;db.provision().await?;
        let identity=db.identity()?;let platform=db.platform()?;
        let account:(Uuid,String)=sqlx::query_as("SELECT id,password_hash FROM users WHERE id=$1").bind(fixture.user).fetch_one(identity).await?;
        ensure!(account.0==fixture.user&&account.1=="phase12-task-owned-phc-placeholder","identity could not read its own credential column");
        sqlx::query("UPDATE users SET real_name='identity profile edit' WHERE id=$1").bind(fixture.user).execute(identity).await?;
        let mut tx=platform.begin().await?;
        let locked:Uuid=sqlx::query_scalar("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(fixture.user).fetch_one(&mut *tx).await?;
        ensure!(locked==fixture.user,"safe platform row-lock qualification failed");tx.commit().await?;
        sqlx::query("UPDATE users SET is_super_admin=true WHERE id=$1").bind(fixture.user).execute(platform).await?;
        for (sql,label) in [
            ("SELECT password_hash FROM users","platform PHC select"),("SELECT * FROM users","platform whole-row credential read"),
            ("UPDATE users SET password_hash='forged'","platform PHC update"),("UPDATE users SET session_version=session_version+1","platform account generation write"),
            ("UPDATE users SET real_name='forged'","platform profile write"),("SELECT * FROM oidc_clients","platform private OIDC read"),
            ("UPDATE oidc_clients SET status='DISABLED'","platform private OIDC write"),
            ("INSERT INTO identity_service_clients(id,secret_hash,purpose,status,created_at) VALUES('019a174e-0000-7000-8000-000000000099',repeat('a',64),'OIDC_ADMIN','ENABLED',clock_timestamp())","platform internal caller forgery")
        ] {denied(platform,sql,label).await?;}
        for (sql,label) in [
            ("SELECT is_super_admin FROM users","identity platform flag read"),("UPDATE users SET is_super_admin=true","identity platform flag write"),
            ("UPDATE users SET id=id","identity stable-subject update"),("DELETE FROM users","identity cascading account delete"),("TRUNCATE users CASCADE","identity cascading truncate"),
            ("SELECT * FROM memberships","identity membership read"),("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1","identity membership write"),
            ("SELECT * FROM roles","identity role read"),("DELETE FROM user_roles","identity role delete"),
            ("UPDATE identity_service_clients SET purpose='OIDC_ADMIN'","runtime service-purpose rewrite"),
            ("SELECT public.revoke_membership_credentials('019a174e-0000-7000-8000-000000000099')","identity generic platform invalidation helper")
        ] {denied(identity,sql,label).await?;}
        for pool in [identity,platform] {
            for (sql,label) in [("CREATE TABLE public.runtime_escape(id int)","runtime schema create"),("CREATE TEMP TABLE runtime_escape(id int)","runtime temp create"),
                ("ALTER TABLE users ADD COLUMN runtime_escape int","runtime table alter"),("CREATE SCHEMA runtime_escape","runtime schema create"),("CREATE ROLE runtime_escape","runtime role create")] {denied(pool,sql,label).await?;}
            for suffix in ["migration_owner","identity_owner","platform_owner","bridge_owner"] {denied(pool,&format!("SET ROLE \"{}\"",db.role(suffix)),"runtime owner SET ROLE").await?;}
        }
        let unchanged:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM users),(SELECT count(*) FROM memberships),(SELECT count(*) FROM platform_sessions)").fetch_one(&db.pool).await?;
        ensure!(unchanged==(1,1,2),"negative cascading or privilege tests changed platform relations");
        nonowner_membership_escape_is_rejected(&db).await?;
        db.verify().await?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase12_auth_generation_bridge_revokes_only_credentials_and_records_actual_counts(
) -> Result<()> {
    let mut db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;db.provision().await?;
        sqlx::query("UPDATE users SET status='DISABLED' WHERE id=$1").bind(fixture.user).execute(db.identity()?).await?;
        let fact:(i64,i64,i64)=sqlx::query_as("SELECT account_version,platform_session_count,oauth_token_count FROM identity_account_revocation_facts WHERE user_id=$1").bind(fixture.user).fetch_one(db.identity()?).await?;
        ensure!(fact==(4,2,2),"cohost bridge double-counted, lost or overwrote actual generation revocations");
        let retired:bool=sqlx::query_scalar("SELECT (SELECT bool_and(revoked_at IS NOT NULL) FROM platform_sessions WHERE id=ANY($1)) \
            AND (SELECT bool_and(revoked) FROM oauth_tokens WHERE id=ANY($2)) \
            AND (SELECT bool_and(consumed_at IS NOT NULL) FROM platform_session_refresh_tokens) \
            AND (SELECT bool_and(consumed) FROM oauth_authorization_codes) \
            AND (SELECT bool_and(status='EXPIRED') FROM oauth_device_codes)")
            .bind(&fixture.sessions).bind(&fixture.tokens).fetch_one(db.platform()?).await?;
        ensure!(retired,"bridge did not atomically retire every existing platform credential kind");
        let member:(String,i64)=sqlx::query_as("SELECT status,identity_version FROM memberships WHERE id=$1").bind(fixture.member).fetch_one(db.platform()?).await?;
        ensure!(member==("ENABLED".into(),5),"account bridge changed a platform membership");
        denied(db.identity()?,"UPDATE platform_sessions SET revoked_at=NULL","identity cannot resurrect a platform session").await?;
        denied(db.identity()?,"UPDATE identity_account_revocation_facts SET platform_session_count=999","identity cannot forge the bridge fact").await?;
        denied(db.platform()?,"SELECT * FROM identity_account_revocation_facts","platform does not read private auth command facts").await?;
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(fixture.user).execute(db.identity()?).await?;
        let next:(i64,i64)=sqlx::query_as("SELECT platform_session_count,oauth_token_count FROM identity_account_revocation_facts WHERE user_id=$1 AND account_version=5").bind(fixture.user).fetch_one(db.identity()?).await?;
        ensure!(next==(0,0),"new empty generation reused another command's old counts");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase12_narrow_projections_and_command_fences_hold_exact_original_bindings() -> Result<()>
{
    let mut db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;
        bind_sso_fixture(&db.pool, &f).await?;
        db.provision().await?;
        let identity=db.identity()?;let platform=db.platform()?;
        for table in ["public.platform_sessions","public.memberships"] {
            assert_definer_owner_privileges(identity,"public.platform_password_context_active(uuid,uuid,uuid,uuid,bigint)",&db.role("platform_owner"),table).await?;
        }
        let mut command=identity.begin().await?;
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(f.user).execute(&mut *command).await?;
        let active:bool=sqlx::query_scalar("SELECT platform_password_context_active($1,$2,$3,$4,3)")
            .bind(f.user).bind(f.sessions[0]).bind(f.tenant).bind(f.org).fetch_one(&mut *command).await
            .context("identity runtime invoking platform-owner original personal context fence")?;
        ensure!(active,"identity's narrow original personal context fence failed");
        let mut revoke=platform.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='100ms'").execute(&mut *revoke).await?;
        let blocked=sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1").bind(f.member).execute(&mut *revoke).await;
        ensure!(matches!(blocked,Err(sqlx::Error::Database(ref error)) if error.code().as_deref()==Some("55P03")),"member revocation was not fenced until identity command commit");
        revoke.rollback().await?;command.rollback().await?;
        sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1").bind(f.member).execute(platform).await?;
        let active:bool=sqlx::query_scalar("SELECT platform_password_context_active($1,$2,$3,$4,3)")
            .bind(f.user).bind(f.sessions[0]).bind(f.tenant).bind(f.org).fetch_one(identity).await?;
        ensure!(!active,"retired original member was repaired by a current account");
        denied(identity,"SELECT * FROM platform_sessions","fence gives no raw platform table read").await?;
        denied(platform,&format!("SELECT platform_password_context_active('{}','{}','{}','{}',3)",f.user,f.sessions[0],f.tenant,f.org),"personal fence signature only identity EXEC").await?;
        let operation=uuid7::new_uuid();let service=uuid7::new_uuid();let digest="a".repeat(64);
        sqlx::query("INSERT INTO platform_oidc_command_intents(id,actor_user_id,tenant_id,organization_id,session_id,service_client_id,request_digest,method,path,accepted_at) VALUES($1,$2,$3,$4,$5,$6,$7,'POST','/internal/identity/oidc/clients',clock_timestamp())")
            .bind(operation).bind(f.user).bind(f.tenant).bind(f.org).bind(f.sessions[0]).bind(service).bind(&digest).execute(platform).await?;
        for (service_ref,digest_ref,expected) in [(service,digest.as_str(),true),(uuid7::new_uuid(),digest.as_str(),false),(service,"wrong",false)] {
            let accepted:bool=sqlx::query_scalar("SELECT identity_oidc_command_intent_active($1,$2,$3,'POST','/internal/identity/oidc/clients')")
                .bind(operation).bind(service_ref).bind(digest_ref).fetch_one(identity).await?;
            ensure!(accepted==expected,"OIDC admin acceptance crossed its original service/digest tuple");
        }
        denied(identity,"SELECT * FROM platform_oidc_command_intents","identity raw admin intent read").await?;
        denied(platform,"UPDATE platform_oidc_command_intents SET request_digest=repeat('b',64)","immutable runtime admin acceptance").await?;
        sqlx::query("UPDATE platform_oidc_command_intents SET accepted_at=clock_timestamp()-interval '61 seconds' WHERE id=$1").bind(operation).execute(&db.pool).await?;
        let expired:bool=sqlx::query_scalar("SELECT identity_oidc_command_intent_active($1,$2,$3,'POST','/internal/identity/oidc/clients')")
            .bind(operation).bind(service).bind(digest).fetch_one(identity).await?;
        ensure!(!expired,"expired new admin command acceptance remained valid");
        intent_lock_wait_expiry(&db,&f,false).await?;
        intent_lock_wait_expiry(&db,&f,true).await?;
        Ok(())
    }.await;
    db.finish(result).await
}

struct OidcSource {
    client: Uuid,
    grant: Uuid,
    sid: Uuid,
    digest: String,
    auth_time: i64,
    expires: DateTime<Utc>,
}
async fn oidc_source(pool: &PgPool, user: Uuid, customer: bool) -> Result<OidcSource> {
    let client = uuid7::new_uuid();
    let sid = uuid7::new_uuid();
    let grant = uuid7::new_uuid();
    let digest = hash::sha256_hex(&uuid7::new_uuid().to_string());
    sqlx::query("INSERT INTO oidc_clients(id,name,purpose,secret_hash,allowed_member_types,redirect_uris,status,access_policy_version,created_at,updated_at) VALUES($1,'Owned phase12 RP',$2,$3,$4,ARRAY['https://phase12.example.invalid/callback'],'ENABLED',1,clock_timestamp(),clock_timestamp())")
        .bind(client).bind(if customer{"CUSTOMER_PORTAL"}else{"PLATFORM_CONSOLE"})
        .bind(hash::sha256_hex(&client.to_string())).bind(if customer{vec!["CUSTOMER"]}else{vec!["EMPLOYEE"]}).execute(pool).await?;
    sqlx::query("INSERT INTO sso_sessions(id,user_id,token_hash,csrf_hash,account_version,authenticated_at,expires_at) VALUES($1,$2,$3,$4,3,clock_timestamp(),clock_timestamp()+interval '1 hour')")
        .bind(sid).bind(user).bind(hash::sha256_hex(&sid.to_string())).bind(hash::sha256_hex(&format!("csrf-{sid}"))).execute(pool).await?;
    sqlx::query("INSERT INTO oidc_grants(id,client_id,client_policy_version,session_id,user_id,account_version,scope,nonce,auth_time,created_at,expires_at) VALUES($1,$2,1,$3,$4,3,ARRAY['openid','profile'],'owned',date_trunc('second',clock_timestamp()),clock_timestamp(),clock_timestamp()+interval '30 minutes')")
        .bind(grant).bind(client).bind(sid).bind(user).execute(pool).await?;
    sqlx::query("INSERT INTO oidc_access_tokens(id,grant_id,token_hash,created_at,expires_at) VALUES($1,$2,$3,clock_timestamp(),clock_timestamp()+interval '5 minutes')")
        .bind(uuid7::new_uuid()).bind(grant).bind(&digest).execute(pool).await?;
    let (auth_time,expires):(i64,DateTime<Utc>)=sqlx::query_as("SELECT floor(extract(epoch FROM g.auth_time))::bigint,LEAST(t.expires_at,g.expires_at,s.expires_at) FROM oidc_access_tokens t JOIN oidc_grants g ON g.id=t.grant_id JOIN sso_sessions s ON s.id=g.session_id WHERE g.id=$1")
        .bind(grant).fetch_one(pool).await?;
    Ok(OidcSource {
        client,
        grant,
        sid,
        digest,
        auth_time,
        expires,
    })
}

// Permission/fence fixtures use real SSO source rows; historical password-mode
// sessions are no longer valid positive inputs after migration 0021.
async fn bind_sso_fixture(pool: &PgPool, fixture: &Fixture) -> Result<()> {
    let source = oidc_source(pool, fixture.user, false).await?;
    let browser = uuid7::new_uuid();
    let bff = uuid7::new_uuid();
    let hash = hash::sha256_hex(&bff.to_string());
    sqlx::query("INSERT INTO console_browsers(id,token_hash,generation,current_session_id,created_at,expires_at) VALUES($1,$2,1,$3,clock_timestamp(),clock_timestamp()+interval '1 hour')")
        .bind(browser).bind(&hash).bind(bff).execute(pool).await?;
    sqlx::query("INSERT INTO bff_sessions(id,browser_id,token_hash,csrf_hash,user_id,account_version,global_session_id,oidc_client_id,oidc_grant_id,oidc_client_policy_version,issuer,auth_time,nonce_hash,access_ciphertext,refresh_ciphertext,access_expires_at,version,selected_session_id,created_at,expires_at) VALUES($1,$2,$3,$3,$4,3,$5,$6,$7,1,$8,$9,$3,'owned-fixture-only','owned-fixture-only',$10,1,$11,clock_timestamp(),clock_timestamp()+interval '30 minutes')")
        .bind(bff).bind(browser).bind(&hash).bind(fixture.user).bind(source.sid).bind(source.client)
        .bind(source.grant).bind(ISSUER).bind(source.auth_time).bind(source.expires).bind(fixture.sessions[0])
        .execute(pool).await?;
    sqlx::query("UPDATE platform_sessions SET bff_session_id=$1,bff_context_version=1,origin_issuer=$2,origin_client_ref=$3,origin_client_policy_version=1 WHERE id=$4")
        .bind(bff).bind(ISSUER).bind(source.client.to_string()).bind(fixture.sessions[0]).execute(pool).await?;
    Ok(())
}

async fn customer_projection(pool: &PgPool, user: Uuid, source: &OidcSource) -> Result<bool> {
    sqlx::query_scalar("SELECT customer_portal_account_source_active($1,$2,$3,$4,$5,$6,3,1,$7,$8)")
        .bind(&source.digest)
        .bind(ISSUER)
        .bind(source.client)
        .bind(source.grant)
        .bind(source.sid)
        .bind(user)
        .bind(source.auth_time)
        .bind(source.expires)
        .fetch_one(pool)
        .await
        .context(
            "platform runtime invoking authentication-owner original customer access projection",
        )
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase12_auth_owned_bool_projections_preserve_rp_type_and_lock_without_raw_auth_grants(
) -> Result<()> {
    let mut db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;
        let console=oidc_source(&db.pool,f.user,false).await?;
        let customer=oidc_source(&db.pool,f.user,true).await?;
        db.provision().await?;
        let platform=db.platform()?;let identity=db.identity()?;
        assert_definer_owner_privileges(platform,"public.bff_oidc_proof_active(uuid,uuid,uuid,bigint,uuid,bigint,text)",&db.role("identity_owner"),"public.oidc_grants").await?;
        let console_active:bool=sqlx::query_scalar("SELECT bff_oidc_proof_active($1,$2,$3,3,$4,1,$5)")
            .bind(f.user).bind(console.sid).bind(console.grant).bind(console.client).bind(ISSUER).fetch_one(platform).await
            .context("platform runtime invoking authentication-owner BFF account projection")?;
        ensure!(console_active,"auth-owned console projection failed without raw auth SELECT");
        ensure!(customer_projection(platform,f.user,&customer).await?,"auth-owned original access projection failed without raw auth SELECT");
        ensure!(!customer_projection(platform,f.user,&console).await?,"employee RP became a customer source");
        let wrong_console:bool=sqlx::query_scalar("SELECT bff_oidc_proof_active($1,$2,$3,3,$4,1,$5)")
            .bind(f.user).bind(customer.sid).bind(customer.grant).bind(customer.client).bind(ISSUER).fetch_one(platform).await?;
        ensure!(!wrong_console,"customer RP became a console source");
        let wrapper:bool=sqlx::query_scalar("SELECT platform_source_active($1,$2,1,'EMPLOYEE')").bind(ISSUER).bind(console.client.to_string()).fetch_one(platform).await?;
        ensure!(wrapper,"platform wrapper did not delegate to the auth-only projection");
        let mut lock=platform.begin().await?;
        for (client,expected) in [(console.client.to_string(),true),(console.client.to_string().to_uppercase(),false),(customer.client.to_string(),false)] {
            let found:bool=sqlx::query_scalar("SELECT identity_lock_oidc_source($1,$2)").bind(client).bind(ISSUER).fetch_one(&mut *lock).await?;
            ensure!(found==expected,"source lock accepted a noncanonical or wrong-purpose RP");
        }
        let mut policy=identity.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='100ms'").execute(&mut *policy).await?;
        let blocked=sqlx::query("UPDATE oidc_clients SET status='DISABLED',access_policy_version=access_policy_version+1 WHERE id=$1").bind(console.client).execute(&mut *policy).await;
        ensure!(matches!(blocked,Err(sqlx::Error::Database(ref error)) if error.code().as_deref()==Some("55P03")),"narrow source lock did not survive until platform commit");
        policy.rollback().await?;lock.rollback().await?;
        sqlx::query("UPDATE oidc_clients SET status='DISABLED',access_policy_version=access_policy_version+1 WHERE id=$1").bind(console.client).execute(identity).await?;
        let retired:bool=sqlx::query_scalar("SELECT bff_oidc_proof_active($1,$2,$3,3,$4,1,$5)")
            .bind(f.user).bind(console.sid).bind(console.grant).bind(console.client).bind(ISSUER).fetch_one(platform).await?;
        ensure!(!retired&&customer_projection(platform,f.user,&customer).await?,"console policy retirement crossed RP identity");
        denied(platform,"SELECT secret_hash FROM oidc_clients","projection cannot reveal client secret hashes").await?;
        denied(identity,&format!("SELECT identity_lock_oidc_source('{}','{ISSUER}')",console.client),"identity does not get the platform-only source lock signature").await?;
        db.verify().await?;
        Ok(())
    }.await;
    db.finish(result).await
}
