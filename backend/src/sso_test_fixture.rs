//! Owned-database domain fixtures for a completed canonical SSO exchange.
//! The proof-only transport cannot issue credentials or serve an authorization
//! endpoint. Canonical protocol and HTTPS acceptance run in their own harness.
use anyhow::{ensure, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::util::{hash, uuid7};

pub(crate) async fn token(
    pool: &PgPool,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    secret: &str,
) -> Result<String> {
    let sid = session(pool, user, tenant, org).await?;
    let version = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let now = Utc::now().timestamp();
    crate::util::jwt::sign_internal(
        &crate::util::jwt::Claims {
            sub: user.to_string(),
            typ: "platform_session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            cid: None,
            scope: None,
            sid: Some(sid.to_string()),
            ver: Some(version),
            aud: Some(crate::util::jwt::PLATFORM_API_AUDIENCE.into()),
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        crate::util::jwt::InternalUse::PlatformSession,
        secret,
    )
}

pub(crate) async fn session(pool: &PgPool, user: Uuid, tenant: Uuid, org: Uuid) -> Result<Uuid> {
    let client = client();
    let application = fixed_id("019d0000-0000-7000-8000-00000000d002");
    let group = fixed_id("019d0000-0000-7000-8000-00000000d003");
    let global = uuid7::new_uuid();
    let grant = uuid7::new_uuid();
    let browser = uuid7::new_uuid();
    let bff = uuid7::new_uuid();
    let session = uuid7::new_uuid();
    let request = uuid7::new_uuid();
    let context = uuid7::new_uuid();
    let digest = hash::sha256_hex(&bff.to_string());
    let access = hash::sha256_hex(&global.to_string());
    let access_digest = hash::sha256_hex(&access);
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO auth_provider_settings(singleton,issuer,created_at) VALUES(true,$1,clock_timestamp()) ON CONFLICT(singleton) DO NOTHING")
        .bind(ISSUER).execute(&mut *tx).await?;
    let issuer: String =
        sqlx::query_scalar("SELECT issuer FROM auth_provider_settings WHERE singleton")
            .fetch_one(&mut *tx)
            .await?;
    ensure!(
        issuer == ISSUER,
        "domain fixture issuer does not match its owned database"
    );
    sqlx::query("INSERT INTO auth_applications(application_id,ownership,status,display_name,purpose,approved_scopes) VALUES($1,'FIRST_PARTY','APPROVED','Owned domain fixture','PLATFORM_CONSOLE',ARRAY['openid','profile']) ON CONFLICT(application_id) DO NOTHING")
        .bind(application).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,allowed_member_types,grant_types,token_endpoint_auth_method,redirect_uris,status) VALUES($1,$2,'Owned domain fixture',$3,'test','CONFIDENTIAL','WEB_BFF','USER','PLATFORM_CONTEXT',ARRAY['EMPLOYEE'],ARRAY['authorization_code','refresh_token'],'client_secret_basic',ARRAY[$4],'ENABLED') ON CONFLICT(id) DO NOTHING")
        .bind(client).bind(client.to_string()).bind(application).bind(REDIRECT).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO auth_resources(id,resource_key,audience,status) VALUES($1,'userinfo',$2,'ACTIVE') ON CONFLICT(resource_key) DO NOTHING")
        .bind(uuid7::new_uuid()).bind(format!("{ISSUER}/oauth/userinfo")).execute(&mut *tx).await?;
    let (resource, audience): (Uuid, String) =
        sqlx::query_as("SELECT id,audience FROM auth_resources WHERE resource_key='userinfo'")
            .fetch_one(&mut *tx)
            .await?;
    let mut scopes = Vec::new();
    for name in ["openid", "profile"] {
        sqlx::query("INSERT INTO auth_scopes(id,resource_id,scope_name,status) VALUES($1,$2,$3,'ACTIVE') ON CONFLICT(resource_id,scope_name) DO NOTHING")
            .bind(uuid7::new_uuid()).bind(resource).bind(name).execute(&mut *tx).await?;
        let scope: Uuid =
            sqlx::query_scalar("SELECT id FROM auth_scopes WHERE resource_id=$1 AND scope_name=$2")
                .bind(resource)
                .bind(name)
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query("INSERT INTO auth_client_permissions(client_id,resource_id,scope_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(client).bind(resource).bind(scope).execute(&mut *tx).await?;
        scopes.push(scope);
    }
    sqlx::query("INSERT INTO auth_consent_groups(id,application_id,environment,display_name,purpose,context_mode,status) VALUES($1,$2,'test','Owned domain consent','PLATFORM_CONSOLE','PLATFORM_CONTEXT','ACTIVE') ON CONFLICT(id) DO NOTHING")
        .bind(group).bind(application).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO auth_consent_group_clients(group_id,client_id,application_id,environment,context_mode) VALUES($1,$2,$3,'test','PLATFORM_CONTEXT') ON CONFLICT DO NOTHING")
        .bind(group).bind(client).bind(application).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO auth_consents(id,user_id,group_id,status,authorized_at) VALUES($1,$2,$3,'ACTIVE',clock_timestamp()) ON CONFLICT(user_id,group_id) DO NOTHING")
        .bind(uuid7::new_uuid()).bind(user).bind(group).execute(&mut *tx).await?;
    let consent: Uuid = sqlx::query_scalar("SELECT id FROM auth_consents WHERE user_id=$1 AND group_id=$2 AND status='ACTIVE' AND generation=1 AND group_version=1")
        .bind(user).bind(group).fetch_one(&mut *tx).await?;
    for scope in &scopes {
        sqlx::query("INSERT INTO auth_consent_scopes(consent_id,resource_id,scope_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(consent).bind(resource).bind(scope).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO sso_sessions(id,user_id,token_hash,csrf_hash,account_version,authenticated_at,expires_at) SELECT $1,id,$2,$2,session_version,date_trunc('second',clock_timestamp()),clock_timestamp()+interval '1 hour' FROM users WHERE id=$3")
        .bind(global).bind(&digest).bind(user).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO auth_authorization_requests(id,client_id,user_id,source_session_id,account_version,auth_time,client_policy_version,application_projection_version,redirect_uri,state_digest,state_value,nonce_digest,nonce_value,code_challenge,code_challenge_method,scope_ids,browser_digest,csrf_digest,status,expires_at,consumed_at) SELECT $1,$2,user_id,id,account_version,authenticated_at,1,1,$4,$5,'owned-state',$5,'owned-nonce',repeat('c',43),'S256',$6,$5,$5,'CONSUMED',clock_timestamp()+interval '5 minutes',clock_timestamp() FROM sso_sessions WHERE id=$3")
        .bind(request).bind(client).bind(global).bind(REDIRECT).bind(&digest).bind(&scopes).execute(&mut *tx).await?;
    let member: Uuid = sqlx::query_scalar("SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2 AND member_type='EMPLOYEE'")
        .bind(user).bind(org).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO auth_context_interactions(transaction_id,request_hash,client_id,client_policy_version,application_id,application_projection_version,redirect_uri,state_digest,nonce_digest,code_challenge,intent,authorization_request_id,source_digest,selected_membership_id,credential_ciphertext,selection_expires_at,expires_at) VALUES($1,$2,$3,1,$4,1,$5,$2,$2,repeat('c',43),'{\"kind\":\"LOGIN\"}',$6,$7,$8,'owned-fixture-only',clock_timestamp()+interval '2 minutes',clock_timestamp()+interval '5 minutes')")
        .bind(context).bind(hash::sha256_hex(&context.to_string())).bind(client).bind(application).bind(REDIRECT).bind(request).bind(&access_digest).bind(member).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO auth_grants(id,client_id,subject_mode,user_id,consent_id,consent_generation,consent_group_version,source_session_id,context_mode,context_ref,account_version,client_policy_version,application_projection_version,scope_ids,audiences,auth_time,expires_at) SELECT $1,$2,'USER',user_id,$3,1,1,id,'PLATFORM_CONTEXT',$4,account_version,1,1,$5,ARRAY[$6],authenticated_at,expires_at FROM sso_sessions WHERE id=$7")
        .bind(grant).bind(client).bind(consent).bind(context).bind(&scopes).bind(&audience).bind(global).execute(&mut *tx).await?;
    sqlx::query("UPDATE auth_context_interactions SET grant_id=$2,delivered_at=clock_timestamp(),credential_ciphertext=NULL WHERE transaction_id=$1")
        .bind(context).bind(grant).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO auth_refresh_families(id,grant_id,absolute_expires_at,idle_expires_at) SELECT $1,id,expires_at,expires_at FROM auth_grants WHERE id=$2")
        .bind(uuid7::new_uuid()).bind(grant).execute(&mut *tx).await?;
    let access_id = uuid7::new_uuid();
    sqlx::query("INSERT INTO auth_access_tokens(id,grant_id,token_digest,expires_at) SELECT $1,id,$3,expires_at FROM auth_grants WHERE id=$2")
        .bind(access_id).bind(grant).bind(&access_digest).execute(&mut *tx).await?;
    for scope in &scopes {
        sqlx::query(
            "INSERT INTO auth_grant_scopes(grant_id,resource_id,scope_id) VALUES($1,$2,$3)",
        )
        .bind(grant)
        .bind(resource)
        .bind(scope)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO auth_access_token_scopes(token_id,resource_id,scope_id) VALUES($1,$2,$3)",
        )
        .bind(access_id)
        .bind(resource)
        .bind(scope)
        .execute(&mut *tx)
        .await?;
    }
    let encrypted = crate::bff::CipherKeys::fixture().fixture_access(bff, &issuer, client, &access);
    sqlx::query("INSERT INTO console_browsers(id,token_hash,generation,current_session_id,created_at,expires_at) VALUES($1,$2,1,$3,clock_timestamp(),clock_timestamp()+interval '1 hour')")
        .bind(browser).bind(&digest).bind(bff).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO bff_sessions(id,browser_id,token_hash,csrf_hash,user_id,account_version,global_session_id,oidc_client_id,oidc_grant_id,oidc_client_policy_version,issuer,auth_time,nonce_hash,access_ciphertext,refresh_ciphertext,access_expires_at,version,selected_session_id,created_at,expires_at,oidc_public_sid,oidc_public_sub) SELECT $1,$2,$3,$3,user_id,account_version,source_session_id,client_id,id,1,$4,floor(extract(epoch FROM auth_time))::bigint,$3,$7,'owned-fixture-only',expires_at,1,$5,clock_timestamp(),expires_at,$3,$8 FROM auth_grants WHERE id=$6")
        .bind(bff).bind(browser).bind(&digest).bind(&issuer).bind(session).bind(grant).bind(encrypted).bind(subject(user)).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version,bff_session_id,bff_context_version) SELECT $1,u.id,$2,$3,clock_timestamp()+interval '1 hour',u.session_version,m.id,m.member_type,m.identity_version,$4,$5,1,$6,1 FROM users u JOIN memberships m ON m.user_id=u.id AND m.organization_id=$3 AND m.member_type='EMPLOYEE' WHERE u.id=$7")
        .bind(session).bind(tenant).bind(org).bind(issuer).bind(client.to_string()).bind(bff).bind(user).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(session)
}

/// An internal capability derived from the same canonical employee source as a
/// real host request. The target application's policy and source deadline are
/// copied from the owned database, never from a retired OAuth registration.
pub(crate) async fn app_capability(
    pool: &PgPool,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    application: Uuid,
    code: &str,
    secret: &str,
) -> Result<String> {
    let sid = session(pool, user, tenant, org).await?;
    let (version, expires): (i64, DateTime<Utc>) = sqlx::query_as("SELECT account_version,date_trunc('second',LEAST(expires_at,clock_timestamp()+interval '5 minutes')) FROM platform_sessions WHERE id=$1")
        .bind(sid).fetch_one(pool).await?;
    let actual_code: String = sqlx::query_scalar("SELECT code FROM applications WHERE id=$1")
        .bind(application)
        .fetch_one(pool)
        .await?;
    ensure!(
        actual_code == code,
        "capability fixture application and audience differ"
    );
    let now = Utc::now().timestamp();
    let id = uuid7::new_uuid();
    let token = crate::util::jwt::sign_internal(
        &crate::util::jwt::Claims {
            sub: user.to_string(),
            typ: "app_session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            cid: Some(code.into()),
            scope: Some("app:access".into()),
            sid: Some(sid.to_string()),
            ver: Some(version),
            aud: Some(code.into()),
            jti: Some(id.to_string()),
            iat: now,
            exp: expires.timestamp(),
        },
        crate::util::jwt::InternalUse::AppSession,
        secret,
    )?;
    let inserted = sqlx::query("INSERT INTO platform_app_capabilities(id,jti,token_hash,user_id,tenant_id,organization_id,application_id,audience,scope,platform_session_id,membership_id,member_type,identity_version,account_version,application_policy_version,origin_issuer,origin_client_ref,origin_client_policy_version,issued_at,expires_at) SELECT $1,$1,$2,s.user_id,s.tenant_id,s.organization_id,a.id,a.code,'app:access',s.id,s.membership_id,s.member_type,s.identity_version,s.account_version,a.access_policy_version,s.origin_issuer,s.origin_client_ref,s.origin_client_policy_version,$5,$6 FROM platform_sessions s JOIN applications a ON a.id=$4 WHERE s.id=$3")
        .bind(id).bind(hash::sha256_hex(&token)).bind(sid).bind(application).bind(DateTime::from_timestamp(now, 0)).bind(expires).execute(pool).await?.rows_affected();
    ensure!(
        inserted == 1,
        "capability fixture did not retain its exact source"
    );
    Ok(token)
}

/// Customer-shaped rows are negative inputs to the employee-only platform API.
/// They deliberately have no valid employee BFF source or new token issuer.
pub(crate) async fn session_for_member(
    pool: &PgPool,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    member_type: &str,
) -> Result<Uuid> {
    if member_type == "EMPLOYEE" {
        return session(pool, user, tenant, org).await;
    }
    ensure!(member_type == "CUSTOMER", "unknown fixture member type");
    let sid = uuid7::new_uuid();
    let inserted = sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) SELECT $1,u.id,$2,$3,clock_timestamp()+interval '1 hour',u.session_version,m.id,m.member_type,m.identity_version,$4,$5,1 FROM users u JOIN memberships m ON m.user_id=u.id AND m.organization_id=$3 AND m.member_type='CUSTOMER' WHERE u.id=$6")
        .bind(sid).bind(tenant).bind(org).bind(ISSUER).bind(client().to_string()).bind(user).execute(pool).await?.rows_affected();
    ensure!(
        inserted == 1,
        "customer negative fixture membership is missing"
    );
    Ok(sid)
}

