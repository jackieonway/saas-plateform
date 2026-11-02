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
mod phase13;
mod phase14;
mod phase15;
mod phase15_http;
mod phase20;

pub(crate) async fn canonical_employee_source(
    pool: &PgPool,
    user: Uuid,
) -> Result<(Uuid, Uuid, Uuid, i64, DateTime<Utc>)> {
    let source = phase13::console_source(pool, user).await?;
    Ok((
        source.client,
        source.grant,
        source.sid,
        source.auth_time,
        source.expires,
    ))
}

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
        Self::create_with_namespace(
            "IDENTITY_PHASE12_TEST_DATABASE_URL",
            "identity_phase12_",
            "id12_",
        )
        .await
    }
    async fn create_phase02() -> Result<Self> {
        Self::create_with_namespace(
            "IDENTITY_PHASE02_TEST_DATABASE_URL",
            "identity_phase02_",
            "id02_",
        )
        .await
    }
    async fn create_with_namespace(
        database_url_variable: &str,
        database_prefix: &str,
        role_prefix: &str,
    ) -> Result<Self> {
        let url = std::env::var(database_url_variable)
            .map_err(|_| anyhow!("explicit isolated database administration URL is required"))?;
        let options = PgConnectOptions::from_str(&url)
            .map_err(|_| anyhow!("invalid isolated administration configuration"))?;
        ensure!(
            options.get_database() == Some("postgres"),
            "isolated administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated PostgreSQL administration connection failed"))?;
        let deployment = uuid7::new_uuid();
        let simple = deployment.simple().to_string();
        let name = format!("{database_prefix}{simple}");
        // UUIDv7 timestamps and a monotonic random prefix may be identical for
        // concurrent fixtures. Keep the random tail in role names as well.
        let prefix = format!("{role_prefix}{}", &simple[12..]);
        eprintln!("identity_test_resource database={name} role_prefix={prefix}");
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
                return Err(anyhow!("task-owned isolated database connection failed"));
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
        sqlx::query("INSERT INTO auth_provider_settings(singleton,issuer,created_at) VALUES(true,$1,clock_timestamp())").bind(ISSUER).execute(&self.pool).await?;
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
        let phase12_namespace =
            self.name.starts_with("identity_phase12_") && self.prefix.starts_with("id12_");
        let phase02_namespace =
            self.name.starts_with("identity_phase02_") && self.prefix.starts_with("id02_");
        let phase15_http_namespace =
            self.name.starts_with("identity_phase15_http_") && self.prefix.starts_with("id15h_");
        ensure!(
            (phase12_namespace || phase02_namespace || phase15_http_namespace)
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && self
                    .prefix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "refusing cleanup outside an explicitly task-owned database/role namespace"
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
        eprintln!("identity_test_cleanup database={} assertions_ok={} database_removed={} roles_removed={roles_removed} pool_closed={closed}",self.name,result.is_ok(),removed.is_ok());
        result?;
        removed?;
        ensure!(
            closed && roles_removed,
            "task-owned database/role cleanup incomplete"
        );
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

async fn constraint_rejected(pool: &PgPool, sql: &str, code: &str, label: &str) -> Result<()> {
    match sqlx::raw_sql(sql).execute(pool).await {
        Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some(code) => Ok(()),
        Err(_) => Err(anyhow!(
            "{label}: rejected for an unexpected PostgreSQL error"
        )),
        Ok(_) => Err(anyhow!("{label}: invalid write unexpectedly succeeded")),
    }
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn phase02_auth_model_constraints_and_runtime_boundary() -> Result<()> {
    let mut db = Database::create_phase02().await?;
    let result = async {
        let checksum: Vec<u8> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version=1")
                .fetch_one(&db.pool)
                .await?;
        let history_before: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&db.pool)
            .await?;
        sqlx::migrate!("./migrations").run(&db.pool).await?;
        let checksum_after: Vec<u8> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version=1")
                .fetch_one(&db.pool)
                .await?;
        let history_after: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&db.pool)
            .await?;
        ensure!(checksum == checksum_after && history_before == 1 && history_after == 1,
            "rechecking canonical version 1 changed migration history");

        let fixture = Fixture::create(&db.pool).await?;
        let fresh_user = uuid7::new_uuid();
        let fresh_generation: i64 = sqlx::query_scalar("INSERT INTO users(id,username,password_hash) VALUES($1,$2,'owned-canonical-schema-placeholder') RETURNING session_version")
            .bind(fresh_user).bind(format!("phase02-default-{}",fresh_user.simple())).fetch_one(&db.pool).await?;
        ensure!(fresh_generation==1,"fresh canonical account must begin at generation 1");
        constraint_rejected(&db.pool,&format!("UPDATE users SET session_version=0 WHERE id='{fresh_user}'"),"23514","canonical account generation cannot be zero").await?;
        let application = uuid7::new_uuid();
        let partner = uuid7::new_uuid();
        let resource = uuid7::new_uuid();
        let scope = uuid7::new_uuid();
        let client = uuid7::new_uuid();
        let public_client = uuid7::new_uuid();
        let credential = uuid7::new_uuid();
        let group = uuid7::new_uuid();
        let consent = uuid7::new_uuid();
        let sso_session = uuid7::new_uuid();
        let sector = uuid7::new_uuid();
        let subject_link = uuid7::new_uuid();
        let request = uuid7::new_uuid();
        let code = uuid7::new_uuid();
        let grant = uuid7::new_uuid();
        let access = uuid7::new_uuid();
        let family = uuid7::new_uuid();
        let refresh = uuid7::new_uuid();
        let projection_service = uuid7::new_uuid();
        let auth_admin_service = uuid7::new_uuid();

        sqlx::query("INSERT INTO identity_service_clients(id,secret_hash,purpose,status,created_at) VALUES($1,repeat('a',64),'APPLICATION_PROJECTION','ENABLED',clock_timestamp()),($2,repeat('b',64),'AUTH_ADMIN','ENABLED',clock_timestamp())")
            .bind(projection_service).bind(auth_admin_service).execute(&db.pool).await?;
        let service_purposes: i64 = sqlx::query_scalar("SELECT count(*) FROM identity_service_clients WHERE purpose IN ('APPLICATION_PROJECTION','AUTH_ADMIN') AND status='ENABLED'")
            .fetch_one(&db.pool).await?;
        ensure!(service_purposes == 2, "current canonical schema rejected a dedicated projection/admin purpose");

        sqlx::query("INSERT INTO auth_applications(application_id,partner_id,ownership,status,display_name,purpose,approved_scopes,projection_version) VALUES($1,$2,'THIRD_PARTY','APPROVED','Phase 2 fixture','Account-only test',ARRAY['openid','profile'],1)")
            .bind(application).bind(partner).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_resources(id,resource_key,audience,status,version) VALUES($1,'phase2-profile','https://resource.example.test/profile','ACTIVE',1)")
            .bind(resource).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_scopes(id,resource_id,scope_name,description,status,version) VALUES($1,$2,'profile:read','Read account profile','ACTIVE',1)")
            .bind(scope).bind(resource).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,allowed_member_types,grant_types,token_endpoint_auth_method,redirect_uris,post_logout_redirect_uris,status) VALUES($1,'phase2-account-client','Phase 2 account client',$2,'test','CONFIDENTIAL','WEB_BFF','USER','ACCOUNT_ONLY',ARRAY[]::text[],ARRAY['authorization_code','refresh_token']::text[],'client_secret_basic',ARRAY['https://client.example.test/callback'],ARRAY[]::text[],'ENABLED')")
            .bind(client).bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,allowed_member_types,grant_types,token_endpoint_auth_method,redirect_uris,post_logout_redirect_uris,status) VALUES($1,'phase2-public-client','Phase 2 public client',$2,'test','PUBLIC','SPA','USER','ACCOUNT_ONLY',ARRAY[]::text[],ARRAY['authorization_code']::text[],'none',ARRAY['https://public.example.test/callback'],ARRAY[]::text[],'ENABLED')")
            .bind(public_client).bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_client_credentials(id,client_id,credential_version,secret_digest) VALUES($1,$2,1,$3)")
            .bind(credential).bind(client).bind("a".repeat(64)).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_client_permissions(client_id,resource_id,scope_id) VALUES($1,$2,$3)")
            .bind(client).bind(resource).bind(scope).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_consent_groups(id,application_id,environment,display_name,purpose,subject_mode,context_mode,status) VALUES($1,$2,'test','Phase 2 clients','Account sign-in','USER','ACCOUNT_ONLY','ACTIVE')")
            .bind(group).bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_consent_group_clients(group_id,client_id,application_id,environment,subject_mode,context_mode) VALUES($1,$2,$3,'test','USER','ACCOUNT_ONLY')")
            .bind(group).bind(client).bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_consents(id,user_id,group_id,status,generation,version,authorized_at) VALUES($1,$2,$3,'ACTIVE',1,1,clock_timestamp())")
            .bind(consent).bind(fixture.user).bind(group).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_consent_scopes(consent_id,resource_id,scope_id) VALUES($1,$2,$3)")
            .bind(consent).bind(resource).bind(scope).execute(&db.pool).await?;
        sqlx::query("INSERT INTO sso_sessions(id,user_id,token_hash,csrf_hash,account_version,authenticated_at,expires_at) VALUES($1,$2,$3,$4,3,clock_timestamp(),clock_timestamp()+interval '1 hour')")
            .bind(sso_session).bind(fixture.user).bind(hash::sha256_hex("phase02-session-token")).bind(hash::sha256_hex("phase02-session-csrf")).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_subject_sectors(id,application_id,environment,status) VALUES($1,$2,'test','ACTIVE')")
            .bind(sector).bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_subject_links(id,user_id,sector_id,subject) VALUES($1,$2,$3,$4)")
            .bind(subject_link).bind(fixture.user).bind(sector).bind("sub_phase2_opaque_fixture_0001").execute(&db.pool).await?;
        let state_value = "phase02-state-value-1234567890";
        let nonce_value = "phase02-nonce-value-1234567890";
        sqlx::query("INSERT INTO auth_authorization_requests(id,client_id,user_id,source_session_id,account_version,auth_time,client_policy_version,application_projection_version,redirect_uri,state_digest,state_value,nonce_digest,nonce_value,code_challenge,code_challenge_method,scope_ids,browser_digest,csrf_digest,force_login,force_consent,max_age_seconds,prior_session_id,status,expires_at) VALUES($1,$2,$3,$4,3,clock_timestamp(),1,1,'https://client.example.test/callback',$5,$6,$7,$8,$9,'S256',ARRAY[$10]::uuid[],$11,$12,false,false,NULL,NULL,'AUTHORIZED',clock_timestamp()+interval '5 minutes')")
            .bind(request).bind(client).bind(fixture.user).bind(sso_session).bind(hash::sha256_hex(state_value)).bind(state_value).bind(hash::sha256_hex(nonce_value)).bind(nonce_value).bind("d".repeat(43)).bind(scope).bind(hash::sha256_hex("browser-binding")).bind(hash::sha256_hex("csrf-binding")).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_authorization_codes(id,request_id,client_id,user_id,consent_id,consent_generation,consent_group_version,code_digest,expires_at) VALUES($1,$2,$3,$4,$5,1,1,$6,clock_timestamp()+interval '60 seconds')")
            .bind(code).bind(request).bind(client).bind(fixture.user).bind(consent).bind("e".repeat(64)).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_grants(id,client_id,subject_mode,user_id,consent_id,consent_generation,consent_group_version,source_session_id,context_mode,account_version,client_policy_version,application_projection_version,scope_ids,audiences,auth_time,expires_at) VALUES($1,$2,'USER',$3,$4,1,1,$5,'ACCOUNT_ONLY',3,1,1,ARRAY[$6]::uuid[],ARRAY['https://resource.example.test/profile']::text[],clock_timestamp(),clock_timestamp()+interval '1 hour')")
            .bind(grant).bind(client).bind(fixture.user).bind(consent).bind(sso_session).bind(scope).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_grant_scopes(grant_id,resource_id,scope_id) VALUES($1,$2,$3)")
            .bind(grant).bind(resource).bind(scope).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_access_tokens(id,grant_id,token_digest,expires_at) VALUES($1,$2,$3,clock_timestamp()+interval '5 minutes')")
            .bind(access).bind(grant).bind("f".repeat(64)).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_access_token_scopes(token_id,resource_id,scope_id) VALUES($1,$2,$3)")
            .bind(access).bind(resource).bind(scope).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_refresh_families(id,grant_id,absolute_expires_at,idle_expires_at) VALUES($1,$2,clock_timestamp()+interval '7 days',clock_timestamp()+interval '24 hours')")
            .bind(family).bind(grant).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_refresh_tokens(id,family_id,token_digest,generation,expires_at) VALUES($1,$2,$3,1,clock_timestamp()+interval '24 hours')")
            .bind(refresh).bind(family).bind("1".repeat(64)).execute(&db.pool).await?;

        db.provision().await?;
        let runtime: (String, String, String) = sqlx::query_as(
            "SELECT current_user::text, a.client_id, c.secret_digest FROM auth_client_credentials c JOIN auth_clients a ON a.id=c.client_id WHERE c.id=$1",
        )
        .bind(credential)
        .fetch_one(db.identity()?)
        .await?;
        ensure!(runtime.0 == db.role("identity_runtime") && runtime.1 == "phase2-account-client" && runtime.2 == "a".repeat(64),
            "identity LOGIN runtime cannot use its own unified credential model");
        denied(db.platform()?, "SELECT secret_digest FROM auth_client_credentials", "platform secret projection read").await?;
        denied(db.platform()?, "UPDATE auth_clients SET status='DISABLED'", "platform identity client write").await?;
        denied(db.identity()?, "SELECT * FROM memberships", "identity membership read").await?;
        denied(db.identity()?, "SELECT * FROM roles", "identity business role read").await?;
        denied(db.identity()?, "CREATE TABLE public.phase02_runtime_escape(id int)", "identity runtime DDL").await?;
        denied(db.platform()?, "CREATE TABLE public.phase02_platform_escape(id int)", "platform runtime DDL").await?;

        constraint_rejected(&db.pool,
            &format!("INSERT INTO auth_resources(id,resource_key,audience,status,version) VALUES('{}','phase2-profile','https://duplicate.example.test','ACTIVE',1)",uuid7::new_uuid()),
            "23505","resource key uniqueness").await?;
        constraint_rejected(&db.pool,
            &format!("UPDATE auth_clients SET version=0 WHERE id='{}'",client),
            "23514","client optimistic version lower bound").await?;
        constraint_rejected(&db.pool,
            &format!("INSERT INTO auth_client_credentials(id,client_id,credential_version,secret_digest) VALUES('{}','{}',1,'{}')",uuid7::new_uuid(),public_client,"9".repeat(64)),
            "23503","public client secret credential reference").await?;
        constraint_rejected(&db.pool,
            &format!("INSERT INTO auth_client_permissions(client_id,resource_id,scope_id) VALUES('{}','{}','{}')",client,uuid7::new_uuid(),scope),
            "23503","resource-qualified scope reference").await?;
        constraint_rejected(&db.pool,
            &format!("INSERT INTO auth_consents(id,user_id,group_id,status,generation,version,authorized_at) VALUES('{}','{}','{}','ACTIVE',1,1,clock_timestamp())",uuid7::new_uuid(),fixture.user,group),
            "23505","one consent generation row per account and group").await?;
        constraint_rejected(&db.pool,
            &format!("INSERT INTO auth_subject_links(id,user_id,sector_id,subject) VALUES('{}','{}','{}','sub_phase2_opaque_fixture_0002')",uuid7::new_uuid(),uuid7::new_uuid(),sector),
            "23503","subject link account reference").await?;
        db.verify().await?;
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn phase11_machine_grants_require_a_persisted_client_credential_version() -> Result<()> {
    let db = Database::create_phase02().await?;
    let result = async {
        let application = uuid7::new_uuid();
        let resource = uuid7::new_uuid();
        let scope = uuid7::new_uuid();
        let client = uuid7::new_uuid();
        let credential = uuid7::new_uuid();
        let grant = uuid7::new_uuid();
        let audience = "https://api.example.test/machine";
        sqlx::query(
            "INSERT INTO public.auth_applications(application_id,ownership,status,display_name,purpose,approved_scopes,projection_version,projected_at) \
             VALUES($1,'FIRST_PARTY','APPROVED','Machine test app','Approved service access',ARRAY['orders.read'],1,clock_timestamp())",
        )
        .bind(application)
        .execute(&db.pool)
        .await?;
        sqlx::query(
            "INSERT INTO public.auth_resources(id,resource_key,audience,status,version) \
             VALUES($1,'orders',$2,'ACTIVE',1)",
        )
        .bind(resource)
        .bind(audience)
        .execute(&db.pool)
        .await?;
        sqlx::query(
            "INSERT INTO public.auth_scopes(id,resource_id,scope_name,status) \
             VALUES($1,$2,'orders.read','ACTIVE')",
        )
        .bind(scope)
        .bind(resource)
        .execute(&db.pool)
        .await?;
        sqlx::query(
            "INSERT INTO public.auth_clients(id,client_id,display_name,application_id,environment,client_type, \
             runtime_kind,subject_mode,context_mode,allowed_member_types,grant_types,token_endpoint_auth_method, \
             redirect_uris,post_logout_redirect_uris,status,version,policy_version) \
             VALUES($1,'phase11-schema-machine','Phase 11 machine',$2,'test','CONFIDENTIAL','SERVICE','MACHINE','NONE', \
             ARRAY[]::text[],ARRAY['client_credentials']::text[],'client_secret_basic',ARRAY[]::text[],ARRAY[]::text[], \
             'ENABLED',1,1)",
        )
        .bind(client)
        .bind(application)
        .execute(&db.pool)
        .await?;
        sqlx::query(
            "INSERT INTO public.auth_client_credentials(id,client_id,client_type,credential_version,secret_digest) \
             VALUES($1,$2,'CONFIDENTIAL',1,$3)",
        )
        .bind(credential)
        .bind(client)
        .bind("a".repeat(64))
        .execute(&db.pool)
        .await?;
        sqlx::query(
            "INSERT INTO public.auth_client_permissions(client_id,resource_id,scope_id) VALUES($1,$2,$3)",
        )
        .bind(client)
        .bind(resource)
        .bind(scope)
        .execute(&db.pool)
        .await?;
        sqlx::query(
            "INSERT INTO public.auth_grants(id,client_id,subject_mode,user_id,consent_id,consent_generation, \
             consent_group_version,source_session_id,context_mode,context_ref,account_version,client_policy_version, \
             application_projection_version,credential_version,scope_ids,audiences,auth_time,expires_at) \
             VALUES($1,$2,'MACHINE',NULL,NULL,NULL,NULL,NULL,'NONE',NULL,NULL,1,1,1,ARRAY[$3]::uuid[], \
             ARRAY[$4]::text[],NULL,clock_timestamp()+interval '5 minutes')",
        )
        .bind(grant)
        .bind(client)
        .bind(scope)
        .bind(audience)
        .execute(&db.pool)
        .await?;
        sqlx::query(
            "INSERT INTO public.auth_grant_scopes(grant_id,resource_id,scope_id) VALUES($1,$2,$3)",
        )
        .bind(grant)
        .bind(resource)
        .bind(scope)
        .execute(&db.pool)
        .await?;
        let stored: (String, Option<Uuid>, Option<Uuid>, String, Option<i64>) = sqlx::query_as(
            "SELECT subject_mode,user_id,consent_id,context_mode,credential_version \
             FROM public.auth_grants WHERE id=$1",
        )
        .bind(grant)
        .fetch_one(&db.pool)
        .await?;
        ensure!(
            stored == ("MACHINE".to_owned(), None, None, "NONE".to_owned(), Some(1)),
            "canonical MACHINE grant stored a user context or lost its credential version"
        );

        let missing_version = sqlx::query(
            "INSERT INTO public.auth_grants(id,client_id,subject_mode,user_id,consent_id,consent_generation, \
             consent_group_version,source_session_id,context_mode,context_ref,account_version,client_policy_version, \
             application_projection_version,credential_version,scope_ids,audiences,auth_time,expires_at) \
             VALUES($1,$2,'MACHINE',NULL,NULL,NULL,NULL,NULL,'NONE',NULL,NULL,1,1,NULL,ARRAY[$3]::uuid[], \
             ARRAY[$4]::text[],NULL,clock_timestamp()+interval '5 minutes')",
        )
        .bind(uuid7::new_uuid())
        .bind(client)
        .bind(scope)
        .bind(audience)
        .execute(&db.pool)
        .await;
        ensure!(
            matches!(missing_version, Err(sqlx::Error::Database(ref error)) if error.code().as_deref() == Some("23514")),
            "canonical schema accepted a MACHINE grant without credential_version"
        );
        let unknown_version = sqlx::query(
            "INSERT INTO public.auth_grants(id,client_id,subject_mode,user_id,consent_id,consent_generation, \
             consent_group_version,source_session_id,context_mode,context_ref,account_version,client_policy_version, \
             application_projection_version,credential_version,scope_ids,audiences,auth_time,expires_at) \
             VALUES($1,$2,'MACHINE',NULL,NULL,NULL,NULL,NULL,'NONE',NULL,NULL,1,1,99,ARRAY[$3]::uuid[], \
             ARRAY[$4]::text[],NULL,clock_timestamp()+interval '5 minutes')",
        )
        .bind(uuid7::new_uuid())
        .bind(client)
        .bind(scope)
        .bind(audience)
        .execute(&db.pool)
        .await;
        ensure!(
            matches!(unknown_version, Err(sqlx::Error::Database(ref error)) if error.code().as_deref() == Some("23503")),
            "canonical schema accepted a MACHINE grant bound to an unregistered credential version"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;
    db.finish(result).await
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
        let expiry:DateTime<Utc>=sqlx::query_scalar("INSERT INTO platform_auth_command_intents(id,actor_user_id,tenant_id,organization_id,session_id,service_client_id,request_digest,method,path,management_permission,accepted_at) VALUES($1,$2,$3,$4,$5,$6,$7,'POST','/internal/identity/auth/clients','system:oauth:register',clock_timestamp()-interval '59 seconds') RETURNING accepted_at+interval '60 seconds'")
            .bind(operation).bind(fixture.user).bind(fixture.tenant).bind(fixture.org).bind(fixture.sessions[0]).bind(service).bind(&digest).fetch_one(db.platform()?).await?;
        ("platform_auth_command_intents",expiry,format!("SELECT public.identity_auth_command_intent_active('{operation}','{service}','{digest}','POST','/internal/identity/auth/clients')"))
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
        for _ in 0..2 {
            let sid = uuid7::new_uuid();
            sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) VALUES($1,$2,$3,$4,clock_timestamp()+interval '1 hour',3,$5,'EMPLOYEE',5,'https://unbound-source.example.invalid','019d0000-0000-7000-8000-00000000ffff',1)")
                .bind(sid).bind(user).bind(tenant).bind(org).bind(member).execute(pool).await?;
            sqlx::query("INSERT INTO platform_session_refresh_tokens(id,session_id,token_hash,session_version,expires_at) VALUES($1,$2,$3,3,clock_timestamp()+interval '30 minutes')")
                .bind(uuid7::new_uuid()).bind(sid).bind(hash::sha256_hex(&sid.to_string())).execute(pool).await?;
            sessions.push(sid);
        }
        Ok(Self {
            user,
            tenant,
            org,
            member,
            sessions,
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
            ("UPDATE users SET real_name='forged'","platform profile write"),("SELECT * FROM auth_clients","platform private client read"),
            ("UPDATE auth_clients SET status='DISABLED'","platform private client write"),
            ("SELECT * FROM auth_context_interactions","platform raw canonical context interaction read"),
            ("SELECT * FROM auth_consents","platform private saved approval read"),("UPDATE auth_consents SET revoked_at=clock_timestamp()","platform saved approval revocation"),
            ("INSERT INTO identity_service_clients(id,secret_hash,purpose,status,created_at) VALUES('019a174e-0000-7000-8000-000000000099',repeat('a',64),'AUTH_ADMIN','ENABLED',clock_timestamp())","platform internal caller forgery")
        ] {denied(platform,sql,label).await?;}
        for (sql,label) in [
            ("SELECT is_super_admin FROM users","identity platform flag read"),("UPDATE users SET is_super_admin=true","identity platform flag write"),
            ("UPDATE users SET id=id","identity stable-subject update"),("DELETE FROM users","identity cascading account delete"),("TRUNCATE users CASCADE","identity cascading truncate"),
            ("SELECT * FROM memberships","identity membership read"),("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1","identity membership write"),
            ("SELECT * FROM roles","identity role read"),("DELETE FROM user_roles","identity role delete"),
            ("UPDATE identity_service_clients SET purpose='KEY_ADMIN'","runtime service-purpose rewrite"),
            ("SELECT * FROM context_selection_services","identity platform selection services read"),
            ("SELECT * FROM context_selection_registrations","identity platform RP mapping read"),
            ("SELECT * FROM context_selections","identity platform selected member read"),
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
        let fact:(i64,i64)=sqlx::query_as("SELECT account_version,platform_session_count FROM identity_account_revocation_facts WHERE user_id=$1").bind(fixture.user).fetch_one(db.identity()?).await?;
        ensure!(fact==(4,2),"cohost bridge double-counted, lost or overwrote actual generation revocations");
        let retired:bool=sqlx::query_scalar("SELECT (SELECT bool_and(revoked_at IS NOT NULL) FROM platform_sessions WHERE id=ANY($1)) \
            AND (SELECT bool_and(consumed_at IS NOT NULL) FROM platform_session_refresh_tokens)")
            .bind(&fixture.sessions).fetch_one(db.platform()?).await?;
        ensure!(retired,"bridge did not atomically retire every existing platform credential kind");
        let member:(String,i64)=sqlx::query_as("SELECT status,identity_version FROM memberships WHERE id=$1").bind(fixture.member).fetch_one(db.platform()?).await?;
        ensure!(member==("ENABLED".into(),5),"account bridge changed a platform membership");
        denied(db.identity()?,"UPDATE platform_sessions SET revoked_at=NULL","identity cannot resurrect a platform session").await?;
        denied(db.identity()?,"UPDATE identity_account_revocation_facts SET platform_session_count=999","identity cannot forge the bridge fact").await?;
        denied(db.platform()?,"SELECT * FROM identity_account_revocation_facts","platform does not read private auth command facts").await?;
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(fixture.user).execute(db.identity()?).await?;
        let next:i64=sqlx::query_scalar("SELECT platform_session_count FROM identity_account_revocation_facts WHERE user_id=$1 AND account_version=5").bind(fixture.user).fetch_one(db.identity()?).await?;
        ensure!(next==0,"new empty generation reused another command's old counts");
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
        grant_command_administration_fixture(&db.pool, &f).await?;
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
        denied(identity,"SELECT * FROM platform_sessions","fence gives no raw platform table read").await?;
        denied(platform,&format!("SELECT platform_password_context_active('{}','{}','{}','{}',3)",f.user,f.sessions[0],f.tenant,f.org),"personal fence signature only identity EXEC").await?;
        let operation=uuid7::new_uuid();let service=uuid7::new_uuid();let digest="a".repeat(64);
        sqlx::query("INSERT INTO platform_auth_command_intents(id,actor_user_id,tenant_id,organization_id,session_id,service_client_id,request_digest,method,path,management_permission,accepted_at) VALUES($1,$2,$3,$4,$5,$6,$7,'POST','/internal/identity/auth/clients','system:oauth:register',clock_timestamp())")
            .bind(operation).bind(f.user).bind(f.tenant).bind(f.org).bind(f.sessions[0]).bind(service).bind(&digest).execute(platform).await?;
        for (service_ref,digest_ref,expected) in [(service,digest.as_str(),true),(uuid7::new_uuid(),digest.as_str(),false),(service,"wrong",false)] {
            let accepted:bool=sqlx::query_scalar("SELECT identity_auth_command_intent_active($1,$2,$3,'POST','/internal/identity/auth/clients')")
                .bind(operation).bind(service_ref).bind(digest_ref).fetch_one(identity).await?;
            ensure!(accepted==expected,"AUTH admin acceptance crossed its original service/digest tuple");
        }
        denied(identity,"SELECT * FROM platform_auth_command_intents","identity raw admin intent read").await?;
        denied(platform,"UPDATE platform_auth_command_intents SET request_digest=repeat('b',64)","immutable runtime admin acceptance").await?;
        sqlx::query("UPDATE platform_auth_command_intents SET accepted_at=clock_timestamp()-interval '61 seconds' WHERE id=$1").bind(operation).execute(&db.pool).await?;
        let expired:bool=sqlx::query_scalar("SELECT identity_auth_command_intent_active($1,$2,$3,'POST','/internal/identity/auth/clients')")
            .bind(operation).bind(service).bind(digest).fetch_one(identity).await?;
        ensure!(!expired,"expired new admin command acceptance remained valid");
        intent_lock_wait_expiry(&db,&f,false).await?;
        intent_lock_wait_expiry(&db,&f,true).await?;
        let retired_operation=uuid7::new_uuid();
        sqlx::query("INSERT INTO platform_auth_command_intents(id,actor_user_id,tenant_id,organization_id,session_id,service_client_id,request_digest,method,path,management_permission,accepted_at) VALUES($1,$2,$3,$4,$5,$6,$7,'POST','/internal/identity/auth/clients','system:oauth:register',clock_timestamp())")
            .bind(retired_operation).bind(f.user).bind(f.tenant).bind(f.org).bind(f.sessions[0]).bind(service).bind("d".repeat(64)).execute(platform).await?;
        let fresh_operation:bool=sqlx::query_scalar("SELECT identity_auth_command_intent_active($1,$2,$3,'POST','/internal/identity/auth/clients')")
            .bind(retired_operation).bind(service).bind("d".repeat(64)).fetch_one(identity).await?;
        ensure!(fresh_operation,"original operation did not start with a live administrative actor");
        sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1").bind(f.member).execute(platform).await?;
        let active:bool=sqlx::query_scalar("SELECT platform_password_context_active($1,$2,$3,$4,3)")
            .bind(f.user).bind(f.sessions[0]).bind(f.tenant).bind(f.org).fetch_one(identity).await?;
        ensure!(!active,"retired original member was repaired by a current account");
        for restore in [false,true] {
            if restore {
                sqlx::query("UPDATE memberships SET status='ENABLED',identity_version=identity_version+1 WHERE id=$1").bind(f.member).execute(platform).await?;
            }
            let accepted:bool=sqlx::query_scalar("SELECT identity_auth_command_intent_active($1,$2,$3,'POST','/internal/identity/auth/clients')")
                .bind(retired_operation).bind(service).bind("d".repeat(64)).fetch_one(identity).await?;
            ensure!(!accepted,"member restoration revived the original administrative operation");
            let active:bool=sqlx::query_scalar("SELECT platform_password_context_active($1,$2,$3,$4,3)")
                .bind(f.user).bind(f.sessions[0]).bind(f.tenant).bind(f.org).fetch_one(identity).await?;
            ensure!(!active,"member restoration repaired the retired original session");
        }
        Ok(())
    }.await;
    db.finish(result).await
}

// Owner-provisioned facts for the component-level administrative fence test.
// This is not a TOTP login test: canonical HTTPS suites exercise that ceremony.
async fn grant_command_administration_fixture(pool: &PgPool, fixture: &Fixture) -> Result<()> {
    let application = uuid7::new_uuid();
    let menu = uuid7::new_uuid();
    sqlx::query("UPDATE tenants SET is_platform=true WHERE id=$1")
        .bind(fixture.tenant)
        .execute(pool)
        .await?;
    sqlx::query("UPDATE users SET is_super_admin=true WHERE id=$1")
        .bind(fixture.user)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO applications(id,name,code,status,allowed_member_types) VALUES($1,'Owned command gate','phase12-command-gate','ENABLED',ARRAY['EMPLOYEE'])")
        .bind(application).execute(pool).await?;
    sqlx::query("INSERT INTO menus(id,application_id,type,title,permission) VALUES($1,$2,'BUTTON','Owned client registration','system:oauth:register')")
        .bind(menu).bind(application).execute(pool).await?;
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
        .bind(fixture.tenant)
        .bind(application)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
    )
    .bind(fixture.org)
    .bind(application)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
        .bind(fixture.tenant)
        .bind(menu)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
        .bind(fixture.org)
        .bind(menu)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO auth_totp_factors(user_id,secret_ciphertext) VALUES($1,decode(repeat('ab',32),'hex'))")
        .bind(fixture.user).execute(pool).await?;
    sqlx::query("UPDATE sso_sessions SET mfa_method='totp',mfa_verified_at=clock_timestamp() WHERE id=(SELECT b.global_session_id FROM bff_sessions b WHERE b.selected_session_id=$1)")
        .bind(fixture.sessions[0]).execute(pool).await?;
    Ok(())
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
    if !customer {
        return phase13::console_source(pool, user).await;
    }
    sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,identity_version) SELECT $1,$2,m.organization_id,'CUSTOMER',5 FROM memberships m WHERE m.user_id=$2 AND m.member_type='EMPLOYEE' ORDER BY m.id LIMIT 1 ON CONFLICT(user_id,organization_id,member_type) DO NOTHING")
        .bind(uuid7::new_uuid()).bind(user).execute(pool).await?;
    phase13::user_source(pool, user, "CUSTOMER").await
}

