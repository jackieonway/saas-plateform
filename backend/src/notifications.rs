//! Publication and delivery persistence. All persistent new notifications have an explicit owner.
use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::util::hash;

pub const DELIVERY_LIMIT: usize = 1000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Target {
    #[serde(rename_all = "camelCase")]
    User {
        user_id: Uuid,
        organization_id: Uuid,
    },
    #[serde(rename_all = "camelCase")]
    Organization {
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
) -> Result<(String, String, String, String), AppError> {
    if actor
        .token_expires_at
        .is_some_and(|expiry| expiry <= Utc::now())
    {
        return Err(AppError::unauthorized("凭证已过期"));
    }
    let identity: Option<(String, bool, String, String)> = sqlx::query_as(
        "SELECT u.username,u.is_super_admin,a.name,a.code FROM users u \
         JOIN platform_sessions s ON s.user_id=u.id AND s.id=$5 \
           AND s.tenant_id=$2 AND s.organization_id=$3 AND s.revoked_at IS NULL AND s.expires_at>statement_timestamp() \
         JOIN memberships m ON m.user_id=u.id AND m.organization_id=$3 \
         JOIN organizations o ON o.id=m.organization_id AND o.tenant_id=$2 AND org_chain_active(o.id) AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>statement_timestamp()) \
         JOIN tenants t ON t.id=$2 AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>statement_timestamp()) \
         JOIN applications a ON a.id=$4 AND a.status='ENABLED' \
         JOIN tenant_applications ta ON ta.tenant_id=$2 AND ta.application_id=a.id \
         JOIN organization_applications oa ON oa.organization_id=$3 AND oa.application_id=a.id \
         WHERE u.id=$1 AND u.status='ENABLED' \
           AND ($7::timestamptz IS NULL OR $7>statement_timestamp()) \
           AND ($6::text IS NULL OR EXISTS(SELECT 1 FROM oauth_tokens tok \
             JOIN oauth_clients client ON client.client_id=tok.client_id AND client.status='ENABLED' \
               AND client.client_type='CONFIDENTIAL' AND client.application_id=tok.application_id \
             WHERE tok.token_hash=$6 AND tok.token_use='app' AND tok.token_type='access' \
               AND NOT tok.revoked AND tok.expires_at>statement_timestamp() AND tok.user_id=$1 AND tok.tenant_id=$2 \
               AND tok.organization_id=$3 AND tok.application_id=$4 AND tok.platform_session_id=$5 \
               AND tok.audience=a.code))"
    ).bind(actor.user_id).bind(actor.tenant_id).bind(actor.organization_id)
        .bind(actor.application_id).bind(actor.session_id).bind(&actor.token_hash).bind(actor.token_expires_at)
        .fetch_optional(&mut **tx).await?;
    let Some((username, super_admin, app_name, app_code)) = identity else {
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
          ($5 OR EXISTS(SELECT 1 FROM role_menus rm JOIN roles r ON r.id=rm.role_id AND r.status='ENABLED' \
           JOIN anc ON anc.id=r.organization_id JOIN user_roles ur ON ur.role_id=r.id AND ur.user_id=$1 \
           JOIN tenant_menus tm ON tm.menu_id=rm.menu_id AND tm.tenant_id=$2 \
           JOIN organization_menus om ON om.menu_id=rm.menu_id AND om.organization_id=$3 \
           WHERE rm.menu_id=menu.id)))"
    ).bind(actor.user_id).bind(actor.tenant_id).bind(actor.organization_id)
        .bind(actor.application_id).bind(super_admin).fetch_one(&mut **tx).await?;
    if !permission {
        return Err(AppError::forbidden(
            "无当前应用 notification:publish 操作权限",
        ));
    }
    let scope: Option<String> = sqlx::query_scalar(
        "SELECT g.scope_type FROM membership_data_grants g JOIN memberships m ON m.id=g.membership_id \
         WHERE m.user_id=$1 AND m.organization_id=$2 AND g.application_id=$3 \
         AND g.resource_code='notification' AND g.action_code='publish'"
    ).bind(actor.user_id).bind(actor.organization_id).bind(actor.application_id)
        .fetch_optional(&mut **tx).await?;
    let scope = scope.ok_or_else(|| AppError::forbidden("缺少 notification/publish 数据授权"))?;
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
         UNION ALL SELECT o.id FROM organizations o JOIN subtree s ON o.parent_id=s.id WHERE o.tenant_id=$1) \
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
    mut body: PublishBody,
    ip: &str,
    ua: &str,
) -> Result<Publication, AppError> {
    let mut tx = pool.begin().await?;
    let (username, app_name, app_code, scope) = publisher_access(&mut tx, &actor).await?;
    let normalized = normalize(&mut body, &app_code)?;
    target_scope(&mut tx, &actor, &body.target, &scope).await?;
    let request_hash = hash::sha256_hex(&normalized.to_string());
    let namespace = json!([
        actor.tenant_id,
        actor.organization_id,
        actor.application_id,
        actor.user_id,
        body.idempotency_key
    ])
    .to_string();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 10))")
        .bind(namespace)
        .execute(&mut *tx)
        .await?;
    // Recheck after a competing publication finishes waiting for the same namespace.
    let (_, _, _, current_scope) = publisher_access(&mut tx, &actor).await?;
    target_scope(&mut tx, &actor, &body.target, &current_scope).await?;
    let prior: Option<(Uuid, String, i32, i32, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id,request_hash,recipient_count,delivery_count,created_at FROM notification_publications \
         WHERE tenant_id=$1 AND publisher_organization_id=$2 AND application_id=$3 \
         AND publisher_user_id=$4 AND idempotency_key=$5"
    ).bind(actor.tenant_id).bind(actor.organization_id).bind(actor.application_id)
        .bind(actor.user_id).bind(&body.idempotency_key).fetch_optional(&mut *tx).await?;
    if let Some((publication_id, prior_hash, recipient_count, delivery_count, created_at)) = prior {
        if prior_hash.trim_end() != request_hash {
            return Err(AppError::conflict("幂等键已用于不同通知内容"));
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
    let (target_user, target_org) = match body.target {
        Target::User {
            user_id,
            organization_id,
        } => (Some(user_id), Some(organization_id)),
        Target::Organization { organization_id } => (None, Some(organization_id)),
        Target::Tenant {} => (None, None),
    };
    let deliveries: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT m.user_id,o.id FROM memberships m \
         JOIN users u ON u.id=m.user_id AND u.status='ENABLED' \
         JOIN organizations o ON o.id=m.organization_id AND o.tenant_id=$1 AND org_chain_active(o.id) AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>statement_timestamp()) \
         JOIN organization_applications oa ON oa.organization_id=o.id AND oa.application_id=$2 \
         JOIN tenant_applications ta ON ta.tenant_id=o.tenant_id AND ta.application_id=$2 \
         WHERE ($3::uuid IS NULL OR m.user_id=$3) AND ($4::uuid IS NULL OR o.id=$4) \
         ORDER BY o.id,m.user_id LIMIT 1001"
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
        .map(|(user, _)| *user)
        .collect::<HashSet<_>>()
        .len() as i32;
    let delivery_count = deliveries.len() as i32;
    let (publication_id, created_at): (Uuid, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO notification_publications (tenant_id,publisher_organization_id,application_id, \
         publisher_user_id,publisher_username,application_name,idempotency_key,request_hash,request_body,recipient_count,delivery_count) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING id,created_at"
    ).bind(actor.tenant_id).bind(actor.organization_id).bind(actor.application_id).bind(actor.user_id)
        .bind(&username).bind(app_name).bind(&body.idempotency_key).bind(request_hash)
        .bind(sqlx::types::Json(normalized)).bind(recipient_count).bind(delivery_count)
        .fetch_one(&mut *tx).await?;
    for (user_id, organization_id) in deliveries {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO notifications(user_id,type,title,content,link,source_kind,tenant_id,organization_id,application_id,publication_id) \
             VALUES($1,$2,$3,$4,$5,'APP',$6,$7,$8,$9) RETURNING id"
        ).bind(user_id).bind(&body.r#type).bind(&body.title).bind(&body.content).bind(&body.link)
            .bind(actor.tenant_id).bind(organization_id).bind(actor.application_id).bind(publication_id)
            .fetch_one(&mut *tx).await?;
        notify_hint(&mut tx, id, actor.tenant_id, organization_id, user_id).await?;
    }
    sqlx::query(
        "INSERT INTO audit_logs(user_id,username,tenant_id,tenant_name,organization_id,organization_name,action,detail,ip,user_agent) \
         VALUES($1,$2,$3,(SELECT name FROM tenants WHERE id=$3),$4,(SELECT name FROM organizations WHERE id=$4), \
         'PUBLISH_NOTIFICATION',$5,$6,$7)"
    ).bind(actor.user_id).bind(&username).bind(actor.tenant_id).bind(actor.organization_id)
        .bind(format!("发布通知 {publication_id}: {recipient_count} 人 / {delivery_count} 条"))
        .bind(ip).bind(ua).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Publication {
        publication_id,
        recipient_count,
        delivery_count,
        created_at,
        duplicate: false,
    })
}