const ISSUER: &str = "https://sso-fixture.example.invalid";
const REDIRECT: &str = "https://console-fixture.example.invalid/auth/callback";
const CLIENT_SECRET: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SOURCE_ID: &str = "owned-domain-source";
const SOURCE_SECRET: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
fn fixed_id(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("static UUIDv7 fixture identifier")
}
fn client() -> Uuid {
    fixed_id("019d0000-0000-7000-8000-00000000d001")
}
fn subject(user: Uuid) -> String {
    hash::sha256_hex(&format!("owned-pairwise:{user}:{}", client()))
}

/// Explicit proof-only transport injection for business-domain tests.
pub(crate) fn state(
    pool: PgPool,
    config: std::sync::Arc<crate::config::Config>,
) -> crate::state::AppState {
    let mut config = (*config).clone();
    config.oidc_issuer = Some(ISSUER.into());
    config.bff_oidc_issuer = Some(ISSUER.into());
    config.bff_oidc_client_id = Some(client());
    config.bff_oidc_client_secret = Some(CLIENT_SECRET.into());
    config.context_source_service_client_id = Some(SOURCE_ID.into());
    config.context_source_service_client_secret = Some(SOURCE_SECRET.into());
    config.bff_token_keys = Some(std::sync::Arc::new(crate::bff::CipherKeys::fixture()));
    let mut state = crate::state::AppState::new(pool.clone(), std::sync::Arc::new(config));
    state.bff_transport = std::sync::Arc::new(ProofOnly(pool));
    state
}

