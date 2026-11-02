//! Exact employee choices and short platform credentials owned by the RP.

use super::store::{self, Session};
use crate::{
    error::AppError,
    session,
    state::AppState,
    util::{audit, jwt},
};
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) async fn install_selection(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    bff: Uuid,
    verified: &super::protocol::Verified,
    proof: &crate::context_selection_http::authority::SourceProof,
    flow: &store::Flow,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    let extension = verified.selection.as_ref().ok_or(AppError::Unavailable)?;
    let account = &verified.proof;
    use saas_platform_backend::{
        context_selection::{Id, Intent, MemberType, Version},
        context_selection_store::{self, Consumer, ContextResult},
    };
    let consumer = Consumer {
        client_id: Id::new(account.client_id).map_err(|_| AppError::Unavailable)?,
        service_id: None,
    };
    if extension.transaction_id.uuid() != flow.id
        || proof.source.user_id.uuid() != account.user_id
        || proof.source.account_version.value() != account.account_version
        || proof.source.global_session_id.uuid() != account.global_session_id
        || proof.source.auth_time.timestamp() != account.auth_time
        || proof.client.policy_version.value() != account.client_policy_version
    {
        return Err(AppError::unauthorized("选择结果与本次账号认证不匹配"));
    }
    match &proof.intent {
        Intent::Login {} if flow.switch_session_id.is_none() => {}
        Intent::Switch {
            session_id,
            expected_version,
        } if Some(session_id.uuid()) == flow.switch_session_id
            && Some(expected_version.value()) == flow.switch_version => {}
        _ => return Err(AppError::unauthorized("选择操作不属于本次会话")),
    }
    let (binding, member) =
        crate::context_selection_http::consumption::validate(tx, extension, proof, consumer)
            .await?;
    if member.member_type != MemberType::Employee {
        return Err(AppError::forbidden("平台仅接受员工身份"));
    }
    let row: Member = sqlx::query_as(&format!("{MEMBERS} AND m.id=$2 FOR SHARE OF m"))
        .bind(account.user_id)
        .bind(member.membership_id.uuid())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::forbidden("员工身份已失效"))?;
    let current = store::session_id(&mut **tx, bff, true)
        .await?
        .ok_or(AppError::Unavailable)?;
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await?;
    let ttl =
        (state.config.session_ttl_hours * 3600).max(state.config.bff_context_renewal_ttl_seconds);
    let mut end = (now + chrono::Duration::seconds(ttl)).min(current.expires_at);
    if let Some(expiry) = row.org_expires {
        end = end.min(expiry);
    }
    if let Some(expiry) = row.tenant_expires {
        end = end.min(expiry);
    }
    let context = crate::util::uuid7::new_uuid();
    sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,ip,user_agent,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version,bff_session_id,bff_context_version) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'EMPLOYEE',$10,$11,$12,$13,$14,1)")
        .bind(context).bind(account.user_id).bind(member.tenant_id.uuid()).bind(member.organization_id.uuid()).bind(end)
        .bind(audit::client_ip(headers)).bind(audit::ua_of(headers)).bind(account.account_version).bind(member.membership_id.uuid())
        .bind(member.identity_version.value()).bind(&current.issuer).bind(account.client_id.to_string()).bind(account.client_policy_version).bind(bff).execute(&mut **tx).await?;
    // Renewal proofs require the complete active BFF/context association even
    // inside the transaction. Publish the reference before issuing its refresh.
    sqlx::query("UPDATE bff_sessions SET selected_session_id=$2 WHERE id=$1")
        .bind(bff)
        .bind(context)
        .execute(&mut **tx)
        .await?;
    let refresh_end =
        end.min(now + chrono::Duration::seconds(state.config.bff_context_renewal_ttl_seconds));
    let refresh =
        session::issue_refresh_in_tx(tx, context, account.account_version, refresh_end).await?;
    let encrypted = store::keys(state)?.seal(
        bff,
        &current.issuer,
        account.client_id,
        "platform-refresh",
        &refresh,
    )?;
    sqlx::query(
        "UPDATE bff_sessions SET selected_session_id=$2,platform_refresh_ciphertext=$3 WHERE id=$1",
    )
    .bind(bff)
    .bind(context)
    .bind(encrypted)
    .execute(&mut **tx)
    .await?;
    context_selection_store::consume(
        tx,
        &extension.credential,
        consumer,
        &binding,
        &member,
        &ContextResult::Employee {
            context_id: Id::new(context).map_err(|_| AppError::Unavailable)?,
            version: Version::new(1).map_err(|_| AppError::Unavailable)?,
        },
        None,
    )
    .await
    .map_err(|_| AppError::conflict("选择凭据已过期、消费或变化"))?;
    store::deadline(
        tx,
        end.min(proof.source_expires_at)
            .min(proof.interaction_expires_at),
    )
    .await?;
    let active: bool = sqlx::query_scalar("SELECT platform_session_active($1)")
        .bind(context)
        .fetch_one(&mut **tx)
        .await?;
    if !active {
        return Err(AppError::unauthorized("新员工上下文已失效"));
    }
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id=$1")
        .bind(account.user_id)
        .fetch_one(&mut **tx)
        .await?;
    audit::write_in_tx(tx,audit::AuditInfo {user_id:Some(account.user_id),username:&username,
        tenant_id:Some(member.tenant_id.uuid()),organization_id:Some(member.organization_id.uuid()),action:"CONTEXT_SELECTION_CONSUME",
        detail:&json!({"transactionId":extension.transaction_id,"clientId":account.client_id,"oldBffSessionId":flow.switch_session_id,
            "expectedVersion":flow.switch_version.map(|v|v.to_string()),"bffSessionId":bff,"platformSessionId":context,
            "membershipId":member.membership_id,"memberType":"EMPLOYEE","identityVersion":member.identity_version}).to_string(),
        ip:&audit::client_ip(headers),user_agent:&audit::ua_of(headers)}).await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Selection {
    expected_version: String,
    #[serde(default, deserialize_with = "optional_id")]
    membership_id: Option<Uuid>,
    #[serde(default, deserialize_with = "optional_id")]
    organization_id: Option<Uuid>,
}