async fn notify_hint(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    tenant_id: Uuid,
    organization_id: Uuid,
    user_id: Uuid,
) -> Result<(), AppError> {
    let payload = json!({ "id": id, "tenantId": tenant_id, "organizationId": organization_id, "userId": user_id }).to_string();
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
    for &(user_id, tenant_id, organization_id) in contexts {
        let id: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO notifications(user_id,type,title,content,link,source_kind,tenant_id,organization_id) \
             SELECT $1,$2,$3,$4,$5,'SYSTEM',$6,$7 WHERE EXISTS( \
             SELECT 1 FROM memberships m JOIN users u ON u.id=m.user_id AND u.status='ENABLED' \
             JOIN organizations o ON o.id=m.organization_id AND o.tenant_id=$6 AND org_chain_active(o.id) AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>statement_timestamp()) \
             JOIN tenants t ON t.id=o.tenant_id AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>statement_timestamp()) \
             WHERE m.user_id=$1 AND m.organization_id=$7) RETURNING id"
        ).bind(user_id).bind(ntype).bind(title).bind(content).bind(link).bind(tenant_id).bind(organization_id)
            .fetch_optional(&mut **tx).await?;
        if let Some(id) = id {
            notify_hint(tx, id, tenant_id, organization_id, user_id).await?;
        }
    }
    Ok(())
}
