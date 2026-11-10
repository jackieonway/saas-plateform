use super::{assert_actor, global_operator, policy, require_application};
#[cfg(test)]
use crate::util::{audit, hash::sha256_hex};
use crate::{error::AppError, extractors::CurrentUser, state::AppState};
#[cfg(test)]
use axum::http::HeaderMap;
use axum::http::Method;
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(crate) async fn management_tx<'a>(
    state: &'a AppState,
    user: &CurrentUser,
    permission: &'static str,
    recent: bool,
) -> Result<Transaction<'a, Postgres>, AppError> {
    let mut tx = state.pool.begin().await?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    assert_actor(&mut tx, state, user, permission, recent).await?;
    Ok(tx)
}

pub(crate) fn permission_is_recent(permission: &str) -> bool {
    permission != policy::VIEW
}

pub(crate) fn permission_for(method: &Method, path: &str, body: Option<&Value>) -> &'static str {
    if path.contains("/invitations") || path.contains("/invitation-operations/") {
        return policy::INVITE;
    }
    if method == Method::GET {
        return policy::VIEW;
    }
    if path.contains("/review/applications/") {
        return match body.and_then(|value| value["status"].as_str()) {
            Some("SUSPENDED") => policy::DISABLE,
            _ if path.ends_with("/production") => policy::PUBLISH,
            _ => policy::REVIEW,
        };
    }
    if path.ends_with("/credentials/revoke") {
        return policy::DISABLE;
    }
    if path.ends_with("/credentials") {
        return policy::ROTATE;
    }
    if path.contains("/keys/") {
        return "system:oauth:keys";
    }
    if method == Method::PATCH
        && body
            .is_some_and(|value| matches!(value["status"].as_str(), Some("DISABLED" | "RETIRED")))
    {
        return policy::DISABLE;
    }
    if path.contains("/consent-groups")
        || path.contains("/applications/")
        || method == Method::PATCH
    {
        policy::SCOPES
    } else {
        policy::REGISTER
    }
}

pub(crate) async fn application_allowed(
    tx: &mut Transaction<'_, Postgres>,
    user: &CurrentUser,
    application: Uuid,
) -> Result<bool, AppError> {
    if global_operator(tx, user).await? {
        return Ok(true);
    }
    let assigned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_auth_application_operators operator JOIN platform_auth_applications app USING(application_id) JOIN platform_auth_partners partner ON partner.id=app.partner_id WHERE operator.application_id=$1 AND operator.user_id=$2 AND partner.status='ACTIVE')")
        .bind(application).bind(user.user_id).fetch_one(&mut **tx).await?;
    Ok(assigned)
}