// Permission/fence fixtures attach the original canonical SSO source. Unbound
// internal sessions are intentionally invalid until this explicit binding.
async fn bind_sso_fixture(pool: &PgPool, fixture: &Fixture) -> Result<()> {
    let source = oidc_source(pool, fixture.user, false).await?;
    let browser = uuid7::new_uuid();
    let bff = uuid7::new_uuid();
    let hash = hash::sha256_hex(&bff.to_string());
    sqlx::query("INSERT INTO console_browsers(id,token_hash,generation,current_session_id,created_at,expires_at) VALUES($1,$2,1,$3,clock_timestamp(),clock_timestamp()+interval '1 hour')")
        .bind(browser).bind(&hash).bind(bff).execute(pool).await?;
    sqlx::query("INSERT INTO bff_sessions(id,browser_id,token_hash,csrf_hash,user_id,account_version,global_session_id,oidc_client_id,oidc_grant_id,oidc_client_policy_version,issuer,auth_time,nonce_hash,access_ciphertext,refresh_ciphertext,access_expires_at,version,selected_session_id,created_at,expires_at,oidc_public_sid,oidc_public_sub) VALUES($1,$2,$3,$3,$4,3,$5,$6,$7,1,$8,$9,$3,'owned-fixture-only','owned-fixture-only',$10,1,$11,clock_timestamp(),clock_timestamp()+interval '30 minutes',$3,'owned-pairwise-'||$4::text)")
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
        assert_definer_owner_privileges(platform,"public.bff_auth_proof_active(uuid,uuid,uuid,bigint,uuid,bigint,text)",&db.role("identity_owner"),"public.auth_grants").await?;
        let console_active:bool=sqlx::query_scalar("SELECT bff_auth_proof_active($1,$2,$3,3,$4,1,$5)")
            .bind(f.user).bind(console.sid).bind(console.grant).bind(console.client).bind(ISSUER).fetch_one(platform).await
            .context("platform runtime invoking authentication-owner BFF account projection")?;
        ensure!(console_active,"auth-owned console projection failed without raw auth SELECT");
        ensure!(customer_projection(platform,f.user,&customer).await?,"auth-owned original access projection failed without raw auth SELECT");
        ensure!(!customer_projection(platform,f.user,&console).await?,"employee RP became a customer source");
        let wrong_console:bool=sqlx::query_scalar("SELECT bff_auth_proof_active($1,$2,$3,3,$4,1,$5)")
            .bind(f.user).bind(customer.sid).bind(customer.grant).bind(customer.client).bind(ISSUER).fetch_one(platform).await?;
        ensure!(!wrong_console,"customer RP became a console source");
        let wrapper:bool=sqlx::query_scalar("SELECT platform_source_active($1,$2,1,'EMPLOYEE')").bind(ISSUER).bind(console.client.to_string()).fetch_one(platform).await?;
        ensure!(wrapper,"platform wrapper did not delegate to the auth-only projection");
        let mut lock=platform.begin().await?;
        for (client,expected) in [(console.client.to_string(),true),(console.client.to_string().to_uppercase(),false),(customer.client.to_string(),false)] {
            let found:bool=sqlx::query_scalar("SELECT identity_lock_auth_source($1,$2)").bind(client).bind(ISSUER).fetch_one(&mut *lock).await?;
            ensure!(found==expected,"source lock accepted a noncanonical or wrong-purpose RP");
        }
        for policy_change in [
            "UPDATE auth_applications SET projection_version=projection_version+1 WHERE application_id=(SELECT application_id FROM auth_clients WHERE id=$1)",
            "UPDATE auth_clients SET status='DISABLED',policy_version=policy_version+1 WHERE id=$1",
            "UPDATE auth_consent_groups SET version=version+1 WHERE id=(SELECT group_id FROM auth_consent_group_clients WHERE client_id=$1)",
        ] {
            let mut policy=identity.begin().await?;
            sqlx::query("SET LOCAL lock_timeout='100ms'").execute(&mut *policy).await?;
            let blocked=sqlx::query(policy_change).bind(console.client).execute(&mut *policy).await;
            ensure!(matches!(blocked,Err(sqlx::Error::Database(ref error)) if error.code().as_deref()==Some("55P03")),"narrow original app/client/group source lock did not survive until platform commit");
            policy.rollback().await?;
        }
        lock.rollback().await?;
        sqlx::query("UPDATE auth_clients SET status='DISABLED',policy_version=policy_version+1 WHERE id=$1").bind(console.client).execute(identity).await?;
        let retired:bool=sqlx::query_scalar("SELECT bff_auth_proof_active($1,$2,$3,3,$4,1,$5)")
            .bind(f.user).bind(console.sid).bind(console.grant).bind(console.client).bind(ISSUER).fetch_one(platform).await?;
        ensure!(!retired&&customer_projection(platform,f.user,&customer).await?,"console policy retirement crossed RP identity");
        denied(platform,"SELECT secret_digest FROM auth_client_credentials","projection cannot reveal client secret hashes").await?;
        denied(identity,&format!("SELECT identity_lock_auth_source('{}','{ISSUER}')",console.client),"identity does not get the platform-only source lock signature").await?;
        db.verify().await?;
        Ok(())
    }.await;
    db.finish(result).await
}