fn optional_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Uuid>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|raw| {
            let id = crate::util::uuid7::parse_uuid(&raw).map_err(serde::de::Error::custom)?;
            if id.to_string() != raw {
                return Err(serde::de::Error::custom("需要规范 UUID v7"));
            }
            Ok(id)
        })
        .transpose()
}

#[derive(sqlx::FromRow)]
struct Account {
    username: String,
    real_name: String,
    email: String,
    phone: String,
    avatar: String,
    gender: String,
    is_super_admin: bool,
}

#[derive(sqlx::FromRow)]
struct Member {
    membership_id: Uuid,
    user_id: Uuid,
    organization_id: Uuid,
    identity_version: i64,
    organization_name: String,
    organization_path: String,
    org_is_root: bool,
    org_expires: Option<DateTime<Utc>>,
    tenant_id: Uuid,
    tenant_name: String,
    tenant_code: String,
    tenant_is_platform: bool,
    tenant_expires: Option<DateTime<Utc>>,
    department_id: Option<Uuid>,
    department_name: Option<String>,
}

const MEMBERS: &str = "SELECT m.id AS membership_id,m.user_id,m.organization_id,m.identity_version, \
    o.name AS organization_name,COALESCE(org_path(o.id),'') AS organization_path, \
    o.parent_id IS NULL AS org_is_root,org_chain_expires_at(o.id) AS org_expires, \
    t.id AS tenant_id,t.name AS tenant_name,t.code AS tenant_code,t.is_platform AS tenant_is_platform, \
    t.expires_at AS tenant_expires,m.department_id,d.name AS department_name \
    FROM memberships m JOIN organizations o ON o.id=m.organization_id \
    JOIN tenants t ON t.id=o.tenant_id LEFT JOIN departments d ON d.id=m.department_id \
    WHERE m.user_id=$1 AND m.member_type='EMPLOYEE' AND m.status='ENABLED' \
    AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) \
    AND org_chain_active(o.id) \
    AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp())";

impl Member {
    fn choice(&self) -> Value {
        json!({"membershipId":self.membership_id,"userId":self.user_id,
            "memberType":"EMPLOYEE","status":"ENABLED","identityVersion":self.identity_version.to_string(),
            "consoleSelectable":true,"selectionDisabledReason":null,
            "organizationId":self.organization_id,"organizationName":self.organization_name,
            "organizationPath":self.organization_path,"tenantId":self.tenant_id,"tenantName":self.tenant_name,
            "departmentId":self.department_id,"departmentName":self.department_name})
    }
}

