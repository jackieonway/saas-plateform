//! Publication and delivery persistence. All persistent new notifications have an explicit owner.
use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::util::{audit, hash};

pub const DELIVERY_LIMIT: usize = 1000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Target {
    #[serde(rename_all = "camelCase")]
    User {
        #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
        user_id: Uuid,
        #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
        organization_id: Uuid,
    },
    #[serde(rename_all = "camelCase")]
    Organization {
        #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
        organization_id: Uuid,
    },
    Tenant {},
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishBody {
    pub idempotency_key: String,
    pub target: Target,
    pub r#type: String,
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub link: String,
}

pub struct Publisher {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub organization_id: Uuid,
    pub application_id: Uuid,
    pub session_id: Uuid,
    pub token_hash: Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct MemberProof {
    membership_id: Uuid,
    member_type: String,
    identity_version: i64,
}

#[derive(sqlx::FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Recipient {
    user_id: Uuid,
    organization_id: Uuid,
    membership_id: Uuid,
    member_type: String,
    identity_version: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Publication {
    pub publication_id: Uuid,
    pub recipient_count: i32,
    pub delivery_count: i32,
    pub duplicate: bool,
    pub created_at: DateTime<Utc>,
}

pub fn valid_type(value: &str) -> bool {
    matches!(value, "INFO" | "SUCCESS" | "WARNING" | "SECURITY")
}

fn bad_link() -> AppError {
    AppError::bad_request("link 必须位于当前应用 /micro/{appCode} 内")
}

fn unsafe_url(value: &str) -> bool {
    value.contains('\\')
        || value.chars().any(char::is_control)
        || !value.starts_with('/')
        || value.starts_with("//")
}

fn inside_app(value: &str, prefix: &str) -> Result<reqwest::Url, AppError> {
    if unsafe_url(value) {
        return Err(bad_link());
    }
    let url = reqwest::Url::parse(&format!("https://notification.invalid{value}"))
        .map_err(|_| bad_link())?;
    if url.host_str() != Some("notification.invalid")
        || !(url.path() == prefix || url.path().starts_with(&format!("{prefix}/")))
    {
        return Err(bad_link());
    }
    Ok(url)
}

// URL serialization encodes Unicode into ASCII triplets. Keep actual Unicode
// characters for the VARCHAR/character-count contract; reserved ASCII escapes
// remain encoded so they cannot turn into URL delimiters.
fn restore_url_unicode(value: &str) -> String {
    fn encoded_byte(bytes: &[u8], i: usize) -> Option<u8> {
        if bytes.get(i) != Some(&b'%') {
            return None;
        }
        let high = (*bytes.get(i + 1)? as char).to_digit(16)?;
        let low = (*bytes.get(i + 2)? as char).to_digit(16)?;
        Some((high * 16 + low) as u8)
    }
    let bytes = value.as_bytes();
    let mut result = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if encoded_byte(bytes, i).is_some_and(|byte| byte >= 128) {
            let start = i;
            let mut utf8 = Vec::new();
            while let Some(byte) = encoded_byte(bytes, i).filter(|byte| *byte >= 128) {
                utf8.push(byte);
                i += 3;
            }
            match std::str::from_utf8(&utf8) {
                Ok(decoded) => result.push_str(decoded),
                Err(_) => result.push_str(&value[start..i]),
            }
        } else {
            let ch = value[i..].chars().next().expect("valid string boundary");
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    result
}

/// Check both URL normalization and every percent-decoding layer; encoded separators,
/// controls and dot-segment escapes cannot acquire a different application owner.
pub fn normalize_link(link: &str, app_code: &str) -> Result<String, AppError> {
    if link.is_empty() {
        return Ok(String::new());
    }
    let prefix = format!("/micro/{app_code}");
    let url = inside_app(link, &prefix)?;
    let mut decoded = link.to_owned();
    for _ in 0..8 {
        let next = percent_encoding::percent_decode_str(&decoded)
            .decode_utf8()
            .map_err(|_| bad_link())?
            .into_owned();
        if next == decoded {
            let mut normalized = url.path().to_owned();
            if let Some(query) = url.query() {
                normalized.push('?');
                normalized.push_str(query);
            }
            if let Some(fragment) = url.fragment() {
                normalized.push('#');
                normalized.push_str(fragment);
            }
            return Ok(restore_url_unicode(&normalized));
        }
        let path = next.split(['?', '#']).next().unwrap_or("");
        if path
            .split('/')
            .any(|segment| segment == "." || segment == "..")
        {
            return Err(bad_link());
        }
        inside_app(&next, &prefix)?;
        decoded = next;
    }
    Err(bad_link())
}

fn normalize(body: &mut PublishBody, app_code: &str) -> Result<Value, AppError> {
    // PostgreSQL text cannot represent NUL. Reject it as an input error rather
    // than allowing an otherwise valid JSON body to fail during persistence.
    if body.title.contains('\0') || body.content.contains('\0') {
        return Err(AppError::bad_request("title 或 content 不能包含空字符"));
    }
    if body.idempotency_key.is_empty()
        || body.idempotency_key.len() > 128
        || !body
            .idempotency_key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    {
        return Err(AppError::bad_request(
            "idempotencyKey 必须为 1 到 128 位 ASCII 字母、数字或 ._:-",
        ));
    }
    body.title = body.title.trim().to_owned();
    if body.title.is_empty() || body.title.chars().count() > 200 {
        return Err(AppError::bad_request("title 不能为空且不超过 200 字"));
    }
    if !valid_type(&body.r#type) {
        return Err(AppError::bad_request("type 不合法"));
    }
    if body.content.chars().count() > 8192 || body.link.chars().count() > 300 {
        return Err(AppError::bad_request("content 或 link 超过长度上限"));
    }
    body.link = normalize_link(&body.link, app_code)?;
    if body.link.chars().count() > 300 {
        return Err(AppError::bad_request("link 超过长度上限"));
    }
    Ok(
        json!({ "target": body.target, "type": body.r#type, "title": body.title,
        "content": body.content, "link": body.link }),
    )
}

// Data authorization is attached to this direct membership, never to ancestor roles.
async fn publisher_access(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Publisher,
    private: Option<(
        &crate::state::AppState,
        &crate::directory_authorization::PrivateAccess,
    )>,
) -> Result<(String, String, String, String), AppError> {
    if let Some((_, access)) = private {
        let checked = access.initial();
        let binding = &checked.binding;
        let active: bool =
            sqlx::query_scalar("SELECT identity_binding_active($1,$2,$3,$4,$5,'EMPLOYEE',$6)")
                .bind(binding.user_id)
                .bind(binding.tenant_id)
                .bind(binding.organization_id)
                .bind(binding.account_version)
                .bind(binding.membership_id)
                .bind(binding.identity_version)
                .fetch_one(&mut **tx)
                .await?;
        if !active || binding.expires_at <= Utc::now() {
            return Err(AppError::unauthorized("通知发布原成员来源已失效"));
        }
        let context = &checked.context;
        if !context["permissions"]
            .as_array()
            .is_some_and(|permissions| permissions.iter().any(|p| p == "notification:publish"))
        {
            return Err(AppError::forbidden(
                "无当前应用 notification:publish 操作权限",
            ));
        }
        let scope = context["dataGrants"]
            .as_array()
            .and_then(|grants| {
                grants.iter().find(|grant| {
                    grant["resource"] == "notification" && grant["action"] == "publish"
                })
            })
            .and_then(|grant| grant["scope"].as_str())
            .ok_or_else(|| AppError::forbidden("缺少 notification/publish 独立数据授权"))?;
        let username = context["user"]["username"]
            .as_str()
            .ok_or(AppError::Unavailable)?;
        let app_name = context["application"]["name"]
            .as_str()
            .ok_or(AppError::Unavailable)?;
        access.expires_at()?;
        return Ok((
            username.into(),
            app_name.into(),
            binding.audience.clone(),
            scope.into(),
        ));
    }
    if actor
        .token_expires_at
        .is_some_and(|expiry| expiry <= Utc::now())
    {
        return Err(AppError::unauthorized("凭证已过期"));
    }
    let identity: Option<(String, bool, String, String, bool)> = sqlx::query_as(
        "SELECT u.username,u.is_super_admin,a.name,a.code,t.is_platform AND o.parent_id IS NULL FROM users u \
         JOIN platform_sessions s ON s.user_id=u.id AND s.id=$5 \
           AND s.tenant_id=$2 AND s.organization_id=$3 AND s.revoked_at IS NULL AND s.expires_at>statement_timestamp() \
         JOIN memberships m ON m.id=s.membership_id AND m.user_id=u.id AND m.organization_id=$3 \
           AND m.member_type='EMPLOYEE' AND m.status='ENABLED' \
           AND m.identity_version=s.identity_version AND s.member_type='EMPLOYEE' \
           AND platform_session_active(s.id) \
         JOIN organizations o ON o.id=m.organization_id AND o.tenant_id=$2 AND org_chain_active(o.id) AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>statement_timestamp()) \
         JOIN tenants t ON t.id=$2 AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>statement_timestamp()) \
         JOIN applications a ON a.id=$4 AND a.status='ENABLED' \
           AND 'EMPLOYEE'=ANY(a.allowed_member_types) \
         JOIN tenant_applications ta ON ta.tenant_id=$2 AND ta.application_id=a.id \
         JOIN organization_applications oa ON oa.organization_id=$3 AND oa.application_id=a.id \
         WHERE u.id=$1 AND u.status='ENABLED' \
           AND ($7::timestamptz IS NULL OR $7>statement_timestamp()) \
           AND ($6::text IS NULL OR EXISTS(SELECT 1 FROM platform_app_capabilities tok \
             WHERE tok.token_hash=$6 AND platform_app_capability_active(tok.token_hash) \
               AND tok.user_id=$1 AND tok.tenant_id=$2 AND tok.organization_id=$3 \
               AND tok.application_id=$4 AND tok.platform_session_id=$5 \
               AND tok.membership_id=s.membership_id AND tok.member_type='EMPLOYEE' \
               AND tok.identity_version=s.identity_version AND tok.account_version=s.account_version))"
    ).bind(actor.user_id).bind(actor.tenant_id).bind(actor.organization_id)
        .bind(actor.application_id).bind(actor.session_id).bind(&actor.token_hash).bind(actor.token_expires_at)
        .fetch_optional(&mut **tx).await?;
    let Some((username, super_admin, app_name, app_code, platform_root)) = identity else {
        if actor.token_hash.is_some() {
            return Err(AppError::unauthorized("应用凭证或上下文已失效"));
        }
        return Err(AppError::forbidden("当前上下文未授权该应用"));
    };
    let permission: bool = sqlx::query_scalar(
        "WITH RECURSIVE anc AS (SELECT id,parent_id FROM organizations WHERE id=$3 \
         UNION ALL SELECT o.id,o.parent_id FROM organizations o JOIN anc ON o.id=anc.parent_id) \
         SELECT EXISTS(SELECT 1 FROM menus menu WHERE menu.application_id=$4 \
          AND menu.status='ENABLED' AND menu.permission='notification:publish' AND \
          (EXISTS(SELECT 1 FROM tenant_menus tm WHERE tm.menu_id=menu.id AND tm.tenant_id=$2) \
           AND EXISTS(SELECT 1 FROM organization_menus om WHERE om.menu_id=menu.id AND om.organization_id=$3) \
           AND ($5 OR EXISTS(SELECT 1 FROM role_menus rm JOIN roles r ON r.id=rm.role_id AND r.status='ENABLED' \
           JOIN anc ON anc.id=r.organization_id JOIN user_roles ur ON ur.role_id=r.id AND ur.user_id=$1 \
           WHERE rm.menu_id=menu.id))))"
    ).bind(actor.user_id).bind(actor.tenant_id).bind(actor.organization_id)
        .bind(actor.application_id).bind(super_admin)
        .fetch_one(&mut **tx).await?;
    if !permission {
        return Err(AppError::forbidden(
            "无当前应用 notification:publish 操作权限",
        ));
    }
    let scope: Option<String> = sqlx::query_scalar(
        "SELECT g.scope_type FROM membership_data_grants g JOIN memberships m ON m.id=g.membership_id \
         JOIN platform_sessions s ON s.membership_id=m.id AND s.id=$4 \
         WHERE m.user_id=$1 AND m.organization_id=$2 AND g.application_id=$3 \
         AND m.member_type='EMPLOYEE' AND m.status='ENABLED' AND s.member_type='EMPLOYEE' \
         AND m.identity_version=s.identity_version AND platform_session_active(s.id) \
         AND g.resource_code='notification' AND g.action_code='publish'"
    ).bind(actor.user_id).bind(actor.organization_id).bind(actor.application_id).bind(actor.session_id)
        .fetch_optional(&mut **tx).await?;
    let scope = scope.ok_or_else(|| AppError::forbidden("缺少 notification/publish 数据授权"))?;
    // A direct TENANT grant cannot expand the active organization's data boundary.
    // This also applies to global administrators who enter a platform child.
    let scope = if scope == "TENANT" && !platform_root {
        "ORG_SUBTREE".to_owned()
    } else {
        scope
    };
    Ok((username, app_name, app_code, scope))
}

async fn target_scope(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Publisher,
    target: &Target,
    scope: &str,
) -> Result<(), AppError> {
    let org_id = match target {
        Target::Tenant {} if scope == "TENANT" => return Ok(()),
        Target::Tenant {} => return Err(AppError::forbidden("数据授权不允许租户广播")),
        Target::Organization { .. } if scope == "SELF" => {
            return Err(AppError::forbidden("SELF 不允许组织广播"))
        }
        Target::User {
            user_id,
            organization_id,
        } => {
            if scope == "SELF"
                && (*user_id != actor.user_id || *organization_id != actor.organization_id)
            {
                return Err(AppError::not_found("目标不存在或不在授权范围"));
            }
            *organization_id
        }
        Target::Organization { organization_id } => *organization_id,
    };
    let allowed: bool = sqlx::query_scalar(
        "WITH RECURSIVE subtree AS (SELECT id FROM organizations WHERE id=$2 AND tenant_id=$1 \
         UNION SELECT o.id FROM organizations o JOIN subtree s ON o.parent_id=s.id WHERE o.tenant_id=$1) \
         SELECT EXISTS(SELECT 1 FROM organizations o WHERE o.id=$3 AND o.tenant_id=$1 \
         AND ($4='TENANT' OR ($4='ORG_SUBTREE' AND o.id IN (SELECT id FROM subtree)) \
         OR ($4 IN ('SELF','ORG') AND o.id=$2)))"
    ).bind(actor.tenant_id).bind(actor.organization_id).bind(org_id).bind(scope)
        .fetch_one(&mut **tx).await?;
    if !allowed {
        return Err(AppError::not_found("目标不存在或不在授权范围"));
    }
    Ok(())
}

pub async fn publish(
    pool: &sqlx::PgPool,
    actor: Publisher,
    body: PublishBody,
    ip: &str,
    ua: &str,
) -> Result<Publication, AppError> {
    publish_bound(pool, actor, body, ip, ua, None, false).await
}

/// Private service authority is retained as such; no synthetic platform session
/// or replacement bearer is ever created for an external USER.
pub(crate) async fn publish_private(
    state: &crate::state::AppState,
    access: &crate::directory_authorization::PrivateAccess,
    mut body: PublishBody,
    recovery_only: bool,
    ip: &str,
    ua: &str,
) -> Result<Publication, AppError> {
    let checked = access.initial();
    normalize(&mut body, &checked.binding.audience)?;
    body.idempotency_key =
        hash::sha256_hex(&json!([access.source_digest(), body.idempotency_key]).to_string());
    // Both online authority checkpoints finish without a publication connection
    // or foreign-key lock. The final transaction uses a fresh local snapshot.
    access
        .revalidate(state, &checked.binding.audience, checked.revision)
        .await?;
    let binding = &checked.binding;
    let actor = Publisher {
        user_id: binding.user_id,
        tenant_id: binding.tenant_id,
        organization_id: binding.organization_id,
        application_id: binding.application_id,
        // This field is used only by the internal branch; private authority uses
        // its authenticated binding and never looks up a platform session by it.
        session_id: binding.session_id,
        token_hash: None,
        token_expires_at: Some(binding.expires_at),
    };
    publish_bound(
        &state.pool,
        actor,
        body,
        ip,
        ua,
        Some((state, access)),
        recovery_only,
    )
    .await
}

async fn publish_bound(
    pool: &sqlx::PgPool,
    actor: Publisher,
    mut body: PublishBody,
    ip: &str,
    ua: &str,
    private: Option<(
        &crate::state::AppState,
        &crate::directory_authorization::PrivateAccess,
    )>,
    recovery_only: bool,
) -> Result<Publication, AppError> {
    let mut tx = pool.begin().await?;
    let (username, app_name, app_code, scope) = publisher_access(&mut tx, &actor, private).await?;
    let publisher_member: MemberProof = if let Some((_, access)) = private {
        let binding = &access.initial().binding;
        MemberProof {
            membership_id: binding.membership_id,
            member_type: binding.member_type.clone(),
            identity_version: binding.identity_version,
        }
    } else {
        sqlx::query_as(
            "SELECT membership_id,member_type,identity_version FROM platform_sessions \
         WHERE id=$1 AND user_id=$2 AND tenant_id=$3 AND organization_id=$4 \
         AND member_type='EMPLOYEE' AND platform_session_active(id)",
        )
        .bind(actor.session_id)
        .bind(actor.user_id)
        .bind(actor.tenant_id)
        .bind(actor.organization_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::unauthorized("通知发布者成员身份已失效"))?
    };
    let mut normalized = normalize(&mut body, &app_code)?;
    if let Some((_, access)) = private {
        normalized["authority"] = json!({"contractVersion":"resource-notification/1", "sourceDigest":access.source_digest()});
    }
    target_scope(&mut tx, &actor, &body.target, &scope).await?;
    let request_hash = hash::sha256_hex(&normalized.to_string());
    let namespace = json!([
        actor.tenant_id,
        actor.organization_id,
        actor.application_id,
        actor.user_id,
        publisher_member.membership_id,
        publisher_member.identity_version,
        body.idempotency_key
    ])
    .to_string();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 10))")
        .bind(namespace)
        .execute(&mut *tx)
        .await?;
    // Recheck after a competing publication finishes waiting for the same namespace.
    let (_, _, _, current_scope) = publisher_access(&mut tx, &actor, private).await?;
    target_scope(&mut tx, &actor, &body.target, &current_scope).await?;
    let prior: Option<(Uuid, String, i32, i32, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id,request_hash,recipient_count,delivery_count,created_at FROM notification_publications \
         WHERE tenant_id=$1 AND publisher_organization_id=$2 AND application_id=$3 \
         AND publisher_user_id=$4 AND idempotency_key=$5 \
         AND publisher_membership_id=$6 AND publisher_member_type=$7 AND publisher_identity_version=$8"
    ).bind(actor.tenant_id).bind(actor.organization_id).bind(actor.application_id)
        .bind(actor.user_id).bind(&body.idempotency_key).bind(publisher_member.membership_id)
        .bind(&publisher_member.member_type).bind(publisher_member.identity_version)
        .fetch_optional(&mut *tx).await?;
    if let Some((publication_id, prior_hash, recipient_count, delivery_count, created_at)) = prior {
        if prior_hash.trim_end() != request_hash {
            return Err(AppError::conflict("幂等键已用于不同通知内容"));
        }
        let (_, _, _, final_scope) = publisher_access(&mut tx, &actor, private).await?;
        target_scope(&mut tx, &actor, &body.target, &final_scope).await?;
        if let Some((state, access)) = private {
            access.revalidate_local(state).await?;
            recipients_current(&mut tx, publication_id).await?;
        }
        tx.commit().await?;
        return Ok(Publication {
            publication_id,
            recipient_count,
            delivery_count,
            created_at,
            duplicate: true,
        });
    }
    if recovery_only {
        return Err(AppError::not_found("原操作未找到已提交的通知发布结果"));
    }
    let (target_user, target_org) = match body.target {
        Target::User {
            user_id,
            organization_id,
        } => (Some(user_id), Some(organization_id)),
        Target::Organization { organization_id } => (None, Some(organization_id)),
        Target::Tenant {} => (None, None),
    };
    let deliveries: Vec<Recipient> = sqlx::query_as(
        "SELECT m.user_id,o.id AS organization_id,m.id AS membership_id,m.member_type,m.identity_version \
         FROM memberships m \
         JOIN users u ON u.id=m.user_id AND u.status='ENABLED' \
         JOIN organizations o ON o.id=m.organization_id AND o.tenant_id=$1 AND org_chain_active(o.id) AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>statement_timestamp()) \
         JOIN tenants t ON t.id=o.tenant_id AND t.status='ENABLED' \
           AND (t.expires_at IS NULL OR t.expires_at>statement_timestamp()) \
         JOIN applications a ON a.id=$2 AND a.status='ENABLED' \
           AND 'EMPLOYEE'=ANY(a.allowed_member_types) \
         JOIN organization_applications oa ON oa.organization_id=o.id AND oa.application_id=$2 \
         JOIN tenant_applications ta ON ta.tenant_id=o.tenant_id AND ta.application_id=$2 \
         WHERE m.member_type='EMPLOYEE' AND m.status='ENABLED' \
         AND ($3::uuid IS NULL OR m.user_id=$3) AND ($4::uuid IS NULL OR o.id=$4) \
         ORDER BY o.id,m.user_id,m.id LIMIT 1001"
    ).bind(actor.tenant_id).bind(actor.application_id).bind(target_user).bind(target_org)
        .fetch_all(&mut *tx).await?;
    if deliveries.is_empty() {
        return Err(if target_user.is_some() {
            AppError::not_found("目标不存在或无法接收")
        } else {
            AppError::bad_request("没有可接收通知的有效成员")
        });
    }
    if deliveries.len() > DELIVERY_LIMIT {
        return Err(AppError::bad_request("单次发布最多 1000 条投递"));
    }
    let recipient_count = deliveries
        .iter()
        .map(|recipient| recipient.user_id)
        .collect::<HashSet<_>>()
        .len() as i32;
    let delivery_count = deliveries.len() as i32;
    if private.is_some() {
        // Inbox rows can be deleted by their recipient. Keep the original
        // membership proofs in the immutable publication, outside that inbox,
        // so deletion cannot erase the recovery authorization fence. These
        // server facts do not change the hash of the caller's original request.
        normalized["authority"]["recipientProofs"] =
            serde_json::to_value(&deliveries).map_err(|_| AppError::Unavailable)?;
    }
    let (publication_id, created_at): (Uuid, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO notification_publications (tenant_id,publisher_organization_id,application_id, \
         publisher_user_id,publisher_username,application_name,idempotency_key,request_hash,request_body,recipient_count,delivery_count, \
         id,publisher_membership_id,publisher_member_type,publisher_identity_version) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) RETURNING id,created_at"
    ).bind(actor.tenant_id).bind(actor.organization_id).bind(actor.application_id).bind(actor.user_id)
        .bind(&username).bind(app_name).bind(&body.idempotency_key).bind(request_hash)
        .bind(sqlx::types::Json(normalized)).bind(recipient_count).bind(delivery_count)
        .bind(crate::util::uuid7::new_uuid()).bind(publisher_member.membership_id)
        .bind(&publisher_member.member_type).bind(publisher_member.identity_version)
        .fetch_one(&mut *tx).await?;
    for recipient in deliveries {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO notifications(user_id,type,title,content,link,source_kind,tenant_id,organization_id,application_id,publication_id, \
             id,recipient_membership_id,recipient_member_type,recipient_identity_version) \
             VALUES($1,$2,$3,$4,$5,'APP',$6,$7,$8,$9,$10,$11,$12,$13) RETURNING id"
        ).bind(recipient.user_id).bind(&body.r#type).bind(&body.title).bind(&body.content).bind(&body.link)
            .bind(actor.tenant_id).bind(recipient.organization_id).bind(actor.application_id).bind(publication_id)
            .bind(crate::util::uuid7::new_uuid()).bind(recipient.membership_id)
            .bind(&recipient.member_type).bind(recipient.identity_version)
            .fetch_one(&mut *tx).await?;
        notify_hint(&mut tx, id, actor.tenant_id, &recipient).await?;
    }
    audit::write_in_tx_for_app(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(actor.user_id),
            username: &username,
            tenant_id: Some(actor.tenant_id),
            organization_id: Some(actor.organization_id),
            action: "PUBLISH_NOTIFICATION",
            detail: &audit::change_detail(
                "notificationPublication",
                publication_id,
                Value::Null,
                json!({"recipientCount": recipient_count, "deliveryCount": delivery_count}),
            ),
            ip,
            user_agent: ua,
        },
        actor.application_id,
    )
    .await?;
    // Recipient/publication FKs and audit writes can wait after the earlier
    // advisory barrier. Recheck the publisher after those waits so revocation
    // rolls back the complete publication and its transactional notify hints.
    let (_, _, _, final_scope) = publisher_access(&mut tx, &actor, private).await?;
    target_scope(&mut tx, &actor, &body.target, &final_scope).await?;
    if let Some((state, access)) = private {
        access.revalidate_local(state).await?;
        recipients_current(&mut tx, publication_id).await?;
    }
    tx.commit().await?;
    Ok(Publication {
        publication_id,
        recipient_count,
        delivery_count,
        created_at,
        duplicate: false,
    })
}

