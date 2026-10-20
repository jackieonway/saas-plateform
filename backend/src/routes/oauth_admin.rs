use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser};
use crate::state::AppState;
use crate::session;
use crate::util::{audit, hash};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/oauth/clients", get(list_clients).post(create_client))
        .route("/api/oauth/clients/{id}", put(update_client).delete(delete_client))
        .route("/api/oauth/clients/{id}/regenerate-secret", post(regenerate_secret))
        .route("/api/oauth/tokens", get(list_tokens))
        .route("/api/oauth/tokens/purge", post(purge_tokens))
        .route("/api/oauth/tokens/kick", post(kick_sessions))
        .route("/api/oauth/tokens/sessions", get(list_sessions))
        .route("/api/oauth/tokens/{id}", delete(revoke_token))
        .route("/api/oauth/device-codes", get(list_device_codes))
        .route("/api/oauth/device-codes/{id}/revoke", put(revoke_device_code))
}

const CLIENT_MENU: &str = "/system/oauth/clients";
const TOKEN_MENU: &str = "/system/oauth/tokens";

async fn require_oauth_menu(
    state: &AppState,
    user: &CurrentUser,
    path: &'static str,
) -> Result<(), AppError> {
    state.require(user, "system:oauth").await?;
    state.require_menu_path(user, path).await
}

/// OAuth 客户端完整行 (含 secret 哈希, 仅供内部使用)
#[derive(sqlx::FromRow, Clone)]
pub struct ClientFull {
    pub id: Uuid,
    pub client_id: String,
    pub client_secret_hash: Option<String>,
    pub client_name: String,
    pub description: String,
    pub client_type: String,
    pub redirect_uris: String,
    pub grant_types: String,
    pub scopes: String,
    pub access_token_validity: i64,
    pub refresh_token_validity: i64,
    pub status: String,
    /// 关联应用 (对外 API 数据源; NULL = 未关联, 迁移 0010)
    pub application_id: Option<Uuid>,
}

impl ClientFull {
    pub fn grant_list(&self) -> Vec<String> {
        split_csv(&self.grant_types)
    }
    pub fn scope_list(&self) -> Vec<String> {
        split_csv(&self.scopes)
    }
    pub fn has_grant(&self, g: &str) -> bool {
        self.grant_list().iter().any(|x| x == g)
    }
    pub fn to_public_json(&self) -> Value {
        json!({
            "id": self.id, "clientId": self.client_id, "clientName": self.client_name,
            "clientType": self.client_type, "description": self.description,
            "redirectUris": split_csv(&self.redirect_uris), "grantTypes": self.grant_list(),
            "scopes": self.scope_list(), "accessTokenValidity": self.access_token_validity,
            "refreshTokenValidity": self.refresh_token_validity, "status": self.status,
            "applicationId": self.application_id,
        })
    }
}

pub fn split_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