#[derive(sqlx::FromRow)]
struct Source {
    user_id: Uuid,
    source_session_id: Uuid,
    grant_id: Uuid,
    account_version: i64,
    client_policy_version: i64,
    auth_time: i64,
    expires_at: DateTime<Utc>,
    absolute_expires_at: DateTime<Utc>,
}
struct ProofOnly(PgPool);
impl ProofOnly {
    async fn source(&self, access: &str) -> Result<Option<Source>, crate::error::AppError> {
        sqlx::query_as("SELECT g.user_id,g.source_session_id,g.id AS grant_id,g.account_version,g.client_policy_version,floor(extract(epoch FROM g.auth_time))::bigint AS auth_time,LEAST(tok.expires_at,g.expires_at,s.expires_at) AS expires_at,LEAST(g.expires_at,s.expires_at) AS absolute_expires_at FROM auth_access_tokens tok JOIN auth_grants g ON g.id=tok.grant_id JOIN sso_sessions s ON s.id=g.source_session_id WHERE tok.token_digest=$1 AND g.client_id=$2 AND tok.revoked_at IS NULL AND tok.expires_at>clock_timestamp() AND (SELECT count(*) FROM auth_access_token_scopes ts WHERE ts.token_id=tok.id AND ts.scope_id=ANY(g.scope_ids))=cardinality(g.scope_ids) AND NOT EXISTS(SELECT 1 FROM auth_access_token_scopes ts WHERE ts.token_id=tok.id AND NOT (ts.scope_id=ANY(g.scope_ids))) AND bff_auth_proof_active(g.user_id,g.source_session_id,g.id,g.account_version,g.client_id,g.client_policy_version,$3)")
            .bind(hash::sha256_hex(access)).bind(client()).bind(ISSUER).fetch_optional(&self.0).await.map_err(Into::into)
    }
}
impl crate::bff::OidcTransport for ProofOnly {
    fn send<'a>(&'a self, outgoing: crate::bff::OidcRequest) -> crate::bff::OidcFuture<'a> {
        Box::pin(async move {
            use crate::{bff::Endpoint, error::AppError};
            let invalid = || AppError::unauthorized("owned canonical fixture source is inactive");
            let body = match outgoing.endpoint {
                Endpoint::AccountProof => {
                    let authorization = format!(
                        "Basic {}",
                        STANDARD.encode(format!("{SOURCE_ID}:{SOURCE_SECRET}"))
                    );
                    if outgoing.authorization.as_deref() != Some(&authorization)
                        || !outgoing.form.is_empty()
                    {
                        return Err(invalid());
                    }
                    let request = outgoing
                        .json
                        .as_ref()
                        .filter(|value| value.as_object().is_some_and(|object| object.len() == 3))
                        .ok_or_else(invalid)?;
                    if request["contractVersion"] != "platform-account-proof-request/1"
                        || request["expectedClientId"] != client().to_string()
                    {
                        return Err(invalid());
                    }
                    let access = request["oidcAccessToken"].as_str().ok_or_else(invalid)?;
                    let source = self.source(access).await?.ok_or_else(invalid)?;
                    json!({"contractVersion":"platform-account-proof/1","issuer":ISSUER,"clientId":client(),"clientPurpose":"PLATFORM_CONSOLE","contextMode":"PLATFORM_CONTEXT","allowedMemberTypes":["EMPLOYEE"],"scope":["openid","profile"],"sub":subject(source.user_id),"userId":source.user_id,"globalSessionId":source.source_session_id,"grantId":source.grant_id,"accountVersion":source.account_version.to_string(),"clientPolicyVersion":source.client_policy_version.to_string(),"authTime":source.auth_time,"expiresAt":source.expires_at,"absoluteExpiresAt":source.absolute_expires_at})
                }
                Endpoint::UserInfo => {
                    if outgoing.json.is_some() || !outgoing.form.is_empty() {
                        return Err(invalid());
                    }
                    let access = outgoing
                        .authorization
                        .as_deref()
                        .and_then(|value| value.strip_prefix("Bearer "))
                        .ok_or_else(invalid)?;
                    let source = self.source(access).await?.ok_or_else(invalid)?;
                    json!({"sub":subject(source.user_id)})
                }
                _ => return Err(AppError::Unavailable),
            };
            Ok(crate::bff::OidcResponse {
                status: 200,
                body: serde_json::to_vec(&body).map_err(|_| AppError::Unavailable)?,
            })
        })
    }
}