async fn recipients_current(
    tx: &mut Transaction<'_, Postgres>,
    publication: Uuid,
) -> Result<(), AppError> {
    let (tenant, application, count, body): (Uuid, Uuid, i32, Value) = sqlx::query_as(
        "SELECT tenant_id,application_id,delivery_count,request_body FROM notification_publications WHERE id=$1",
    )
    .bind(publication)
    .fetch_one(&mut **tx)
    .await?;
    let original = recipient_proofs(&body, count)?;
    let invalid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jsonb_to_recordset($1::jsonb) AS original(\"userId\" uuid,\"organizationId\" uuid,\"membershipId\" uuid,\"memberType\" text,\"identityVersion\" bigint) WHERE NOT EXISTS(SELECT 1 FROM memberships m JOIN users u ON u.id=m.user_id AND u.status='ENABLED' JOIN organizations o ON o.id=m.organization_id AND o.tenant_id=$2 AND org_chain_active(o.id) AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp()) JOIN tenants t ON t.id=o.tenant_id AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) JOIN applications a ON a.id=$3 AND a.status='ENABLED' AND 'EMPLOYEE'=ANY(a.allowed_member_types) JOIN tenant_applications ta ON ta.tenant_id=t.id AND ta.application_id=a.id JOIN organization_applications oa ON oa.organization_id=o.id AND oa.application_id=a.id WHERE m.id=original.\"membershipId\" AND m.user_id=original.\"userId\" AND m.organization_id=original.\"organizationId\" AND m.member_type=original.\"memberType\" AND m.status='ENABLED' AND m.identity_version=original.\"identityVersion\"))")
        .bind(sqlx::types::Json(original)).bind(tenant).bind(application).fetch_one(&mut **tx).await?;
    if invalid {
        return Err(AppError::forbidden("原通知接收成员或应用范围已失效"));
    }
    Ok(())
}

