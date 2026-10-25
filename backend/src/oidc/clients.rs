//! Explicit confidential USER registrations, separate from legacy OAuth clients.

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{Executor, Postgres, Transaction};
use uuid::Uuid;

use super::{invalid, OidcError, Result};
use crate::{
    error::AppError,
    extractors::{ApiJson, CurrentUser},
    state::AppState,
    util::audit,
};

#[derive(Clone, sqlx::FromRow)]
pub(super) struct Client {
    pub id: Uuid,
    pub name: String,
    pub purpose: String,
    pub secret_hash: String,
    pub allowed_member_types: Vec<String>,
    pub redirect_uris: Vec<String>,
    pub post_logout_redirect_uris: Vec<String>,
    pub refresh_enabled: bool,
    pub status: String,
    pub access_policy_version: i64,
}

pub(super) fn projection(client: &Client) -> Value {
    json!({"id":client.id,"clientId":client.id,"name":client.name,"purpose":client.purpose,
        "subjectMode":"USER","clientType":"CONFIDENTIAL","allowedMemberTypes":client.allowed_member_types,
        "redirectUris":client.redirect_uris,"postLogoutRedirectUris":client.post_logout_redirect_uris,
        "refreshEnabled":client.refresh_enabled,"status":client.status,"accessPolicyVersion":client.access_policy_version})
}

pub(super) async fn load<'e, E: Executor<'e, Database = Postgres>>(
    executor: E,
    id: Uuid,
    lock: bool,
) -> std::result::Result<Option<Client>, AppError> {
    let sql = if lock {
        "SELECT * FROM oidc_clients WHERE id=$1 FOR UPDATE"
    } else {
        "SELECT * FROM oidc_clients WHERE id=$1"
    };
    sqlx::query_as(sql)
        .bind(id)
        .fetch_optional(executor)
        .await
        .map_err(|_| AppError::Unavailable)
}

pub(super) fn canonical_id(raw: &str) -> Result<Uuid> {
    let id = Uuid::parse_str(raw).map_err(|_| invalid())?;
    if !crate::util::uuid7::is_uuid_v7(id) || id.to_string() != raw {
        return Err(invalid());
    }
    Ok(id)
}

pub(super) async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    parameters: &std::collections::BTreeMap<String, String>,
) -> Result<Client> {
    super::authority(state).await?;
    if parameters.contains_key("client_secret")
        || parameters.contains_key("client_assertion")
        || parameters.contains_key("client_assertion_type")
    {
        return Err(OidcError::InvalidClient);
    }
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let raw = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or(OidcError::InvalidClient)?;
    if values.next().is_some() {
        return Err(OidcError::InvalidClient);
    }
    let (scheme, encoded) = raw.split_once(' ').ok_or(OidcError::InvalidClient)?;
    if !scheme.eq_ignore_ascii_case("Basic") || encoded.len() > 2048 {
        return Err(OidcError::InvalidClient);
    }
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| OidcError::InvalidClient)?;
    let decoded = std::str::from_utf8(&decoded).map_err(|_| OidcError::InvalidClient)?;
    let (id, secret) = decoded.split_once(':').ok_or(OidcError::InvalidClient)?;
    let id = super::protocol::decode_component(id).map_err(|_| OidcError::InvalidClient)?;
    let secret = super::protocol::decode_component(secret).map_err(|_| OidcError::InvalidClient)?;
    let id = canonical_id(&id).map_err(|_| OidcError::InvalidClient)?;
    if let Some(body_id) = parameters.get("client_id") {
        if canonical_id(body_id).map_err(|_| OidcError::InvalidClient)? != id {
            return Err(OidcError::InvalidClient);
        }
    }
    let client = load(&state.pool, id, false)
        .await?
        .ok_or(OidcError::InvalidClient)?;
    if client.status != "ENABLED"
        || !super::canonical_secret(&secret)
        || !super::equal(&client.secret_hash, &super::digest(&secret))
    {
        return Err(OidcError::InvalidClient);
    }
    Ok(client)
}

pub(super) async fn require_admin(
    state: &AppState,
    user: &CurrentUser,
) -> std::result::Result<(), AppError> {
    super::authority(state).await?;
    state.require(user, "system:oauth").await
}