pub async fn fetch_client(pool: &sqlx::PgPool, client_id: &str) -> Result<Option<ClientFull>, AppError> {
    let row = sqlx::query_as::<_, ClientFull>(
        "SELECT id, client_id, client_secret_hash, client_name, COALESCE(description,'') AS description, \
         client_type, COALESCE(redirect_uris,'') AS redirect_uris, COALESCE(grant_types,'') AS grant_types, \
         COALESCE(scopes,'') AS scopes, access_token_validity, refresh_token_validity, status, application_id \
         FROM oauth_clients WHERE client_id = $1",
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Safe metadata only: credential values and redirect bodies never enter audit.
async fn client_audit_snapshot(
    tx: &mut Transaction<'_, Postgres>, id: Uuid,
) -> Result<Value, AppError> {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('id',id,'name',client_name,'clientId',client_id, \
         'clientType',client_type,'status',status,'applicationId',application_id, \
         'grantTypes',grant_types,'scopeCount',COALESCE(array_length(string_to_array(scopes,','),1),0), \
         'redirectUriCount',COALESCE(array_length(string_to_array(redirect_uris,','),1),0), \
         'accessTokenValidity',access_token_validity,'refreshTokenValidity',refresh_token_validity) \
         FROM oauth_clients WHERE id=$1 FOR UPDATE",
    ).bind(id).fetch_optional(&mut **tx).await?
        .ok_or_else(|| AppError::not_found("客户端不存在"))
}

/// App deletion locks app -> client through ON DELETE SET NULL. Take the same
/// order before a client update can need an application's foreign-key lock.
async fn lock_client_applications(
    tx: &mut Transaction<'_, Postgres>, id: Uuid, requested: Option<Option<Uuid>>,
) -> Result<(), AppError> {
    let observed: Option<Uuid> = sqlx::query_scalar("SELECT application_id FROM oauth_clients WHERE id=$1")
        .bind(id).fetch_optional(&mut **tx).await?
        .ok_or_else(|| AppError::not_found("客户端不存在"))?;
    let target = requested.unwrap_or(observed);
    let mut applications: Vec<Uuid> = observed.into_iter().chain(target).collect();
    applications.sort_unstable();
    applications.dedup();
    let locked: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM applications WHERE id=ANY($1) ORDER BY id FOR KEY SHARE",
    ).bind(&applications).fetch_all(&mut **tx).await?;
    if target.is_some_and(|app| !locked.contains(&app)) {
        return Err(AppError::not_found("关联应用不存在"));
    }
    let current: Option<Uuid> = sqlx::query_scalar("SELECT application_id FROM oauth_clients WHERE id=$1 FOR UPDATE")
        .bind(id).fetch_optional(&mut **tx).await?
        .ok_or_else(|| AppError::not_found("客户端不存在"))?;
    if current != observed {
        return Err(AppError::conflict("客户端关联应用已变化，请重新请求"));
    }
    Ok(())
}

/// GET /api/oauth/clients — 不返回 secret
async fn list_clients(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, CLIENT_MENU).await?;
    // (id, created_at, application_id, application_name): LEFT JOIN applications 取关联应用名
    let times: Vec<(Uuid, DateTime<Utc>, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT c.id, c.created_at, c.application_id, a.name \
         FROM oauth_clients c LEFT JOIN applications a ON a.id = c.application_id \
         ORDER BY c.created_at",
    )
    .fetch_all(&state.pool)
    .await?;
    let rows: Vec<ClientFull> = sqlx::query_as(
        "SELECT id, client_id, client_secret_hash, client_name, COALESCE(description,'') AS description, \
         client_type, COALESCE(redirect_uris,'') AS redirect_uris, COALESCE(grant_types,'') AS grant_types, \
         COALESCE(scopes,'') AS scopes, access_token_validity, refresh_token_validity, status, application_id \
         FROM oauth_clients ORDER BY created_at",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut time_map: HashMap<Uuid, (DateTime<Utc>, Option<Uuid>, Option<String>)> = HashMap::new();
    for (id, t, app_id, app_name) in times {
        time_map.insert(id, (t, app_id, app_name));
    }
    let items: Vec<Value> = rows
        .iter()
        .map(|c| {
            let mut v = c.to_public_json();
            let (created, app_id, app_name) = time_map
                .get(&c.id)
                .cloned()
                .unwrap_or((Utc::now(), c.application_id, None));
            v["createdAt"] = json!(created);
            v["applicationId"] = json!(app_id);
            v["applicationName"] = json!(app_name);
            v
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientBody {
    client_name: String,
    #[serde(default = "default_client_type")]
    client_type: String,
    #[serde(default)]
    redirect_uris: Vec<String>,
    #[serde(default)]
    grant_types: Vec<String>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default = "default_access_validity")]
    access_token_validity: i64,
    #[serde(default = "default_refresh_validity")]
    refresh_token_validity: i64,
    #[serde(default)]
    description: String,
}

fn default_client_type() -> String {
    "CONFIDENTIAL".into()
}

fn default_access_validity() -> i64 {
    7200
}

fn default_refresh_validity() -> i64 {
    604800
}

fn validate_client_body(body: &ClientBody) -> Result<(), AppError> {
    if body.client_name.trim().is_empty() {
        return Err(AppError::bad_request("客户端名称不能为空"));
    }
    if !["CONFIDENTIAL", "PUBLIC"].contains(&body.client_type.as_str()) {
        return Err(AppError::bad_request("客户端类型仅支持 CONFIDENTIAL / PUBLIC"));
    }
    if body.grant_types.is_empty() {
        return Err(AppError::bad_request("至少配置一种授权模式"));
    }
    if body.scopes.is_empty() {
        return Err(AppError::bad_request("至少配置一个 scope"));
    }
    if body.access_token_validity <= 0 || body.refresh_token_validity <= 0 {
        return Err(AppError::bad_request("令牌有效期必须为正整数 (秒)"));
    }
    Ok(())
}

/// 生成可读 client_id: 名称 slug + 随机后缀
async fn generate_client_id(pool: &sqlx::PgPool, name: &str) -> Result<String, AppError> {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    let base = if slug.is_empty() { "client".to_string() } else { slug };
    for _ in 0..8 {
        let suffix = hash::random_token(6).to_lowercase();
        let candidate = format!("{}-{}", base, suffix);
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM oauth_clients WHERE client_id = $1)")
                .bind(&candidate)
                .fetch_one(pool)
                .await?;
        if !exists {
            return Ok(candidate);
        }
    }
    Ok(format!("client-{}", hash::random_token(12).to_lowercase()))
}