fn recipient_proofs(body: &Value, count: i32) -> Result<Vec<Recipient>, AppError> {
    let original: Vec<Recipient> = serde_json::from_value(
        body.pointer("/authority/recipientProofs")
            .cloned()
            .ok_or(AppError::Unavailable)?,
    )
    .map_err(|_| AppError::Unavailable)?;
    if count <= 0
        || count as usize > DELIVERY_LIMIT
        || original.len() != count as usize
        || original.iter().any(|recipient| {
            ![
                recipient.user_id,
                recipient.organization_id,
                recipient.membership_id,
            ]
            .into_iter()
            .all(crate::util::uuid7::is_uuid_v7)
                || recipient.member_type != "EMPLOYEE"
                || recipient.identity_version < 1
        })
        || original
            .iter()
            .map(|recipient| {
                (
                    recipient.user_id,
                    recipient.organization_id,
                    recipient.membership_id,
                )
            })
            .collect::<HashSet<_>>()
            .len()
            != original.len()
    {
        // Missing/corrupt immutable proof is an unknown result, never a 404
        // that could authorize publication of the request again.
        return Err(AppError::Unavailable);
    }
    Ok(original)
}

async fn notify_hint(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    tenant_id: Uuid,
    recipient: &Recipient,
) -> Result<(), AppError> {
    let payload = json!({
        "hintVersion":"1", "id":id,"tenantId":tenant_id,
        "organizationId":recipient.organization_id,"userId":recipient.user_id,
        "membershipId":recipient.membership_id,"memberType":recipient.member_type,
        "identityVersion":recipient.identity_version,
    })
    .to_string();
    sqlx::query("SELECT pg_notify('new_notification',$1)")
        .bind(payload)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The caller supplies proven recipient contexts and includes delivery in its domain transaction.
pub async fn system_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    contexts: &[(Uuid, Uuid, Uuid)],
    ntype: &str,
    title: &str,
    content: &str,
    link: &str,
) -> Result<(), AppError> {
    let mut seen = HashSet::new();
    for &(user_id, tenant_id, organization_id) in contexts {
        if !seen.insert((user_id, tenant_id, organization_id)) {
            continue;
        }
        let delivered: Option<(Uuid,Uuid,String,i64)> = sqlx::query_as(
            "INSERT INTO notifications(user_id,type,title,content,link,source_kind,tenant_id,organization_id, \
             id,recipient_membership_id,recipient_member_type,recipient_identity_version) \
             SELECT $1,$2,$3,$4,$5,'SYSTEM',$6,$7,$8,m.id,m.member_type,m.identity_version \
             FROM memberships m JOIN users u ON u.id=m.user_id AND u.status='ENABLED' \
             JOIN organizations o ON o.id=m.organization_id AND o.tenant_id=$6 AND org_chain_active(o.id) AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>statement_timestamp()) \
             JOIN tenants t ON t.id=o.tenant_id AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>statement_timestamp()) \
             WHERE m.user_id=$1 AND m.organization_id=$7 AND m.member_type='EMPLOYEE' AND m.status='ENABLED' \
             RETURNING id,recipient_membership_id,recipient_member_type,recipient_identity_version"
        ).bind(user_id).bind(ntype).bind(title).bind(content).bind(link).bind(tenant_id).bind(organization_id)
            .bind(crate::util::uuid7::new_uuid()).fetch_optional(&mut **tx).await?;
        if let Some((id, membership_id, member_type, identity_version)) = delivered {
            let recipient = Recipient {
                user_id,
                organization_id,
                membership_id,
                member_type,
                identity_version,
            };
            notify_hint(tx, id, tenant_id, &recipient).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> PublishBody {
        PublishBody {
            idempotency_key: "text-validation".into(),
            target: Target::Tenant {},
            r#type: "INFO".into(),
            title: "通知".into(),
            content: "第一行\n第二行\t详情".into(),
            link: String::new(),
        }
    }

    #[test]
    fn notification_text_rejects_nul_but_preserves_multiline_content() {
        let mut valid = body();
        normalize(&mut valid, "example").unwrap();
        assert_eq!(valid.content, "第一行\n第二行\t详情");
        for is_title in [true, false] {
            let mut invalid = body();
            if is_title {
                invalid.title.push('\0');
            } else {
                invalid.content.push('\0');
            }
            assert!(matches!(
                normalize(&mut invalid, "example"),
                Err(AppError::BadRequest(_))
            ));
        }
    }

    #[test]
    fn immutable_recipient_proofs_fail_closed_when_missing_corrupt_or_incomplete() {
        let recipient = json!({
            "userId":crate::util::uuid7::fixture_uuid(1),
            "organizationId":crate::util::uuid7::fixture_uuid(2),
            "membershipId":crate::util::uuid7::fixture_uuid(3),
            "memberType":"EMPLOYEE", "identityVersion":5,
        });
        let snapshot = |proofs: Value| json!({"authority":{"recipientProofs":proofs}});
        let valid = snapshot(json!([recipient]));
        assert!(recipient_proofs(&valid, 1).is_ok());
        for (value, count) in [
            (json!({}), 1),
            (snapshot(Value::Null), 1),
            (snapshot(json!([])), 0),
            (valid.clone(), 2),
            (valid.clone(), 1001),
            (snapshot(json!([recipient, recipient])), 2),
        ] {
            assert!(matches!(
                recipient_proofs(&value, count),
                Err(AppError::Unavailable)
            ));
        }
        for (field, value) in [
            ("membershipId", json!(Uuid::nil())),
            ("memberType", json!("CUSTOMER")),
            ("identityVersion", json!(0)),
            ("identityVersion", json!("5")),
            ("unexpected", json!(true)),
        ] {
            let mut corrupt = recipient.clone();
            corrupt[field] = value;
            assert!(matches!(
                recipient_proofs(&snapshot(json!([corrupt])), 1),
                Err(AppError::Unavailable)
            ));
        }
    }

    #[test]
    fn publication_request_cannot_supply_immutable_recipient_authority() {
        let request = json!({"idempotencyKey":"owned-proof-injection",
            "target":{"kind":"TENANT"},"type":"INFO","title":"Notification",
            "authority":{"recipientProofs":[]}});
        assert!(serde_json::from_value::<PublishBody>(request).is_err());
    }
}