#[derive(sqlx::FromRow)]
struct Context {
    id: Uuid,
    user_id: Uuid,
    tenant_id: Uuid,
    organization_id: Uuid,
    membership_id: Uuid,
    member_type: String,
    identity_version: i64,
    account_version: i64,
    expires_at: DateTime<Utc>,
}

impl Context {
    fn identity(&self) -> Value {
        json!({"userId":self.user_id,"tenantId":self.tenant_id,"organizationId":self.organization_id,
            "membershipId":self.membership_id,"memberType":self.member_type,
            "identityVersion":self.identity_version.to_string(),"accountVersion":self.account_version.to_string()})
    }
}

fn changed() -> AppError {
    AppError::conflict("平台上下文已变化，请重新读取认证状态")
}

fn next_version(version: i64) -> Result<i64, AppError> {
    version.checked_add(1).ok_or_else(changed)
}

async fn locked<'a>(
    state: &'a AppState,
    original: &Session,
) -> Result<(Transaction<'a, Postgres>, Session, Account), AppError> {
    if original.issuer != store::issuer(state)? || original.oidc_client_id != store::client(state)?
    {
        return Err(AppError::unauthorized("平台认证来源不匹配"));
    }
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    store::lock_user(&mut tx, original.user_id).await?;
    store::lock_client(&mut tx, original.oidc_client_id, &original.issuer).await?;
    let current = store::session_id(&mut *tx, original.id, true)
        .await?
        .ok_or_else(|| AppError::unauthorized("平台认证已失效"))?;
    if current.version != original.version
        || current.selected_session_id != original.selected_session_id
    {
        return Err(changed());
    }
    if current.user_id != original.user_id
        || current.account_version != original.account_version
        || current.global_session_id != original.global_session_id
        || current.oidc_grant_id != original.oidc_grant_id
        || current.issuer != original.issuer
        || current.oidc_client_id != original.oidc_client_id
        || current.oidc_client_policy_version != original.oidc_client_policy_version
        || current.browser_id != original.browser_id
        || current.auth_time != original.auth_time
        || !super::equal(&current.token_hash, &original.token_hash)
        || !super::equal(&current.csrf_hash, &original.csrf_hash)
        || !super::equal(&current.nonce_hash, &original.nonce_hash)
    {
        return Err(AppError::unauthorized("原账号认证绑定已变化"));
    }
    if current.refresh_uncertain
        || current.refresh_lease_id.is_some()
        || current.logout_request_id.is_some()
    {
        return Err(AppError::Unavailable);
    }
    let browser: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM console_browsers WHERE id=$1 AND current_session_id=$2 AND expires_at>clock_timestamp())",
    ).bind(current.browser_id).bind(current.id).fetch_one(&mut *tx).await.map_err(|_|AppError::Unavailable)?;
    if !browser {
        return Err(AppError::unauthorized("浏览器认证代际已变化"));
    }
    store::session_proof(&mut tx, &current).await?;
    store::deadline(
        &mut tx,
        original
            .expires_at
            .min(original.access_expires_at)
            .min(current.expires_at)
            .min(current.access_expires_at),
    )
    .await?;
    let account: Account = sqlx::query_as(
        "SELECT username,COALESCE(real_name,'') AS real_name,COALESCE(email,'') AS email, \
         COALESCE(phone,'') AS phone,COALESCE(avatar,'') AS avatar,COALESCE(gender,'UNKNOWN') AS gender,is_super_admin \
         FROM users WHERE id=$1 AND status='ENABLED' AND session_version=$2",
    ).bind(original.user_id).bind(original.account_version).fetch_optional(&mut *tx).await
        .map_err(|_|AppError::Unavailable)?.ok_or_else(||AppError::unauthorized("原账号认证已失效"))?;
    Ok((tx, current, account))
}