/// POST /api/oauth/clients — 响应含一次性明文 clientSecret
async fn create_client(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<ClientBody>,
) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, CLIENT_MENU).await?;
    validate_client_body(&body)?;
    let client_id = generate_client_id(&state.pool, &body.client_name).await?;
    let secret = hash::random_token(32);
    let secret_hash = if body.client_type == "CONFIDENTIAL" {
        Some(hash::sha256_hex(&secret))
    } else {
        None
    };
    let mut tx = state.pool.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO oauth_clients \
         (client_id, client_secret_hash, client_name, description, client_type, redirect_uris, grant_types, scopes, access_token_validity, refresh_token_validity) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(&client_id)
    .bind(&secret_hash)
    .bind(body.client_name.trim())
    .bind(&body.description)
    .bind(&body.client_type)
    .bind(body.redirect_uris.join(","))
    .bind(body.grant_types.join(","))
    .bind(body.scopes.join(","))
    .bind(body.access_token_validity)
    .bind(body.refresh_token_validity)
    .fetch_one(&mut *tx)
    .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("oauth_client", id, Value::Null, client_audit_snapshot(&mut tx, id).await?);
    audit::write_in_tx(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "CREATE_CLIENT", detail: &detail, ip: &ip, user_agent: &ua,
    }).await?;
    tx.commit().await?;
    Ok(Json(json!({
        "id": id, "clientId": client_id, "clientName": body.client_name,
        "clientType": body.client_type, "description": body.description,
        "redirectUris": body.redirect_uris, "grantTypes": body.grant_types, "scopes": body.scopes,
        "accessTokenValidity": body.access_token_validity, "refreshTokenValidity": body.refresh_token_validity,
        "status": "ENABLED", "clientSecret": secret, "applicationId": null,
    })))
}

/// 三态可空字段反序列化: 键缺省 → None(不更新); 显式 null → Some(None)(清除); 有值 → Some(Some(v))
/// (与 tenants/organizations 的 expiresAt 同款, Task 18 经验)
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientUpdate {
    #[serde(default)]
    client_name: Option<String>,
    #[serde(default)]
    client_type: Option<String>,
    #[serde(default)]
    redirect_uris: Option<Vec<String>>,
    #[serde(default)]
    grant_types: Option<Vec<String>>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
    #[serde(default)]
    access_token_validity: Option<i64>,
    #[serde(default)]
    refresh_token_validity: Option<i64>,
    #[serde(default)]
    description: Option<String>,
    /// 关联应用 (对外 API 数据源): 键缺省不更新; 显式 null / 空串清除; uuid 值校验后关联
    #[serde(default, deserialize_with = "double_option")]
    application_id: Option<Option<String>>,
}