#[cfg(test)]
pub(super) fn operation(headers: &HeaderMap) -> Result<Uuid, AppError> {
    let mut values = headers.get_all("x-identity-operation").iter();
    let raw = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::bad_request("管理写入需要浏览器预先保存的操作编号"))?;
    if values.next().is_some() {
        return Err(AppError::bad_request("管理操作编号重复"));
    }
    policy::id(raw)
}
#[cfg(test)]
pub(super) fn digest(path: &str, body: &Value) -> Result<String, AppError> {
    Ok(sha256_hex(
        &serde_json::to_string(&json!({"path":path,"body":body}))
            .map_err(|_| AppError::Unavailable)?,
    ))
}
#[cfg(test)]
pub(super) async fn replay(
    tx: &mut Transaction<'_, Postgres>,
    user: &CurrentUser,
    operation: Uuid,
    digest: &str,
) -> Result<Option<Value>, AppError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,6351749))")
        .bind(operation.to_string())
        .execute(&mut **tx)
        .await?;
    let row:Option<(Uuid,Uuid,String,Value)>=sqlx::query_as("SELECT actor_user_id,actor_session_id,request_digest,result_json FROM platform_auth_review_operations WHERE operation_id=$1")
        .bind(operation).fetch_optional(&mut **tx).await?;
    if let Some((actor, session, stored, result)) = row {
        if actor != user.user_id || session != user.session_id || stored != digest {
            return Err(AppError::conflict("原操作已绑定另一会话或管理内容"));
        }
        return Ok(Some(result));
    }
    let milliseconds = operation.as_bytes()[..6]
        .iter()
        .fold(0_i64, |value, byte| (value << 8) | i64::from(*byte));
    let started = chrono::DateTime::from_timestamp_millis(milliseconds)
        .ok_or_else(|| AppError::bad_request("管理操作时间无效"))?;
    let (admissible,remote_exists):(bool,bool)=sqlx::query_as("SELECT clock_timestamp() BETWEEN $2::timestamptz-interval '30 seconds' AND $2::timestamptz+interval '60 seconds',EXISTS(SELECT 1 FROM platform_auth_command_intents WHERE id=$1)")
        .bind(operation).bind(started).fetch_one(&mut **tx).await?;
    if !admissible {
        return Err(AppError::bad_request(
            "管理操作首次接收窗口已关闭，请先核查原操作结果",
        ));
    }
    if remote_exists {
        return Err(AppError::conflict("原操作编号已绑定另一管理请求"));
    }
    Ok(None)
}
#[cfg(test)]
pub(super) struct Record<'a> {
    pub operation: Uuid,
    pub digest: &'a str,
    pub action: &'a str,
    pub permission: &'a str,
    pub application: Option<Uuid>,
    pub request: Option<Uuid>,
    pub remote: bool,
    pub result: &'a Value,
}
#[cfg(test)]
pub(super) async fn record(
    tx: &mut Transaction<'_, Postgres>,
    user: &CurrentUser,
    headers: &HeaderMap,
    fact: Record<'_>,
) -> Result<(), AppError> {
    let Record {
        operation,
        digest,
        action,
        permission,
        application,
        request,
        remote,
        result,
    } = fact;
    sqlx::query("INSERT INTO platform_auth_review_operations(operation_id,actor_user_id,actor_session_id,request_digest,action,management_permission,application_id,request_id,remote_required,result_json) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(operation).bind(user.user_id).bind(user.session_id).bind(digest).bind(action).bind(permission).bind(application).bind(request).bind(remote).bind(result).execute(&mut **tx).await?;
    audit::write_in_tx(tx,audit::AuditInfo{user_id:Some(user.user_id),username:&user.username,tenant_id:Some(user.tenant_id),organization_id:Some(user.org_id),action:"AUTH_INTAKE_COMMAND",detail:&json!({"operationId":operation,"action":action,"applicationId":application,"requestId":request,"digest":digest}).to_string(),ip:&audit::client_ip(headers),user_agent:&audit::ua_of(headers)}).await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct LocalReceipt {
    actor_user_id: Uuid,
    actor_session_id: Uuid,
    management_permission: String,
    application_id: Option<Uuid>,
    remote_required: bool,
    result_json: Value,
    remote_result: Option<Value>,
}

pub(crate) async fn local_operation(
    state: &AppState,
    user: &CurrentUser,
    operation: Uuid,
) -> Result<Option<Value>, AppError> {
    let row:Option<LocalReceipt>=sqlx::query_as("SELECT actor_user_id,actor_session_id,management_permission,application_id,remote_required,result_json,remote_result FROM platform_auth_review_operations WHERE operation_id=$1")
        .bind(operation).fetch_optional(&state.pool).await?;
    let Some(LocalReceipt {
        actor_user_id: actor,
        actor_session_id: session,
        management_permission: permission,
        application_id: application,
        remote_required: remote,
        result_json: result,
        remote_result,
    }) = row
    else {
        return Ok(None);
    };
    if actor != user.user_id || session != user.session_id {
        return Err(AppError::forbidden(
            "原审核操作不属于当前管理员会话，请返回原会话核查",
        ));
    }
    let permission = known_permission(&permission)?;
    let mut tx = management_tx(state, user, permission, permission_is_recent(permission)).await?;
    if let Some(application) = application {
        require_application(&mut tx, user, application).await?;
    }
    tx.commit().await?;
    if remote && remote_result.is_none() {
        return Ok(None);
    }
    let mut result = result;
    if remote_result.is_some_and(|value| value["state"] == "NOT_APPLIED") {
        result["syncState"] = json!("NOT_APPLIED");
    }
    Ok(Some(
        json!({"operationId":operation,"state":"COMPLETED","result":result}),
    ))
}
/// Called only after the shared admission lock and the original receipt prove
/// the remote command cannot still apply. The local review decision is retained.
pub(crate) async fn local_not_applied(
    state: &AppState,
    user: &CurrentUser,
    operation: Uuid,
) -> Result<Option<Value>, AppError> {
    let row:Option<(Uuid,Uuid,String,Option<Uuid>,Value)>=sqlx::query_as("SELECT actor_user_id,actor_session_id,management_permission,application_id,result_json FROM platform_auth_review_operations WHERE operation_id=$1 AND remote_required")
        .bind(operation).fetch_optional(&state.pool).await?;
    let Some((actor, session, permission, application, mut result)) = row else {
        return Ok(None);
    };
    if actor != user.user_id || session != user.session_id {
        return Err(AppError::forbidden("原审核操作不属于当前管理员会话"));
    }
    let permission = known_permission(&permission)?;
    let mut tx = management_tx(state, user, permission, true).await?;
    if let Some(application) = application {
        require_application(&mut tx, user, application).await?;
    }
    result["syncState"] = json!("NOT_APPLIED");
    sqlx::query("UPDATE platform_auth_review_operations SET remote_result='{\"state\":\"NOT_APPLIED\"}'::jsonb,result_json=$2 WHERE operation_id=$1 AND remote_result IS NULL")
        .bind(operation).bind(&result).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Some(
        json!({"operationId":operation,"state":"COMPLETED","result":result}),
    ))
}
pub(crate) fn known_permission(permission: &str) -> Result<&'static str, AppError> {
    [
        policy::VIEW,
        policy::REGISTER,
        policy::REVIEW,
        policy::PUBLISH,
        policy::ROTATE,
        policy::DISABLE,
        policy::SCOPES,
        policy::INVITE,
        "system:oauth:keys",
        "system:oauth:assign",
    ]
    .into_iter()
    .find(|value| *value == permission)
    .ok_or_else(|| AppError::forbidden("原管理权限合同无法核验"))
}
pub(crate) async fn complete_remote(
    tx: &mut Transaction<'_, Postgres>,
    operation: Uuid,
    result: &Value,
) -> Result<(), AppError> {
    if result["state"] == "NOT_APPLIED" {
        sqlx::query("UPDATE platform_auth_review_operations SET remote_result=$2,result_json=jsonb_set(result_json,'{syncState}','\"NOT_APPLIED\"'::jsonb) WHERE operation_id=$1 AND remote_required AND remote_result IS NULL")
            .bind(operation).bind(result).execute(&mut **tx).await?;
        return Ok(());
    }
    let receipt:Option<(Option<Uuid>,Value)>=sqlx::query_as("SELECT request_id,result_json FROM platform_auth_review_operations WHERE operation_id=$1 AND remote_required FOR UPDATE")
        .bind(operation).fetch_optional(&mut **tx).await?;
    let request = if let Some((request, local)) = receipt {
        let completed = synchronized_result(local, result)?;
        sqlx::query("UPDATE platform_auth_review_operations SET remote_result=$2,result_json=$3,remote_applied_at=clock_timestamp() WHERE operation_id=$1")
            .bind(operation).bind(result).bind(completed).execute(&mut **tx).await?;
        request
    } else {
        None
    };
    if let Some(request) = request {
        if result["requestId"].as_str() != Some(request.to_string().as_str()) {
            return Err(AppError::Unavailable);
        }
        let version = result["requestVersion"]
            .as_i64()
            .ok_or(AppError::Unavailable)?;
        let projection = result["projectionVersion"]
            .as_i64()
            .ok_or(AppError::Unavailable)?;
        let admission = result["admissionVersion"]
            .as_i64()
            .ok_or(AppError::Unavailable)?;
        let changed=sqlx::query("UPDATE platform_auth_application_requests SET synced_version=$2,identity_projection_version=$3,identity_admission_version=$4,updated_at=clock_timestamp() WHERE id=$1 AND version=$2 AND remote_operation_id=$5")
            .bind(request).bind(version).bind(projection).bind(admission).bind(operation).execute(&mut **tx).await?.rows_affected();
        // A newer proposal may already exist. Preserve the original outcome
        // without replacing the new proposal's version or approval facts.
        if changed > 1 {
            return Err(AppError::Unavailable);
        }
    }
    Ok(())
}

// Preserve read-only recovery of commands accepted before administration was retired.
pub(super) fn synchronized_result(mut result: Value, remote: &Value) -> Result<Value, AppError> {
    if result["id"] != remote["requestId"] || result["version"] != remote["requestVersion"] {
        return Err(AppError::Unavailable);
    }
    result["syncedVersion"] = remote["requestVersion"].clone();
    result["identityProjectionVersion"] = remote["projectionVersion"].clone();
    result["identityAdmissionVersion"] = remote["admissionVersion"].clone();
    result["syncState"] = json!("COMPLETED");
    Ok(result)
}