pub(super) async fn admin_tx<'a>(
    state: &AppState,
    user: &CurrentUser,
) -> std::result::Result<Transaction<'a, Postgres>, AppError> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    let platform:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenants t JOIN organizations o ON o.tenant_id=t.id WHERE t.id=$1 AND o.id=$2 AND t.is_platform AND o.parent_id IS NULL)")
        .bind(user.tenant_id).bind(user.org_id).fetch_one(&mut *tx).await.map_err(|_|AppError::Unavailable)?;
    let menus = crate::authorization::context_menu_rows_in_tx(&mut tx, user, None).await?;
    if !platform || !crate::authorization::permissions_of(&menus).contains("system:oauth") {
        return Err(AppError::forbidden("仅平台员工管理员可维护 OIDC 配置"));
    }
    Ok(tx)
}

pub(super) async fn admin_audit(
    tx: &mut Transaction<'_, Postgres>,
    user: &CurrentUser,
    headers: &HeaderMap,
    action: &str,
    detail: &str,
) -> std::result::Result<(), AppError> {
    audit::write_in_tx(
        tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action,
            detail,
            ip: &audit::client_ip(headers),
            user_agent: &audit::ua_of(headers),
        },
    )
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Registration {
    name: String,
    purpose: String,
    allowed_member_types: Vec<String>,
    redirect_uris: Vec<String>,
    #[serde(default)]
    post_logout_redirect_uris: Vec<String>,
    #[serde(default)]
    refresh_enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PolicyUpdate {
    name: String,
    purpose: String,
    allowed_member_types: Vec<String>,
    redirect_uris: Vec<String>,
    #[serde(default)]
    post_logout_redirect_uris: Vec<String>,
    #[serde(default)]
    refresh_enabled: bool,
    status: String,
    expected_access_policy_version: i64,
}

fn validate(body: &mut Registration) -> std::result::Result<(), AppError> {
    body.name = body.name.trim().to_owned();
    body.allowed_member_types.sort();
    body.allowed_member_types.dedup();
    if body.name.is_empty()
        || body.name.chars().count() > 120
        || body.name.chars().any(char::is_control)
        || !matches!(
            body.purpose.as_str(),
            "PLATFORM_CONSOLE" | "CUSTOMER_PORTAL" | "ACCOUNT"
        )
        || body.allowed_member_types.is_empty()
        || body.allowed_member_types.len() > 2
        || body
            .allowed_member_types
            .iter()
            .any(|value| !matches!(value.as_str(), "EMPLOYEE" | "CUSTOMER"))
        || (body.purpose == "PLATFORM_CONSOLE" && body.allowed_member_types != ["EMPLOYEE"])
        || (body.purpose == "CUSTOMER_PORTAL" && body.allowed_member_types != ["CUSTOMER"])
        || body.redirect_uris.is_empty()
        || body.redirect_uris.len() > 16
        || body.post_logout_redirect_uris.len() > 16
    {
        return Err(AppError::bad_request("OIDC 客户端用途或允许身份配置无效"));
    }
    for uri in body
        .redirect_uris
        .iter()
        .chain(body.post_logout_redirect_uris.iter())
    {
        validate_uri(uri)?;
    }
    body.redirect_uris.sort();
    body.redirect_uris.dedup();
    body.post_logout_redirect_uris.sort();
    body.post_logout_redirect_uris.dedup();
    Ok(())
}

fn validate_uri(raw: &str) -> std::result::Result<(), AppError> {
    let uri = reqwest::Url::parse(raw)
        .map_err(|_| AppError::bad_request("回调地址必须是精确 HTTPS URL"))?;
    if raw.len() > 2048
        || uri.scheme() != "https"
        || uri.host_str().is_none()
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.fragment().is_some()
        || uri.as_str() != raw
        || raw.contains('*')
        || uri
            .query_pairs()
            .any(|(key, _)| matches!(key.as_ref(), "code" | "state" | "error" | "iss"))
    {
        return Err(AppError::bad_request(
            "回调地址必须是无通配、凭据、fragment 或协议响应参数的规范 HTTPS URL",
        ));
    }
    Ok(())
}

pub(super) async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> std::result::Result<Json<Value>, AppError> {
    require_admin(&state, &user).await?;
    let clients: Vec<Client> =
        sqlx::query_as("SELECT * FROM oidc_clients ORDER BY created_at,id LIMIT 200")
            .fetch_all(&state.pool)
            .await
            .map_err(|_| AppError::Unavailable)?;
    Ok(Json(
        json!({"records":clients.iter().map(projection).collect::<Vec<_>>()}),
    ))
}

pub(super) async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    ApiJson(mut body): ApiJson<Registration>,
) -> std::result::Result<Json<Value>, AppError> {
    require_admin(&state, &user).await?;
    validate(&mut body)?;
    let id = crate::util::uuid7::new_uuid();
    let secret = super::secret()?;
    let mut tx = admin_tx(&state, &user).await?;
    sqlx::query("INSERT INTO oidc_clients(id,name,purpose,secret_hash,allowed_member_types,redirect_uris,post_logout_redirect_uris,refresh_enabled,status,access_policy_version,created_at,updated_at) \
        VALUES($1,$2,$3,$4,$5,$6,$7,$8,'ENABLED',1,clock_timestamp(),clock_timestamp())")
        .bind(id).bind(&body.name).bind(&body.purpose).bind(super::digest(&secret)).bind(&body.allowed_member_types)
        .bind(&body.redirect_uris).bind(&body.post_logout_redirect_uris).bind(body.refresh_enabled).execute(&mut *tx).await?;
    let client = load(&mut *tx, id, false)
        .await?
        .ok_or(AppError::Unavailable)?;
    admin_audit(
        &mut tx,
        &user,
        &headers,
        "OIDC_REGISTER_CLIENT",
        &projection(&client).to_string(),
    )
    .await?;
    tx.commit().await?;
    let mut result = projection(&client);
    result["clientSecret"] = json!(secret);
    Ok(Json(result))
}