/// PUT /api/oauth/clients/{id}
async fn update_client(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ClientUpdate>,
) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, CLIENT_MENU).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    if let Some(t) = &body.client_type {
        if !["CONFIDENTIAL", "PUBLIC"].contains(&t.as_str()) {
            return Err(AppError::bad_request("客户端类型仅支持 CONFIDENTIAL / PUBLIC"));
        }
    }
    if let Some(name) = &body.client_name {
        if name.trim().is_empty() {
            return Err(AppError::bad_request("客户端名称不能为空"));
        }
    }
    // 关联应用三态归一: None(键缺省)=不更新; Some(None)=清除(null/空串); Some(Some)=校验存在后关联
    let application_id: Option<Option<Uuid>> = match body.application_id {
        None => None,
        Some(None) => Some(None),
        Some(Some(ref s)) => {
            let raw = s.trim();
            if raw.is_empty() {
                Some(None)
            } else {
                let app: Uuid =
                    raw.parse().map_err(|_| AppError::bad_request("applicationId 格式错误"))?;
                let exists: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM applications WHERE id = $1)")
                        .bind(app)
                        .fetch_one(&state.pool)
                        .await?;
                if !exists {
                    return Err(AppError::not_found("关联应用不存在"));
                }
                Some(Some(app))
            }
        }
    };
    let mut tx = state.pool.begin().await?;
    lock_client_applications(&mut tx, id, application_id).await?;
    let before = client_audit_snapshot(&mut tx, id).await?;
    let private_changes: (bool, bool, bool) = sqlx::query_as(
        "SELECT redirect_uris IS DISTINCT FROM COALESCE($2,redirect_uris), \
         scopes IS DISTINCT FROM COALESCE($3,scopes), \
         description IS DISTINCT FROM COALESCE($4,description) FROM oauth_clients WHERE id=$1",
    ).bind(id).bind(body.redirect_uris.as_ref().map(|values| values.join(",")))
        .bind(body.scopes.as_ref().map(|values| values.join(",")))
        .bind(&body.description).fetch_one(&mut *tx).await?;
    let row = sqlx::query_as::<_, ClientFull>(
        "UPDATE oauth_clients SET client_name = COALESCE($2, client_name), client_type = COALESCE($3, client_type), \
         redirect_uris = COALESCE($4, redirect_uris), grant_types = COALESCE($5, grant_types), \
         scopes = COALESCE($6, scopes), access_token_validity = COALESCE($7, access_token_validity), \
         refresh_token_validity = COALESCE($8, refresh_token_validity), description = COALESCE($9, description), \
         application_id = CASE WHEN $10 THEN $11::uuid ELSE application_id END, \
         updated_at = now() WHERE id = $1 \
         RETURNING id, client_id, client_secret_hash, client_name, COALESCE(description,'') AS description, \
         client_type, COALESCE(redirect_uris,'') AS redirect_uris, COALESCE(grant_types,'') AS grant_types, \
         COALESCE(scopes,'') AS scopes, access_token_validity, refresh_token_validity, status, application_id",
    )
    .bind(id)
    .bind(body.client_name.as_deref().map(str::trim))
    .bind(&body.client_type)
    .bind(body.redirect_uris.as_ref().map(|v| v.join(",")))
    .bind(body.grant_types.as_ref().map(|v| v.join(",")))
    .bind(body.scopes.as_ref().map(|v| v.join(",")))
    .bind(body.access_token_validity)
    .bind(body.refresh_token_validity)
    .bind(&body.description)
    .bind(application_id.is_some())
    .bind(application_id.flatten())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("客户端不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let mut after = client_audit_snapshot(&mut tx, id).await?;
    for (field, changed) in [("redirectUrisChanged",private_changes.0),("scopesChanged",private_changes.1),("descriptionChanged",private_changes.2)] {
        if changed { after[field] = json!(true); }
    }
    if before != after {
        let detail = audit::change_detail("oauth_client", id, before, after);
        audit::write_in_tx(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "UPDATE_CLIENT", detail: &detail, ip: &ip, user_agent: &ua,
        }).await?;
    }
    let mut v = row.to_public_json();
    // update 响应补关联应用名 (UPDATE ... RETURNING 无法 JOIN 目标表, 单查一次)
    let app_name: Option<String> = match row.application_id {
        Some(app) => {
            sqlx::query_scalar("SELECT name FROM applications WHERE id = $1")
                .bind(app)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => None,
    };
    v["applicationName"] = json!(app_name);
    tx.commit().await?;
    Ok(Json(v))
}

/// DELETE /api/oauth/clients/{id}
async fn delete_client(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, CLIENT_MENU).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let mut tx = state.pool.begin().await?;
    let before = client_audit_snapshot(&mut tx, id).await?;
    let (_client_id,): (String,) =
        sqlx::query_as("DELETE FROM oauth_clients WHERE id = $1 RETURNING client_id")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| AppError::not_found("客户端不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("oauth_client", id, before, Value::Null);
    audit::write_in_tx(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "DELETE_CLIENT", detail: &detail, ip: &ip, user_agent: &ua,
    }).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

