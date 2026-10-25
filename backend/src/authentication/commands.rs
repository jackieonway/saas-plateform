//! Durable platform authorization acceptance and recoverable account follow-up.
//! A committed intent is the administrator authorization linearization point;
//! no platform lock is retained while an account HTTP request is in flight.

use axum::http::HeaderMap;
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::remote::{Command, Receipt};
use crate::{
    authorization, error::AppError, extractors::CurrentUser, state::AppState, util::audit,
};

/// Only non-secret follow-up fields belong here. Persisting a command payload,
/// PreparedPassword, password proof or a PHC string is deliberately impossible
/// through this function's command handling: it stores only explicit metadata.
pub(crate) async fn accept_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    headers: &HeaderMap,
    command: &Command,
    mut followup: Value,
) -> Result<(), AppError> {
    authorization::require_employee_context_in_tx(tx, actor).await?;
    let unexpired: bool = sqlx::query_scalar("SELECT $1::timestamptz>clock_timestamp()")
        .bind(command.expires_at)
        .fetch_one(&mut **tx)
        .await?;
    if !unexpired {
        return Err(AppError::conflict("账号操作已过期，请重新发起"));
    }
    if !followup.is_object() || contains_secret_field(&followup) {
        return Err(AppError::bad_request("账号后续操作包含禁止持久化的字段"));
    }
    followup["actorUsername"] = json!(actor.username);
    followup["ip"] = json!(audit::client_ip(headers));
    followup["userAgent"] = json!(audit::ua_of(headers));
    let inserted = sqlx::query(
        "INSERT INTO platform_account_command_intents \
         (id,target_user_id,actor_user_id,actor_tenant_id,actor_organization_id,actor_session_id, \
          expected_account_version,canonical_digest,command_kind,expires_at,state,followup,created_at) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'PENDING_SECRET',$11,clock_timestamp())",
    )
    .bind(command.operation_id).bind(command.target_user_id).bind(actor.user_id)
    .bind(actor.tenant_id).bind(actor.org_id).bind(actor.session_id)
    .bind(command.expected_account_version.parse::<i64>().map_err(|_| AppError::Unavailable)?)
    .bind(&command.canonical_digest).bind(&command.kind).bind(command.expires_at).bind(followup)
    .execute(&mut **tx).await?.rows_affected();
    // A BEFORE INSERT trigger may return NULL without raising a SQL error.
    // Only one durable intent can authorize the later, out-of-transaction send.
    if inserted != 1 {
        return Err(AppError::Unavailable);
    }
    Ok(())
}

/// The caller must commit acceptance before dispatching. UNKNOWN is durable
/// before sending bytes; recovery only queries the original operation/digest.
pub(crate) async fn dispatch(state: &AppState, command: &Command) -> Result<Receipt, AppError> {
    let remote = state
        .remote_accounts
        .as_ref()
        .ok_or(AppError::Unavailable)?;
    let marked = sqlx::query(
        "UPDATE platform_account_command_intents SET state='UNKNOWN' \
         WHERE id=$1 AND canonical_digest=$2 AND state='PENDING_SECRET' \
         AND expires_at>clock_timestamp()",
    )
    .bind(command.operation_id)
    .bind(&command.canonical_digest)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if marked != 1 {
        // A durable UNKNOWN operation is never submitted twice.
        return reconcile(
            state,
            command.operation_id,
            command.target_user_id,
            &command.canonical_digest,
        )
        .await;
    }
    match remote.execute(command).await {
        Ok(receipt) => Ok(receipt),
        Err(AppError::Unavailable) | Err(AppError::NotFound(_)) => {
            reconcile(
                state,
                command.operation_id,
                command.target_user_id,
                &command.canonical_digest,
            )
            .await
        }
        Err(error) => {
            let rejected=sqlx::query("UPDATE platform_account_command_intents SET state='REJECTED' WHERE id=$1 AND state='UNKNOWN'")
                .bind(command.operation_id).execute(&state.pool).await?.rows_affected();
            if rejected != 1 {
                return Err(AppError::conflict(format!(
                    "账号操作结果尚未持久确认，操作编号 {}；请查询原操作结果",
                    command.operation_id
                )));
            }
            Err(error)
        }
    }
}

pub(crate) async fn reconcile(
    state: &AppState,
    operation: Uuid,
    target: Uuid,
    digest: &str,
) -> Result<Receipt, AppError> {
    let remote = state
        .remote_accounts
        .as_ref()
        .ok_or(AppError::Unavailable)?;
    remote
        .receipt(operation, target, digest)
        .await
        .map_err(|_| {
            AppError::conflict(format!(
                "账号操作结果未知，操作编号 {operation}；请查询原操作结果"
            ))
        })
}

/// Persist a safe receipt before business follow-up so a platform crash between
/// the two commits is repairable without the original password.
pub(crate) async fn record_receipt(state: &AppState, receipt: &Receipt) -> Result<(), AppError> {
    let receipt_value = serde_json::to_value(receipt).map_err(|_| AppError::Unavailable)?;
    let changed = sqlx::query(
        "UPDATE platform_account_command_intents SET receipt=$4 \
         WHERE id=$1 AND target_user_id=$2 AND canonical_digest=$3 \
         AND state IN ('UNKNOWN','PENDING_SECRET') AND expected_account_version=$5",
    )
    .bind(receipt.operation_id)
    .bind(receipt.target_user_id)
    .bind(&receipt.canonical_digest)
    .bind(receipt_value)
    .bind(
        receipt
            .before_version
            .parse::<i64>()
            .map_err(|_| AppError::Unavailable)?,
    )
    .execute(&state.pool)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(AppError::conflict("账号操作已结束或已失效"));
    }
    Ok(())
}

pub(crate) async fn complete_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    operation: Uuid,
) -> Result<(), AppError> {
    let updated=sqlx::query("UPDATE platform_account_command_intents SET state='COMPLETED',completed_at=clock_timestamp() WHERE id=$1 AND receipt IS NOT NULL AND state='UNKNOWN'")
        .bind(operation).execute(&mut **tx).await?.rows_affected();
    if updated != 1 {
        return Err(AppError::conflict("账号操作后续工作已完成或状态不允许"));
    }
    Ok(())
}

fn contains_secret_field(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            let lower = key.to_ascii_lowercase();
            matches!(
                lower.as_str(),
                "password"
                    | "newpassword"
                    | "oldpassword"
                    | "initialpassword"
                    | "passwordhash"
                    | "password_hash"
                    | "preparedpassword"
                    | "secret"
                    | "payload"
                    | "token"
            ) || contains_secret_field(value)
        }),
        Value::Array(values) => values.iter().any(contains_secret_field),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intent_rejects_nested_secret_fields_and_accepts_safe_change_metadata() {
        for value in [
            json!({"newPassword":"secret"}),
            json!({"profile":{"password_hash":"phc"}}),
            json!({"items":[{"initialPassword":"x"}]}),
            json!({"payload":{}}),
        ] {
            assert!(contains_secret_field(&value));
        }
        assert!(!contains_secret_field(
            &json!({"action":"RESET_PASSWORD","passwordChanged":true,"before":{"sessionVersion":0},"targetTenants":[]})
        ));
    }
}
