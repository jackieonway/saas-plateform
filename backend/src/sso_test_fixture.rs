//! Owned-database domain fixtures. Protocol acceptance is covered by bff_tests.
//! These rows model a completed SSO exchange without restoring retired HTTP APIs.
use anyhow::Result;
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
    let now = chrono::Utc::now().timestamp();
    crate::util::jwt::sign(
        &crate::util::jwt::Claims {
            sub: user.to_string(),
            typ: "session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            cid: None,
            scope: None,
            sid: Some(sid.to_string()),
            ver: Some(version),
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        secret,
    )
}

pub(crate) async fn session(pool: &PgPool, user: Uuid, tenant: Uuid, org: Uuid) -> Result<Uuid> {
    let issuer = ISSUER;
    let client = client();
    let global = uuid7::new_uuid();
    let grant = uuid7::new_uuid();
    let browser = uuid7::new_uuid();
    let bff = uuid7::new_uuid();
    let session = uuid7::new_uuid();
    let digest = hash::sha256_hex(&bff.to_string());
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO oidc_provider_settings(singleton,issuer,created_at) VALUES(true,$1,clock_timestamp()) ON CONFLICT(singleton) DO NOTHING")
        .bind(issuer).execute(&mut *tx).await?;
    let issuer: String =
        sqlx::query_scalar("SELECT issuer FROM oidc_provider_settings WHERE singleton")
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query("INSERT INTO oidc_clients(id,name,purpose,secret_hash,allowed_member_types,redirect_uris,status,access_policy_version,created_at,updated_at) VALUES($1,'Owned domain fixture RP','PLATFORM_CONSOLE',$2,ARRAY['EMPLOYEE'],ARRAY['https://console-fixture.example.invalid/auth/callback'],'ENABLED',1,clock_timestamp(),clock_timestamp()) ON CONFLICT(id) DO NOTHING")
        .bind(client).bind(hash::sha256_hex(CLIENT_SECRET)).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO sso_sessions(id,user_id,token_hash,csrf_hash,account_version,authenticated_at,expires_at) SELECT $1,id,$2,$2,session_version,clock_timestamp(),clock_timestamp()+interval '1 hour' FROM users WHERE id=$3")
        .bind(global).bind(&digest).bind(user).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO oidc_grants(id,client_id,client_policy_version,session_id,user_id,account_version,scope,nonce,auth_time,created_at,expires_at) SELECT $1,$2,1,$3,id,session_version,ARRAY['openid','profile'],'owned',date_trunc('second',clock_timestamp()),clock_timestamp(),clock_timestamp()+interval '1 hour' FROM users WHERE id=$4")
        .bind(grant).bind(client).bind(global).bind(user).execute(&mut *tx).await?;
    let access = hash::sha256_hex(&global.to_string());
    sqlx::query("INSERT INTO oidc_access_tokens(id,grant_id,token_hash,created_at,expires_at) VALUES($1,$2,$3,clock_timestamp(),clock_timestamp()+interval '1 hour')")
        .bind(uuid7::new_uuid()).bind(grant).bind(hash::sha256_hex(&access)).execute(&mut *tx).await?;
    let encrypted = crate::bff::CipherKeys::fixture().fixture_access(bff, &issuer, client, &access);
    sqlx::query("INSERT INTO console_browsers(id,token_hash,generation,current_session_id,created_at,expires_at) VALUES($1,$2,1,$3,clock_timestamp(),clock_timestamp()+interval '1 hour')")
        .bind(browser).bind(&digest).bind(bff).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO bff_sessions(id,browser_id,token_hash,csrf_hash,user_id,account_version,global_session_id,oidc_client_id,oidc_grant_id,oidc_client_policy_version,issuer,auth_time,nonce_hash,access_ciphertext,refresh_ciphertext,access_expires_at,version,selected_session_id,created_at,expires_at) SELECT $1,$2,$3,$3,user_id,account_version,$4,$5,id,1,$6,floor(extract(epoch FROM auth_time))::bigint,$3,$9,'owned-fixture-only',expires_at,1,$7,clock_timestamp(),expires_at FROM oidc_grants WHERE id=$8")
        .bind(bff).bind(browser).bind(&digest).bind(global).bind(client).bind(&issuer).bind(session).bind(grant).bind(encrypted).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version,bff_session_id,bff_context_version) SELECT $1,u.id,$2,$3,clock_timestamp()+interval '1 hour',u.session_version,m.id,m.member_type,m.identity_version,$4,$5,1,$6,1 FROM users u JOIN memberships m ON m.user_id=u.id AND m.organization_id=$3 AND m.member_type='EMPLOYEE' WHERE u.id=$7")
        .bind(session).bind(tenant).bind(org).bind(issuer).bind(client.to_string()).bind(bff).bind(user).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(session)
}

const ISSUER: &str = "https://sso-fixture.example.invalid";
const CLIENT_SECRET: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
fn client() -> Uuid {
    Uuid::parse_str("019d0000-0000-7000-8000-00000000d001").unwrap()
}

/// Real provider handlers with an in-process transport; no network dependency or
/// authentication bypass is introduced into the product state constructor.
pub(crate) fn state(
    pool: PgPool,
    config: std::sync::Arc<crate::config::Config>,
) -> crate::state::AppState {
    let mut config = (*config).clone();
    config.oidc_enabled = true;
    config.oidc_issuer = Some(ISSUER.into());
    config.bff_oidc_issuer = Some(ISSUER.into());
    config.bff_oidc_client_id = Some(client());
    config.bff_oidc_client_secret = Some(CLIENT_SECRET.into());
    config.bff_token_keys = Some(std::sync::Arc::new(crate::bff::CipherKeys::fixture()));
    let mut state = crate::state::AppState::new(pool, std::sync::Arc::new(config));
    let provider = crate::oidc::routes().with_state(state.clone());
    state.bff_transport = std::sync::Arc::new(Provider(provider));
    state
}
struct Provider(axum::Router);
impl crate::bff::OidcTransport for Provider {
    fn send<'a>(&'a self, outgoing: crate::bff::OidcRequest) -> crate::bff::OidcFuture<'a> {
        Box::pin(async move {
            use crate::error::AppError;
            use axum::{
                body::{to_bytes, Body},
                extract::ConnectInfo,
                http::{header, Method, Request},
            };
            use tower::ServiceExt;
            let form: String = url_form(&outgoing.form);
            let mut request = Request::builder()
                .method(if outgoing.endpoint.is_post() {
                    Method::POST
                } else {
                    Method::GET
                })
                .uri(outgoing.endpoint.path())
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .extension(ConnectInfo(std::net::SocketAddr::from((
                    [192, 0, 2, 240],
                    30038,
                ))));
            if let Some(value) = outgoing.authorization {
                request = request.header(header::AUTHORIZATION, value);
            }
            let response = self
                .0
                .clone()
                .oneshot(
                    request
                        .body(Body::from(form))
                        .map_err(|_| AppError::Unavailable)?,
                )
                .await
                .map_err(|_| AppError::Unavailable)?;
            let status = response.status().as_u16();
            let body = to_bytes(response.into_body(), 65536)
                .await
                .map_err(|_| AppError::Unavailable)?
                .to_vec();
            Ok(crate::bff::OidcResponse { status, body })
        })
    }
}
fn url_form(parameters: &[(String, String)]) -> String {
    let mut url = reqwest::Url::parse("https://fixture.invalid/").unwrap();
    url.query_pairs_mut()
        .extend_pairs(parameters.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    url.query().unwrap_or("").to_owned()
}