/// POST /api/oauth/clients/{id}/regenerate-secret — 一次性明文
async fn regenerate_secret(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, CLIENT_MENU).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let secret = hash::random_token(32);
    let secret_hash = hash::sha256_hex(&secret);
    let mut tx = state.pool.begin().await?;
    let before = client_audit_snapshot(&mut tx, id).await?;
    let (_client_id,): (String,) = sqlx::query_as(
        "UPDATE oauth_clients SET client_secret_hash = $2, updated_at = now() \
         WHERE id = $1 AND client_type = 'CONFIDENTIAL' RETURNING client_id",
    )
    .bind(id)
    .bind(&secret_hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("客户端不存在或为 PUBLIC 类型无需密钥"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let mut after = client_audit_snapshot(&mut tx, id).await?;
    after["credentialRotated"] = json!(true);
    let detail = audit::change_detail("oauth_client", id, before, after);
    audit::write_in_tx(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "UPDATE_CLIENT", detail: &detail, ip: &ip, user_agent: &ua,
    }).await?;
    tx.commit().await?;
    Ok(Json(json!({"clientSecret": secret})))
}

type TokenRowTuple = (Uuid, String, String, String, String, DateTime<Utc>, DateTime<Utc>, bool);

/// 清理过期的 OAuth 历史数据 (手动 purge 端点与每小时定时任务共用)
///
/// 语义 (统一以 expires_at 为准, 7 天保底保留窗口便于事后排查):
/// - oauth_tokens: expires_at < now() - interval '7 days' —— 已过期超 7 天的令牌才删;
///   吊销但未到期的行保留 (令牌管理页可见), 吊销行同样等到期后满 7 天再删
/// - oauth_device_codes: status = 'DENIED' (已拒绝的终态行无保留价值, 立即删)
///   或 expires_at < now() - interval '7 days' (PENDING/APPROVED/EXPIRED 过期满 7 天统一清理)
pub(crate) async fn purge_stale_oauth_data(pool: &sqlx::PgPool) -> Result<(u64, u64), AppError> {
    let mut tx = pool.begin().await?;
    let counts = purge_stale_oauth_data_in_tx(&mut tx).await?;
    tx.commit().await?;
    Ok(counts)
}

async fn purge_stale_oauth_data_in_tx(tx: &mut Transaction<'_, Postgres>) -> Result<(u64, u64), AppError> {
    let tokens = sqlx::query("DELETE FROM oauth_tokens WHERE expires_at < now() - interval '7 days'")
        .execute(&mut **tx)
        .await?
        .rows_affected();
    let devices = sqlx::query(
        "DELETE FROM oauth_device_codes WHERE status = 'DENIED' OR expires_at < now() - interval '7 days'",
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok((tokens, devices))
}

/// POST /api/oauth/tokens/purge — 手动触发历史数据清理 (与定时任务同一语义)
async fn purge_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, TOKEN_MENU).await?;
    let mut tx = state.pool.begin().await?;
    let (tokens_deleted, devices_deleted) = purge_stale_oauth_data_in_tx(&mut tx).await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    if tokens_deleted != 0 || devices_deleted != 0 {
        let detail = audit::change_detail("oauth_history", Uuid::nil(),
        json!({"eligibleTokenCount":tokens_deleted,"eligibleDeviceCount":devices_deleted}),
        json!({"eligibleTokenCount":0,"eligibleDeviceCount":0}));
        audit::write_in_tx(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "OAUTH_PURGE", detail: &detail, ip: &ip, user_agent: &ua,
        }).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({ "tokensDeleted": tokens_deleted, "devicesDeleted": devices_deleted })))
}