pub(super) async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw): Path<String>,
    ApiJson(body): ApiJson<PolicyUpdate>,
) -> std::result::Result<Json<Value>, AppError> {
    require_admin(&state, &user).await?;
    let mut registration = Registration {
        name: body.name,
        purpose: body.purpose,
        allowed_member_types: body.allowed_member_types,
        redirect_uris: body.redirect_uris,
        post_logout_redirect_uris: body.post_logout_redirect_uris,
        refresh_enabled: body.refresh_enabled,
    };
    validate(&mut registration)?;
    let id =
        canonical_id(&raw).map_err(|_| AppError::bad_request("client ID 必须是规范 UUIDv7"))?;
    if !matches!(body.status.as_str(), "ENABLED" | "DISABLED") {
        return Err(AppError::bad_request("客户端状态无效"));
    }
    let mut tx = admin_tx(&state, &user).await?;
    let before = load(&mut *tx, id, true)
        .await?
        .ok_or_else(|| AppError::not_found("OIDC 客户端不存在"))?;
    if before.access_policy_version != body.expected_access_policy_version {
        return Err(AppError::conflict("客户端策略版本已变化"));
    }
    let b = registration;
    sqlx::query("UPDATE oidc_clients SET name=$2,purpose=$3,allowed_member_types=$4,redirect_uris=$5,post_logout_redirect_uris=$6,refresh_enabled=$7,status=$8,access_policy_version=access_policy_version+1,updated_at=clock_timestamp() WHERE id=$1")
        .bind(id).bind(b.name).bind(b.purpose).bind(b.allowed_member_types).bind(b.redirect_uris)
        .bind(b.post_logout_redirect_uris).bind(b.refresh_enabled).bind(body.status).execute(&mut *tx).await?;
    let after = load(&mut *tx, id, false)
        .await?
        .ok_or(AppError::Unavailable)?;
    admin_audit(
        &mut tx,
        &user,
        &headers,
        "OIDC_UPDATE_CLIENT_POLICY",
        &json!({"before":projection(&before),"after":projection(&after)}).to_string(),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(projection(&after)))
}