async fn selected(
    tx: &mut Transaction<'_, Postgres>,
    session: &Session,
) -> Result<Option<Context>, AppError> {
    let Some(id) = session.selected_session_id else {
        return Ok(None);
    };
    sqlx::query_as(
        "SELECT id,user_id,tenant_id,organization_id,membership_id,member_type,identity_version,account_version,expires_at \
         FROM platform_sessions WHERE id=$1 AND bff_session_id=$2 AND bff_context_version=$3 \
         AND user_id=$4 AND account_version=$5 AND member_type='EMPLOYEE' AND platform_session_active(id)",
    ).bind(id).bind(session.id).bind(session.version).bind(session.user_id).bind(session.account_version)
        .fetch_optional(&mut **tx).await.map_err(|_|AppError::Unavailable)
}

async fn retire_context(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<(), AppError> {
    sqlx::query("UPDATE platform_sessions SET revoked_at=COALESCE(revoked_at,clock_timestamp()) WHERE id=$1")
        .bind(id).execute(&mut **tx).await?;
    sqlx::query("UPDATE platform_session_refresh_tokens SET consumed_at=COALESCE(consumed_at,clock_timestamp()) WHERE session_id=$1")
        .bind(id).execute(&mut **tx).await?;
    sqlx::query(
        "UPDATE platform_app_capabilities SET revoked=true WHERE platform_session_id=$1 AND NOT revoked",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn finish(
    tx: &mut Transaction<'_, Postgres>,
    original: &Session,
    current: &Session,
    context: Option<&Context>,
) -> Result<(), AppError> {
    store::session_proof(tx, current).await?;
    if let Some(context) = context {
        let active: bool = sqlx::query_scalar("SELECT platform_session_active($1)")
            .bind(context.id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| AppError::Unavailable)?;
        if !active {
            return Err(AppError::unauthorized("所选员工上下文已失效"));
        }
        store::deadline(tx, context.expires_at).await?;
    }
    store::deadline(
        tx,
        original
            .expires_at
            .min(original.access_expires_at)
            .min(current.expires_at)
            .min(current.access_expires_at),
    )
    .await
}

pub(super) async fn view(state: &AppState, original: Session) -> Result<Value, AppError> {
    let (mut tx, mut current, account) = locked(state, &original).await?;
    let context = selected(&mut tx, &current).await?;
    if context.is_none() && current.selected_session_id.is_some() {
        if let Some(id) = current.selected_session_id {
            retire_context(&mut tx, id).await?;
        }
        let version = next_version(current.version)?;
        sqlx::query("UPDATE bff_sessions SET selected_session_id=NULL,platform_refresh_ciphertext=NULL,version=$2 WHERE id=$1")
            .bind(current.id).bind(version).execute(&mut *tx).await?;
        current = store::session_id(&mut *tx, current.id, false)
            .await?
            .ok_or(AppError::Unavailable)?;
    }
    let members: Vec<Member> = sqlx::query_as(&format!("{MEMBERS} ORDER BY m.created_at,m.id"))
        .bind(current.user_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    let result = json!({"sessionId":current.id,"version":current.version.to_string(),
        "centralizedContextSelection":sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM context_selection_registrations WHERE client_id=$1 AND enabled)").bind(current.oidc_client_id).fetch_one(&mut *tx).await?,
        "account":{"id":current.user_id,"username":account.username,"realName":account.real_name,"avatar":account.avatar},
        "memberships":members.iter().map(Member::choice).collect::<Vec<_>>(),
        "context":context.as_ref().map(Context::identity),"expiresAt":current.expires_at});
    finish(&mut tx, &original, &current, context.as_ref()).await?;
    tx.commit().await?;
    Ok(result)
}

pub(super) async fn token(
    state: &AppState,
    headers: &HeaderMap,
    original: Session,
    body: Selection,
) -> Result<Value, AppError> {
    let expected = body
        .expected_version
        .parse::<i64>()
        .ok()
        .filter(|version| *version >= 1 && version.to_string() == body.expected_version)
        .ok_or_else(|| AppError::bad_request("上下文版本必须是规范正十进制字符串"))?;
    if expected != original.version {
        return Err(changed());
    }
    if body.membership_id.is_none() && body.organization_id.is_some() {
        return Err(AppError::bad_request("选择组织时必须指定具体员工成员身份"));
    }
    let selecting = body.membership_id.is_some();
    if selecting && sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM context_selection_registrations WHERE client_id=$1 AND enabled)").bind(original.oidc_client_id).fetch_one(&state.pool).await? {
        return Err(AppError::forbidden("请前往认证中心选择租户和组织"));
    }
    let (mut tx, mut current, account) = locked(state, &original).await?;
    if current.version != expected {
        return Err(changed());
    }
    let mut context = selected(&mut tx, &current).await?;
    let member_id = match body.membership_id {
        Some(id) => id,
        None => context
            .as_ref()
            .map(|context| context.membership_id)
            .ok_or_else(|| AppError::unauthorized("请先选择有效员工成员身份"))?,
    };
    let member: Member = sqlx::query_as(&format!("{MEMBERS} AND m.id=$2 FOR SHARE OF m"))
        .bind(original.user_id)
        .bind(member_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?
        .ok_or_else(|| AppError::forbidden("所选员工成员身份不可用"))?;
    if body
        .organization_id
        .is_some_and(|id| id != member.organization_id)
    {
        return Err(AppError::forbidden("组织与具体员工成员身份不匹配"));
    }
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    let refresh_end;
    if selecting {
        if let Some(id) = current.selected_session_id {
            retire_context(&mut tx, id).await?;
        }
        let version = next_version(current.version)?;
        let ttl = (state.config.session_ttl_hours * 3600)
            .max(state.config.bff_context_renewal_ttl_seconds);
        let mut end = (now + chrono::Duration::seconds(ttl))
            .min(original.expires_at)
            .min(current.expires_at);
        if let Some(expiry) = member.org_expires {
            end = end.min(expiry);
        }
        if let Some(expiry) = member.tenant_expires {
            end = end.min(expiry);
        }
        store::deadline(&mut tx, end).await?;
        let id = crate::util::uuid7::new_uuid();
        sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,ip,user_agent,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version,bff_session_id,bff_context_version) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'EMPLOYEE',$10,$11,$12,$13,$14,$15)")
            .bind(id).bind(original.user_id).bind(member.tenant_id).bind(member.organization_id).bind(end)
            .bind(audit::client_ip(headers)).bind(audit::ua_of(headers)).bind(original.account_version).bind(member.membership_id)
            .bind(member.identity_version).bind(&original.issuer).bind(original.oidc_client_id.to_string())
            .bind(original.oidc_client_policy_version).bind(current.id).bind(version).execute(&mut *tx).await?;
        sqlx::query("UPDATE bff_sessions SET selected_session_id=$2,version=$3,platform_refresh_ciphertext=NULL WHERE id=$1")
            .bind(current.id).bind(id).bind(version).execute(&mut *tx).await?;
        current = store::session_id(&mut *tx, current.id, false)
            .await?
            .ok_or(AppError::Unavailable)?;
        context = selected(&mut tx, &current).await?;
        refresh_end =
            end.min(now + chrono::Duration::seconds(state.config.bff_context_renewal_ttl_seconds));
    } else {
        let selected = context
            .as_ref()
            .ok_or_else(|| AppError::unauthorized("员工上下文已失效"))?;
        if selected.membership_id != member.membership_id
            || selected.identity_version != member.identity_version
            || selected.organization_id != member.organization_id
            || selected.tenant_id != member.tenant_id
        {
            return Err(AppError::unauthorized("原员工身份版本已变化"));
        }
        if current.platform_refresh_ciphertext != original.platform_refresh_ciphertext {
            return Err(changed());
        }
        let encrypted = original
            .platform_refresh_ciphertext
            .as_deref()
            .ok_or_else(|| AppError::unauthorized("平台上下文缺少续期证明"))?;
        let refresh = store::keys(state)?.open(
            original.id,
            &original.issuer,
            original.oidc_client_id,
            "platform-refresh",
            encrypted,
        )?;
        if !refresh
            .strip_prefix("psr_")
            .is_some_and(super::canonical_secret)
        {
            return Err(AppError::unauthorized("平台上下文续期证明无效"));
        }
        refresh_end = sqlx::query_scalar::<_, DateTime<Utc>>(
            "UPDATE platform_session_refresh_tokens SET consumed_at=clock_timestamp() \
             WHERE token_hash=$1 AND session_id=$2 AND session_version=$3 AND consumed_at IS NULL \
             AND expires_at>clock_timestamp() RETURNING expires_at",
        )
        .bind(super::digest(&refresh))
        .bind(selected.id)
        .bind(original.account_version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?
        .ok_or_else(|| AppError::unauthorized("平台上下文续期证明已失效"))?
        .min(selected.expires_at)
        .min(original.expires_at)
        .min(current.expires_at);
    }
    let context = context.ok_or_else(|| AppError::unauthorized("员工上下文已失效"))?;
    let refresh_end = member
        .org_expires
        .map_or(refresh_end, |end| refresh_end.min(end));
    let refresh_end = member
        .tenant_expires
        .map_or(refresh_end, |end| refresh_end.min(end));
    let refresh =
        session::issue_refresh_in_tx(&mut tx, context.id, original.account_version, refresh_end)
            .await?;
    let encrypted = store::keys(state)?.seal(
        original.id,
        &original.issuer,
        original.oidc_client_id,
        "platform-refresh",
        &refresh,
    )?;
    sqlx::query("UPDATE bff_sessions SET platform_refresh_ciphertext=$2 WHERE id=$1")
        .bind(current.id)
        .bind(encrypted)
        .execute(&mut *tx)
        .await?;
    let expiry = (now + chrono::Duration::seconds(state.config.bff_context_ttl_seconds))
        .min(context.expires_at)
        .min(original.access_expires_at)
        .min(current.access_expires_at)
        .min(original.expires_at)
        .min(current.expires_at);
    let expiry = member.org_expires.map_or(expiry, |end| expiry.min(end));
    let expiry = member
        .tenant_expires
        .map_or(expiry, |end| expiry.min(end))
        .timestamp();
    let expires_in = expiry - now.timestamp();
    if expires_in <= 0 {
        return Err(AppError::unauthorized("原认证期限不足，请重新登录"));
    }
    let claims = jwt::Claims {
        sub: original.user_id.to_string(),
        typ: "platform_session".to_owned(),
        tenant: Some(context.tenant_id.to_string()),
        org: Some(context.organization_id.to_string()),
        cid: None,
        scope: None,
        sid: Some(context.id.to_string()),
        ver: Some(original.account_version),
        aud: Some(jwt::PLATFORM_API_AUDIENCE.to_owned()),
        jti: None,
        exp: expiry,
        iat: now.timestamp(),
    };
    let access = jwt::sign_internal(
        &claims,
        jwt::InternalUse::PlatformSession,
        &state.config.jwt_secret,
    )?;
    audit::write_in_tx(&mut tx, audit::AuditInfo {
        user_id:Some(original.user_id),username:&account.username,tenant_id:Some(context.tenant_id),
        organization_id:Some(context.organization_id),action:if selecting {"BFF_CONTEXT_SELECT"}else{"BFF_CONTEXT_RENEW"},
        detail:&json!({"bffSessionId":current.id,"platformSessionId":context.id,"version":current.version.to_string(),
            "membershipId":context.membership_id,"memberType":"EMPLOYEE","identityVersion":context.identity_version.to_string(),
            "accountVersion":original.account_version.to_string()}).to_string(),
        ip:&audit::client_ip(headers),user_agent:&audit::ua_of(headers),
    }).await?;
    current = store::session_id(&mut *tx, current.id, false)
        .await?
        .ok_or(AppError::Unavailable)?;
    finish(&mut tx, &original, &current, Some(&context)).await?;
    store::deadline(&mut tx, refresh_end).await?;
    store::deadline(
        &mut tx,
        DateTime::<Utc>::from_timestamp(expiry, 0).ok_or(AppError::Unavailable)?,
    )
    .await?;
    let result = json!({"sessionId":current.id,"version":current.version.to_string(),"accessToken":access,"expiresIn":expires_in,
        "platformContext":member.tenant_is_platform&&member.org_is_root,
        "membershipId":context.membership_id,"memberType":"EMPLOYEE","identityVersion":context.identity_version.to_string(),
        "accountVersion":original.account_version.to_string(),
        "user":{"id":original.user_id,"username":account.username,"realName":account.real_name,"email":account.email,
            "phone":account.phone,"avatar":account.avatar,"gender":account.gender,"isSuperAdmin":account.is_super_admin},
        "tenant":{"id":member.tenant_id,"name":member.tenant_name,"code":member.tenant_code,"isPlatform":member.tenant_is_platform},
        "organization":{"id":member.organization_id,"name":member.organization_name,"path":member.organization_path}});
    tx.commit().await?;
    Ok(result)
}