/// GET /api/oauth/tokens?resourceOwner=
async fn list_tokens(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, TOKEN_MENU).await?;
    let owner = params.get("resourceOwner").cloned().unwrap_or_default();
    let rows: Vec<TokenRowTuple> = sqlx::query_as(
        "SELECT t.id, t.token_type, t.client_id, COALESCE(u.username, '') AS username, \
         COALESCE(t.scope, '') AS scope, t.created_at, t.expires_at, t.revoked \
         FROM oauth_tokens t LEFT JOIN users u ON u.id = t.user_id \
         WHERE ($1 = '' OR u.username ILIKE '%' || $1 || '%') \
         ORDER BY t.created_at DESC LIMIT 500",
    )
    .bind(&owner)
    .fetch_all(&state.pool)
    .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(id, token_type, client_id, username, scope, created_at, expires_at, revoked)| {
            json!({
                "id": id, "tokenType": token_type, "clientId": client_id, "username": username,
                "scope": scope, "createdAt": created_at, "expiresAt": expires_at, "revoked": revoked,
            })
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

type SessionRowTuple = (
    Uuid,
    String,
    String,
    Vec<String>,
    i64,
    i64,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    i64,
);

/// GET /api/oauth/tokens/sessions — OAuth 用户聚合与平台登录设备会话。
///
/// INNER JOIN users 自然排除 client_credentials (user_id IS NULL) 的令牌行;
/// clients 为全历史客户端 (含已吊销); activeAccess/activeRefresh 仅计
/// revoked=false AND expires_at > now() 的行; lastExpiresAt 无活跃令牌时为 null。
/// 静态段 sessions 与参数段 {id} 同层, matchit 静态优先, 路由共存不冲突。
async fn list_sessions(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, TOKEN_MENU).await?;
    let rows: Vec<SessionRowTuple> = sqlx::query_as(
        "SELECT t.user_id, u.username, COALESCE(u.real_name, '') AS real_name, \
         array_agg(DISTINCT t.client_id) AS clients, \
         COUNT(*) FILTER (WHERE t.revoked = false AND t.expires_at > now() AND t.token_type = 'access') AS active_access, \
         COUNT(*) FILTER (WHERE t.revoked = false AND t.expires_at > now() AND t.token_type = 'refresh') AS active_refresh, \
         max(t.created_at) AS last_issued_at, \
         max(t.expires_at) FILTER (WHERE t.revoked = false AND t.expires_at > now()) AS last_expires_at, \
         COUNT(*) FILTER (WHERE t.revoked = true) AS revoked_count \
         FROM oauth_tokens t JOIN users u ON u.id = t.user_id \
         GROUP BY t.user_id, u.username, u.real_name \
         ORDER BY last_issued_at DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await?;
    let sessions: Vec<Value> = rows
        .into_iter()
        .map(
            |(user_id, username, real_name, clients, active_access, active_refresh, last_issued_at, last_expires_at, revoked_count)| {
                json!({
                    "userId": user_id, "username": username, "realName": real_name,
                    "clients": clients, "activeAccess": active_access, "activeRefresh": active_refresh,
                    "lastIssuedAt": last_issued_at, "lastExpiresAt": last_expires_at,
                    "revokedCount": revoked_count,
                })
            },
        )
        .collect();
    let platform_sessions = session::device_sessions(
        &state.pool, None, user.user_id, user.session_id,
    ).await?;
    Ok(Json(json!({ "sessions": sessions, "platformSessions": platform_sessions })))
}

/// DELETE /api/oauth/tokens/{id} — 吊销
async fn revoke_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, TOKEN_MENU).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let mut tx = state.pool.begin().await?;
    let before: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('id',id,'name',client_id,'clientId',client_id, \
         'tokenType',token_type,'revoked',revoked) FROM oauth_tokens WHERE id=$1 FOR UPDATE",
    ).bind(id).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::not_found("令牌不存在"))?;
    let (_client_id,): (String,) = sqlx::query_as(
        "UPDATE oauth_tokens SET revoked = true WHERE id = $1 RETURNING client_id",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("令牌不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let mut after = before.clone();
    after["revoked"] = json!(true);
    if before != after {
        let detail = audit::change_detail("oauth_credential", id, before, after);
        audit::write_in_tx(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "TOKEN_REVOKED", detail: &detail, ip: &ip, user_agent: &ua,
        }).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KickBody {
    user_id: String,
}

/// POST /api/oauth/tokens/kick — 会话踢出: 吊销指定用户全部未吊销令牌
///
/// oauth_tokens 表的 access / refresh 行均以 user_id 关联用户 (issue_tokens 签发的所有
/// access 令牌均入库, userinfo 凭 token_hash 查库校验 revoked, 故吊销后立即生效);
/// 同一 UPDATE 同时覆盖 refresh 行, 被踢用户重新走授权流程仍可正常获取新令牌。
async fn kick_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<KickBody>,
) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, TOKEN_MENU).await?;
    state.require_platform(&user).await?;
    let target: Uuid = body
        .user_id
        .parse()
        .map_err(|_| AppError::bad_request("userId 格式错误"))?;
    // Serialize ordinary refresh with user-first revocation; no gap between
    // retiring platform sessions and the OAuth credentials they can renew.
    let mut tx = state.pool.begin().await?;
    let before = session_audit_snapshot(&mut tx, target).await?;
    let platform_revoked = session::revoke_all_in_tx(&mut tx, target, None).await?;
    let revoked = sqlx::query(
        "UPDATE oauth_tokens SET revoked = true WHERE user_id = $1 AND revoked = false",
    )
    .bind(target)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("user_sessions", target, before, session_audit_snapshot(&mut tx, target).await?);
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "KICK_SESSIONS",
            detail: &detail,
            ip: &ip, user_agent: &ua,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "revoked": revoked, "platformSessionsRevoked": platform_revoked })))
}

async fn session_audit_snapshot(tx: &mut Transaction<'_, Postgres>, target: Uuid) -> Result<Value, AppError> {
    // Counts need a fresh READ COMMITTED statement after waiting for issuance's
    // user lock, otherwise a just-committed rotation can be absent from before.
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(target).fetch_optional(&mut **tx).await?
        .ok_or_else(|| AppError::not_found("目标用户不存在"))?;
    sqlx::query_scalar(
        "SELECT jsonb_build_object('id',u.id,'name',u.username,'sessionVersion',u.session_version, \
         'unrevokedSessionCount',(SELECT count(*) FROM platform_sessions s WHERE s.user_id=u.id AND s.revoked_at IS NULL), \
         'unrevokedCredentialCount',(SELECT count(*) FROM oauth_tokens t WHERE t.user_id=u.id AND t.revoked=false)) \
         FROM users u WHERE u.id=$1",
    ).bind(target).fetch_optional(&mut **tx).await?
        .ok_or_else(|| AppError::not_found("目标用户不存在"))
}

type DeviceRowTuple = (Uuid, String, String, String, String, String, String, DateTime<Utc>, DateTime<Utc>);

/// GET /api/oauth/device-codes
async fn list_device_codes(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, TOKEN_MENU).await?;
    let rows: Vec<DeviceRowTuple> = sqlx::query_as(
        "SELECT d.id, d.user_code, d.client_id, COALESCE(c.client_name, '') AS client_name, \
         COALESCE(d.scope, '') AS scope, \
         CASE WHEN d.status = 'PENDING' AND d.expires_at < now() THEN 'EXPIRED' ELSE d.status END AS status, \
         COALESCE(u.username, '') AS username, d.created_at, d.expires_at \
         FROM oauth_device_codes d \
         LEFT JOIN oauth_clients c ON c.client_id = d.client_id \
         LEFT JOIN users u ON u.id = d.user_id \
         ORDER BY d.created_at DESC LIMIT 500",
    )
    .fetch_all(&state.pool)
    .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(id, user_code, client_id, client_name, scope, status, username, created_at, expires_at)| {
            json!({
                "id": id, "userCode": user_code, "clientId": client_id, "clientName": client_name,
                "scope": scope, "status": status, "username": username,
                "createdAt": created_at, "expiresAt": expires_at,
            })
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

/// PUT /api/oauth/device-codes/{id}/revoke
async fn revoke_device_code(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    require_oauth_menu(&state, &user, TOKEN_MENU).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let mut tx = state.pool.begin().await?;
    let before: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('id',id,'name',client_id,'clientId',client_id,'status',status) \
         FROM oauth_device_codes WHERE id=$1 FOR UPDATE",
    ).bind(id).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::not_found("设备码不存在"))?;
    let (_id,): (Uuid,) = sqlx::query_as(
        "UPDATE oauth_device_codes SET status = 'EXPIRED' WHERE id = $1 RETURNING id",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("设备码不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let mut after = before.clone();
    after["status"] = json!("EXPIRED");
    if before != after {
        let detail = audit::change_detail("oauth_device_authorization", id, before, after);
        audit::write_in_tx(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "DEVICE_DENY", detail: &detail, ip: &ip, user_agent: &ua,
        }).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}
