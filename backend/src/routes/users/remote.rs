//! Account commands accepted by the platform and completed by the identity owner.

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    authentication::{
        commands,
        remote::{CommandContext, Receipt},
    },
    authorization,
    error::AppError,
    extractors::CurrentUser,
    state::AppState,
    util::{audit, uuid7::ParseUuid7},
};

pub(super) async fn create(
    state: &AppState,
    headers: &HeaderMap,
    actor: &CurrentUser,
    username: &str,
    body: &super::UserBody,
    gender: &str,
) -> Result<Value, AppError> {
    let remote = state
        .remote_accounts
        .as_ref()
        .ok_or(AppError::Unavailable)?;
    let target = crate::util::uuid7::new_uuid();
    let command=remote.command(target,0,"CREATE",json!({"username":username,"realName":body.real_name,"email":body.email,"phone":body.phone,"gender":gender,"password":body.password}),None)?;
    let scope = state.scoped_org_ids(actor).await?;
    let welcome_title = if scope.is_some() {
        Some(format!(
            "欢迎加入 {}",
            crate::platform_settings::load_branding(&state.pool)
                .await?
                .platform_name
        ))
    } else {
        None
    };
    let mut tx = state.pool.begin().await?;
    super::require_employee_permission_in_tx(&mut tx, actor, "system:user").await?;
    super::require_scope_unchanged_in_tx(&mut tx, actor, scope.as_deref()).await?;
    let followup = json!({"kind":"USER_MUTATION","commandKind":"CREATE","action":"CREATE_USER","before":null,"targetTenants":[actor.tenant_id],"createMembership":scope.as_ref().map(|_|json!({"tenantId":actor.tenant_id,"organizationId":actor.org_id,"membershipId":crate::util::uuid7::new_uuid()})),"username":username,"welcomeTitle":welcome_title});
    commands::accept_in_tx(&mut tx, actor, headers, &command, followup).await?;
    tx.commit().await?;
    let receipt = commands::dispatch(state, &command).await?;
    commands::record_receipt(state, &receipt)
        .await
        .map_err(|_| pending(command.operation_id))?;
    finalize(state, command.operation_id)
        .await
        .map_err(|_| pending(command.operation_id))?;
    Ok(json!({"id":target,"username":username}))
}

pub(crate) async fn mutate(
    state: &AppState,
    headers: &HeaderMap,
    actor: &CurrentUser,
    target: Uuid,
    kind: &str,
    payload: Value,
    context: Option<CommandContext>,
) -> Result<Value, AppError> {
    let remote = state
        .remote_accounts
        .as_ref()
        .ok_or(AppError::Unavailable)?;
    let personal = kind == "CHANGE_PASSWORD";
    let kick = kind == "REVOKE_ALL";
    let scope = state.scoped_org_ids(actor).await?;
    let mut tx = state.pool.begin().await?;
    let tenants = if personal {
        if target != actor.user_id {
            return Err(AppError::forbidden("个人改密只能操作本人"));
        }
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
        crate::session::require_active_in_tx(
            &mut tx,
            actor.user_id,
            actor.tenant_id,
            actor.org_id,
            actor.session_id,
        )
        .await?;
        vec![actor.tenant_id]
    } else {
        let tenants = super::lock_global_user_mutation(state, actor, target, &mut tx).await?;
        super::require_employee_permission_in_tx(
            &mut tx,
            actor,
            if kick { "system:oauth" } else { "system:user" },
        )
        .await?;
        super::require_scope_unchanged_in_tx(&mut tx, actor, scope.as_deref()).await?;
        if kick {
            let platform:bool=sqlx::query_scalar("SELECT t.is_platform AND o.parent_id IS NULL FROM tenants t JOIN organizations o ON o.tenant_id=t.id WHERE t.id=$1 AND o.id=$2").bind(actor.tenant_id).bind(actor.org_id).fetch_one(&mut *tx).await?;
            let menu = authorization::context_menu_rows_in_tx(&mut tx, actor, None).await?;
            if !platform || !menu.iter().any(|row| row["path"] == "/system/oauth/tokens") {
                return Err(AppError::forbidden("没有会话管理权限"));
            }
        }
        tenants
    };
    let expected:i64=sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1 AND NOT EXISTS(SELECT 1 FROM identity_account_tombstones t WHERE t.user_id=users.id)").bind(target).fetch_optional(&mut *tx).await?.ok_or_else(||AppError::not_found("用户不存在"))?;
    let before = super::user_summary_in_tx(&mut tx, target).await?;
    let retire_members: Value = if kind == "RETIRE" {
        sqlx::query_scalar("SELECT COALESCE(jsonb_agg(jsonb_build_object('id',id,'identityVersion',identity_version)),'[]'::jsonb) FROM memberships WHERE user_id=$1")
            .bind(target).fetch_one(&mut *tx).await?
    } else {
        json!([])
    };
    let retire_roles: Value = if kind == "RETIRE" {
        sqlx::query_scalar("SELECT COALESCE(jsonb_agg(jsonb_build_object('roleId',role_id,'organizationId',organization_id,'createdAt',created_at)),'[]'::jsonb) FROM user_roles WHERE user_id=$1")
            .bind(target).fetch_one(&mut *tx).await?
    } else {
        json!([])
    };
    let context = if personal {
        Some(CommandContext {
            user_id: actor.user_id,
            session_id: actor.session_id,
            tenant_id: actor.tenant_id,
            organization_id: actor.org_id,
            account_version: expected.to_string(),
        })
    } else {
        context
    };
    let command = remote.command(target, expected, kind, payload, context)?;
    let action = match kind {
        "RETIRE" => "DELETE_USER",
        "RESET_PASSWORD" => "RESET_PASSWORD",
        "CHANGE_PASSWORD" => "UPDATE_PASSWORD",
        "REVOKE_ALL" => "KICK_SESSIONS",
        _ => "UPDATE_USER",
    };
    commands::accept_in_tx(&mut tx,actor,headers,&command,json!({"kind":"USER_MUTATION","commandKind":kind,"action":action,"before":before,"targetTenants":tenants,"createMembership":null,"retireMemberships":retire_members,"retireRoles":retire_roles})).await?;
    tx.commit().await?;
    let receipt = commands::dispatch(state, &command).await?;
    commands::record_receipt(state, &receipt)
        .await
        .map_err(|_| pending(command.operation_id))?;
    finalize(state, command.operation_id)
        .await
        .map_err(|_| pending(command.operation_id))?;
    if kick {
        Ok(
            json!({"revoked":receipt.revoked_tokens,"platformSessionsRevoked":receipt.revoked_sessions}),
        )
    } else {
        Ok(json!({"ok":true}))
    }
}

type IntentRow = (
    Uuid,
    Uuid,
    Uuid,
    Uuid,
    Uuid,
    String,
    String,
    Value,
    Option<Value>,
);

async fn finalize(state: &AppState, operation: Uuid) -> Result<Value, AppError> {
    let mut tx = state.pool.begin().await?;
    let row:IntentRow=sqlx::query_as("SELECT target_user_id,actor_user_id,actor_tenant_id,actor_organization_id,actor_session_id,state,canonical_digest,followup,receipt FROM platform_account_command_intents WHERE id=$1 FOR UPDATE")
        .bind(operation).fetch_optional(&mut *tx).await?.ok_or_else(||AppError::not_found("账号操作不存在"))?;
    let (target, actor, tenant, org, _sid, status, _digest, followup, receipt) = row;
    if followup["kind"] != "USER_MUTATION" {
        return Err(AppError::bad_request("此操作须通过原业务入口恢复"));
    }
    if status == "COMPLETED" {
        return Ok(json!({"ok":true,"operationId":operation}));
    }
    let receipt: Receipt =
        serde_json::from_value(receipt.ok_or_else(|| AppError::conflict("账号操作结果尚未确认"))?)
            .map_err(|_| AppError::Unavailable)?;
    let kind = followup["commandKind"]
        .as_str()
        .ok_or(AppError::Unavailable)?;
    let before_version: i64 = receipt
        .before_version
        .parse()
        .map_err(|_| AppError::Unavailable)?;
    if kind == "RETIRE" {
        sqlx::query("DELETE FROM user_roles ur USING jsonb_to_recordset($2::jsonb) AS accepted(\"roleId\" uuid,\"organizationId\" uuid,\"createdAt\" timestamptz) WHERE ur.user_id=$1 AND ur.role_id=accepted.\"roleId\" AND ur.organization_id=accepted.\"organizationId\" AND ur.created_at=accepted.\"createdAt\"")
            .bind(target)
            .bind(&followup["retireRoles"])
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM memberships m USING jsonb_to_recordset($2::jsonb) AS accepted(id uuid,\"identityVersion\" bigint) WHERE m.user_id=$1 AND m.id=accepted.id AND m.identity_version=accepted.\"identityVersion\"")
            .bind(target)
            .bind(&followup["retireMemberships"])
            .execute(&mut *tx)
            .await?;
    }
    let mut revoked = 0;
    if matches!(
        kind,
        "RESET_PASSWORD" | "CHANGE_PASSWORD" | "REVOKE_ALL" | "RETIRE"
    ) || (kind == "SET_STATUS" && receipt.after_version != receipt.before_version)
    {
        sqlx::query("UPDATE platform_sessions SET revoked_at=clock_timestamp() WHERE user_id=$1 AND account_version<=$2 AND revoked_at IS NULL").bind(target).bind(before_version).execute(&mut *tx).await?;
        revoked =
            sqlx::query("UPDATE oauth_tokens SET revoked=true WHERE user_id=$1 AND account_version<=$2 AND revoked=false")
                .bind(target)
                .bind(before_version)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        sqlx::query("UPDATE oauth_authorization_codes SET consumed=true WHERE user_id=$1 AND account_version<=$2 AND consumed=false").bind(target).bind(before_version).execute(&mut *tx).await?;
        sqlx::query("UPDATE oauth_device_codes SET status='EXPIRED' WHERE user_id=$1 AND account_version<=$2 AND status IN ('PENDING','APPROVED')").bind(target).bind(before_version).execute(&mut *tx).await?;
    }
    if let Some(membership) = followup["createMembership"].as_object() {
        let membership_id = parse_json_uuid(
            membership
                .get("membershipId")
                .ok_or(AppError::Unavailable)?,
        )?;
        let organization_id = parse_json_uuid(
            membership
                .get("organizationId")
                .ok_or(AppError::Unavailable)?,
        )?;
        sqlx::query("SELECT id FROM users WHERE id=$1 AND status='ENABLED' FOR UPDATE")
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
        let inserted=sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'EMPLOYEE')").bind(membership_id).bind(target).bind(organization_id).execute(&mut *tx).await?.rows_affected();
        if inserted != 1 {
            return Err(AppError::Unavailable);
        }
        if let Some(title) = followup["welcomeTitle"].as_str() {
            let tenant_id =
                parse_json_uuid(membership.get("tenantId").ok_or(AppError::Unavailable)?)?;
            let content = format!(
                "管理员为您创建了账号「{}」。首次登录后建议前往个人中心修改初始密码。",
                followup["username"].as_str().unwrap_or("")
            );
            crate::notifications::system_in_tx(
                &mut tx,
                &[(target, tenant_id, organization_id)],
                "INFO",
                title,
                &content,
                "/profile",
            )
            .await?;
            // This fresh membership ID has no earlier notification. Required
            // onboarding delivery may not silently take the optional helper's
            // skipped-recipient path or a trigger-suppressed INSERT path.
            let delivered:bool=sqlx::query_scalar(
                "SELECT count(*)=1 FROM notifications n JOIN memberships m ON m.id=n.recipient_membership_id \
                 WHERE m.id=$1 AND m.user_id=$2 AND m.organization_id=$4 AND m.member_type='EMPLOYEE' \
                 AND m.status='ENABLED' AND n.user_id=$2 AND n.tenant_id=$3 AND n.organization_id=$4 \
                 AND n.recipient_member_type='EMPLOYEE' AND n.recipient_identity_version=m.identity_version \
                 AND n.source_kind='SYSTEM' AND n.type='INFO' AND n.title=$5 AND n.content=$6 AND n.link='/profile'"
            ).bind(membership_id).bind(target).bind(tenant_id).bind(organization_id).bind(title).bind(&content)
                .fetch_one(&mut *tx).await?;
            if !delivered {
                return Err(AppError::Unavailable);
            }
        }
    }
    let mut after = if kind == "RETIRE" {
        Value::Null
    } else {
        super::user_summary_in_tx(&mut tx, target).await?
    };
    if matches!(kind, "RESET_PASSWORD" | "CHANGE_PASSWORD") {
        after["passwordChanged"] = json!(true);
        after["revokedSessions"] = json!(receipt.revoked_sessions);
        if kind == "RESET_PASSWORD" {
            let tenants: Vec<Uuid> = serde_json::from_value(followup["targetTenants"].clone())
                .map_err(|_| AppError::Unavailable)?;
            let contexts = super::system_recipient_contexts(&mut tx, target, &tenants).await?;
            crate::notifications::system_in_tx(
                &mut tx,
                &contexts,
                "SECURITY",
                "密码已被管理员重置",
                "管理员刚刚重置了您的登录密码。若非您本人申请，请立即联系平台管理员。",
                "/profile",
            )
            .await?;
        }
    }
    let detail = audit::change_detail("user", target, followup["before"].clone(), after);
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(actor),
            username: followup["actorUsername"].as_str().unwrap_or(""),
            tenant_id: Some(tenant),
            organization_id: Some(org),
            action: followup["action"].as_str().ok_or(AppError::Unavailable)?,
            detail: &detail,
            ip: followup["ip"].as_str().unwrap_or(""),
            user_agent: followup["userAgent"].as_str().unwrap_or(""),
        },
    )
    .await?;
    commands::complete_in_tx(&mut tx, operation).await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(
        json!({"ok":true,"operationId":operation,"revoked":receipt.revoked_tokens,"platformSessionsRevoked":receipt.revoked_sessions,"cleanupRevokedTokens":revoked}),
    )
}

fn parse_json_uuid(value: &Value) -> Result<Uuid, AppError> {
    value.as_str().ok_or(AppError::Unavailable)?.parse_uuid7()
}

fn pending(operation: Uuid) -> AppError {
    AppError::conflict(format!(
        "账号操作后续工作待恢复，操作编号 {operation}；请查询原操作结果"
    ))
}

async fn accessible(
    state: &AppState,
    user: &CurrentUser,
    operation: Uuid,
) -> Result<(Uuid, String, String, Option<Value>), AppError> {
    let row:Option<(Uuid,Uuid,String,String,Value,Option<Value>)>=sqlx::query_as("SELECT target_user_id,actor_user_id,state,canonical_digest,followup,receipt FROM platform_account_command_intents WHERE id=$1").bind(operation).fetch_optional(&state.pool).await?;
    let (target, actor, status, digest, followup, receipt) =
        row.ok_or_else(|| AppError::not_found("账号操作不存在"))?;
    if actor != user.user_id {
        return Err(AppError::not_found("账号操作不存在"));
    }
    if followup["kind"] != "USER_MUTATION" {
        return Err(AppError::bad_request("此操作须通过原业务入口恢复"));
    }
    if followup["commandKind"] == "CHANGE_PASSWORD" {
        if target != user.user_id {
            return Err(AppError::forbidden("个人操作只能本人查询"));
        }
    } else {
        state
            .require(
                user,
                if followup["commandKind"] == "REVOKE_ALL" {
                    "system:oauth"
                } else {
                    "system:user"
                },
            )
            .await?;
    }
    Ok((target, status, digest, receipt))
}

pub(super) async fn status(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let operation = id.parse_uuid7()?;
    let (_, status, _, receipt) = accessible(&state, &user, operation).await?;
    Ok(Json(
        json!({"operationId":operation,"state":status,"receipt":receipt}),
    ))
}

pub(super) async fn recover(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let operation = id.parse_uuid7()?;
    let (target, status, digest, receipt) = accessible(&state, &user, operation).await?;
    if status == "COMPLETED" {
        return Ok(Json(json!({"ok":true,"operationId":operation})));
    }
    if matches!(status.as_str(), "ABANDONED" | "REJECTED") {
        return Err(AppError::conflict("操作未执行，请明确发起新的操作"));
    }
    if receipt.is_none() {
        match commands::reconcile(&state, operation, target, &digest).await {
            Ok(receipt) => commands::record_receipt(&state, &receipt).await?,
            Err(error) => {
                sqlx::query("UPDATE platform_account_command_intents SET state='ABANDONED' WHERE id=$1 AND state='PENDING_SECRET' AND expires_at<=clock_timestamp()")
                    .bind(operation).execute(&state.pool).await?;
                return Err(error);
            }
        }
    }
    finalize(&state, operation).await.map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{anyhow, ensure, Result};
    use sqlx::{
        postgres::{PgConnectOptions, PgPoolOptions},
        PgPool,
    };
    use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

    struct Database {
        pool: PgPool,
        admin: PgPool,
        name: String,
    }
    impl Database {
        async fn create() -> Result<Self> {
            let raw = std::env::var("IDENTITY_PHASE12_TEST_DATABASE_URL")
                .map_err(|_| anyhow!("explicit IDENTITY_PHASE12_TEST_DATABASE_URL required"))?;
            let options = PgConnectOptions::from_str(&raw)
                .map_err(|_| anyhow!("invalid owned Phase12 database configuration"))?;
            ensure!(
                options.get_database() == Some("postgres"),
                "Phase12 administration must select postgres"
            );
            let admin = PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(10))
                .connect_with(options.clone())
                .await
                .map_err(|_| anyhow!("owned Phase12 administration connection failed"))?;
            let name = format!(
                "identity_phase12_accounts_{}",
                crate::util::uuid7::new_uuid().simple()
            );
            sqlx::query(&format!("CREATE DATABASE \"{name}\""))
                .execute(&admin)
                .await?;
            let pool = match PgPoolOptions::new()
                .max_connections(4)
                .acquire_timeout(Duration::from_secs(3))
                .connect_with(options.database(&name))
                .await
            {
                Ok(pool) => pool,
                Err(_) => {
                    let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                        .execute(&admin)
                        .await;
                    admin.close().await;
                    return Err(anyhow!("owned Phase12 database connection failed"));
                }
            };
            let db = Self { pool, admin, name };
            if let Err(error) = sqlx::migrate!("./migrations").run(&db.pool).await {
                let _ = db.finish(Ok(())).await;
                return Err(error.into());
            }
            Ok(db)
        }
        async fn finish(self, result: Result<()>) -> Result<()> {
            ensure!(
                self.name
                    .strip_prefix("identity_phase12_accounts_")
                    .is_some_and(|suffix| suffix.len() == 32
                        && Uuid::parse_str(suffix).is_ok_and(crate::util::uuid7::is_uuid_v7)),
                "refusing cleanup outside owned Phase12 account namespace"
            );
            let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
                .await
                .is_ok();
            let dropped = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
                .execute(&self.admin)
                .await;
            self.admin.close().await;
            eprintln!("identity_phase12_accounts_cleanup database={} assertions_ok={} removed={} pool_closed={closed}",self.name,result.is_ok(),dropped.is_ok());
            result?;
            dropped?;
            ensure!(closed, "owned Phase12 account pool did not close");
            Ok(())
        }
    }

    async fn fixture(pool: &PgPool) -> Result<(AppState, Uuid, Uuid, Uuid)> {
        let target = crate::util::uuid7::new_uuid();
        let tenant = crate::util::uuid7::new_uuid();
        let org = crate::util::uuid7::new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code) VALUES($1,'Owned Phase12 account followup','owned-phase12-accounts')").bind(tenant).execute(pool).await?;
        sqlx::query(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned Phase12 root')",
        )
        .bind(org)
        .bind(tenant)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO users(id,username,password_hash,session_version) VALUES($1,'owned-phase12-account','unusable-owned-fixture',1)").bind(target).execute(pool).await?;
        let config = crate::config::Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/phase12-account".into(),
            ),
            (
                "JWT_SECRET".into(),
                "owned-phase12-account-test-signing-secret".into(),
            ),
        ]))?;
        Ok((
            AppState::new(pool.clone(), Arc::new(config)),
            target,
            tenant,
            org,
        ))
    }

    async fn intent(
        pool: &PgPool,
        target: Uuid,
        tenant: Uuid,
        org: Uuid,
        kind: &str,
        extra: Value,
    ) -> Result<Uuid> {
        let operation = crate::util::uuid7::new_uuid();
        let digest = "a".repeat(64);
        let mut followup = json!({"kind":"USER_MUTATION","commandKind":kind,"action":"OWNED_PHASE12_FOLLOWUP","before":{"sessionVersion":0},"targetTenants":[tenant],"createMembership":null,"retireMemberships":[],"retireRoles":[],"actorUsername":"owned-phase12-account","ip":"192.0.2.12","userAgent":"owned-phase12-account-test"});
        for (key, value) in extra
            .as_object()
            .ok_or_else(|| anyhow!("expected safe followup object"))?
        {
            followup[key] = value.clone();
        }
        let receipt = json!({"operationId":operation,"targetUserId":target,"canonicalDigest":digest,"state":"COMPLETED","beforeVersion":"0","afterVersion":"1","revokedSessions":0,"revokedTokens":0});
        sqlx::query("INSERT INTO platform_account_command_intents(id,target_user_id,actor_user_id,actor_tenant_id,actor_organization_id,actor_session_id,expected_account_version,canonical_digest,command_kind,expires_at,state,followup,receipt,created_at) VALUES($1,$2,$2,$3,$4,$5,0,$6,$7,clock_timestamp()+interval '50 seconds','UNKNOWN',$8,$9,clock_timestamp())")
            .bind(operation).bind(target).bind(tenant).bind(org).bind(crate::util::uuid7::new_uuid()).bind(&digest).bind(kind).bind(followup).bind(receipt).execute(pool).await?;
        Ok(operation)
    }

    #[tokio::test]
    #[ignore = "requires explicit owned PostgreSQL administration database"]
    async fn account_command_acknowledgements_reject_trigger_suppressed_writes() -> Result<()> {
        let db = Database::create().await?;
        let result = async {
            let (state,target,tenant,org)=fixture(&db.pool).await?;
            let member=crate::util::uuid7::new_uuid();
            let session=crate::util::uuid7::new_uuid();
            sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,'EMPLOYEE','ENABLED',1)")
                .bind(member).bind(target).bind(org).execute(&db.pool).await?;
            sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) VALUES($1,$2,$3,$4,clock_timestamp()+interval '10 minutes',1,$5,'EMPLOYEE',1,'urn:saas:legacy-platform','platform-console-legacy',1)")
                .bind(session).bind(target).bind(tenant).bind(org).bind(member).execute(&db.pool).await?;
            let actor=CurrentUser{user_id:target,username:"owned-phase12-account".into(),real_name:String::new(),is_super_admin:false,tenant_id:tenant,org_id:org,session_id:session};
            let command=crate::authentication::remote::Command{
                operation_id:crate::util::uuid7::new_uuid(),target_user_id:target,
                expected_account_version:"1".into(),expires_at:chrono::Utc::now()+chrono::Duration::seconds(50),
                canonical_digest:"a".repeat(64),kind:"REVOKE_ALL".into(),payload:json!({}),context:None,
            };
            let mut tx=db.pool.begin().await?;
            commands::accept_in_tx(&mut tx,&actor,&HeaderMap::new(),&command,json!({"kind":"OWNED_ACK_TEST"})).await?;
            let inserted:i64=sqlx::query_scalar("SELECT count(*) FROM platform_account_command_intents WHERE id=$1")
                .bind(command.operation_id).fetch_one(&mut *tx).await?;
            ensure!(inserted==1,"baseline acceptance did not insert its intent");
            tx.rollback().await?;

            sqlx::query("CREATE FUNCTION owned_phase12_skip_account_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$")
                .execute(&db.pool).await?;
            sqlx::query("CREATE TRIGGER owned_phase12_skip_account_insert BEFORE INSERT ON platform_account_command_intents FOR EACH ROW EXECUTE FUNCTION owned_phase12_skip_account_write()")
                .execute(&db.pool).await?;
            let mut tx=db.pool.begin().await?;
            let accepted=commands::accept_in_tx(&mut tx,&actor,&HeaderMap::new(),&command,json!({"kind":"OWNED_ACK_TEST"})).await;
            ensure!(matches!(accepted,Err(AppError::Unavailable)),"suppressed intent insert was reported accepted");
            let absent:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM platform_account_command_intents WHERE id=$1)")
                .bind(command.operation_id).fetch_one(&mut *tx).await?;
            ensure!(absent,"suppressed acceptance unexpectedly retained an intent");
            tx.rollback().await?;
            sqlx::query("DROP TRIGGER owned_phase12_skip_account_insert ON platform_account_command_intents")
                .execute(&db.pool).await?;

            let operation=intent(&db.pool,target,tenant,org,"REVOKE_ALL",json!({})).await?;
            let receipt_value:Value=sqlx::query_scalar("SELECT receipt FROM platform_account_command_intents WHERE id=$1")
                .bind(operation).fetch_one(&db.pool).await?;
            let receipt:Receipt=serde_json::from_value(receipt_value)?;
            sqlx::query("CREATE TRIGGER owned_phase12_skip_account_update BEFORE UPDATE ON platform_account_command_intents FOR EACH ROW EXECUTE FUNCTION owned_phase12_skip_account_write()")
                .execute(&db.pool).await?;
            ensure!(commands::record_receipt(&state,&receipt).await.is_err(),"suppressed receipt update was acknowledged");
            let mut tx=db.pool.begin().await?;
            ensure!(commands::complete_in_tx(&mut tx,operation).await.is_err(),"suppressed completion update was acknowledged");
            tx.rollback().await?;
            ensure!(finalize(&state,operation).await.is_err(),"suppressed completion reported business followup success");
            let status:String=sqlx::query_scalar("SELECT state FROM platform_account_command_intents WHERE id=$1")
                .bind(operation).fetch_one(&db.pool).await?;
            let audits:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='OWNED_PHASE12_FOLLOWUP'")
                .fetch_one(&db.pool).await?;
            ensure!(status=="UNKNOWN" && audits==0,"failed completion did not roll back its audit and retain UNKNOWN");
            sqlx::query("DROP TRIGGER owned_phase12_skip_account_update ON platform_account_command_intents")
                .execute(&db.pool).await?;

            let employee_org=crate::util::uuid7::new_uuid();
            let employee_member=crate::util::uuid7::new_uuid();
            sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,'Owned Phase12 suppressed employee followup')")
                .bind(employee_org).bind(tenant).bind(org).execute(&db.pool).await?;
            let employee_operation=intent(&db.pool,target,tenant,employee_org,"CREATE",json!({"createMembership":{"tenantId":tenant,"organizationId":employee_org,"membershipId":employee_member},"welcomeTitle":"Owned Phase12 required welcome","username":"owned-phase12-account"})).await?;
            sqlx::query("CREATE TRIGGER owned_phase12_skip_employee_insert BEFORE INSERT ON memberships FOR EACH ROW EXECUTE FUNCTION owned_phase12_skip_account_write()")
                .execute(&db.pool).await?;
            ensure!(finalize(&state,employee_operation).await.is_err(),"suppressed employee insert reported completed user creation");
            let employee_status:String=sqlx::query_scalar("SELECT state FROM platform_account_command_intents WHERE id=$1")
                .bind(employee_operation).fetch_one(&db.pool).await?;
            let employee_absent:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM memberships WHERE id=$1)")
                .bind(employee_member).fetch_one(&db.pool).await?;
            let audits:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='OWNED_PHASE12_FOLLOWUP'")
                .fetch_one(&db.pool).await?;
            ensure!(employee_status=="UNKNOWN" && employee_absent && audits==0,"suppressed employee followup retained completion, member or audit");
            sqlx::query("DROP TRIGGER owned_phase12_skip_employee_insert ON memberships")
                .execute(&db.pool).await?;
            sqlx::query("CREATE TRIGGER owned_phase12_skip_welcome_insert BEFORE INSERT ON notifications FOR EACH ROW EXECUTE FUNCTION owned_phase12_skip_account_write()")
                .execute(&db.pool).await?;
            ensure!(finalize(&state,employee_operation).await.is_err(),"suppressed required welcome reported completed user creation");
            let welcome_failed:bool=sqlx::query_scalar("SELECT state='UNKNOWN' AND NOT EXISTS(SELECT 1 FROM memberships WHERE id=$2) AND NOT EXISTS(SELECT 1 FROM notifications WHERE recipient_membership_id=$2) AND NOT EXISTS(SELECT 1 FROM audit_logs WHERE action='OWNED_PHASE12_FOLLOWUP') FROM platform_account_command_intents WHERE id=$1")
                .bind(employee_operation).bind(employee_member).fetch_one(&db.pool).await?;
            ensure!(welcome_failed,"suppressed welcome did not roll back membership, audit and completion");
            sqlx::query("DROP TRIGGER owned_phase12_skip_welcome_insert ON notifications")
                .execute(&db.pool).await?;
            finalize(&state,employee_operation).await?;
            let recovered_employee:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM memberships WHERE id=$1 AND user_id=$2 AND organization_id=$3 AND member_type='EMPLOYEE' AND status='ENABLED')")
                .bind(employee_member).bind(target).bind(employee_org).fetch_one(&db.pool).await?;
            ensure!(recovered_employee,"same accepted operation did not recover its exact employee after trigger removal");
            finalize(&state,employee_operation).await?;
            let welcome_count:i64=sqlx::query_scalar("SELECT count(*) FROM notifications WHERE recipient_membership_id=$1 AND user_id=$2 AND tenant_id=$3 AND organization_id=$4 AND source_kind='SYSTEM' AND type='INFO' AND title='Owned Phase12 required welcome' AND link='/profile'")
                .bind(employee_member).bind(target).bind(tenant).bind(employee_org).fetch_one(&db.pool).await?;
            ensure!(welcome_count==1,"recovered completed user creation did not retain exactly one required welcome");

            let administrator_org=crate::util::uuid7::new_uuid();
            let administrator_role=crate::util::uuid7::new_uuid();
            sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,'Owned Phase12 suppressed role followup')")
                .bind(administrator_org).bind(tenant).bind(org).execute(&db.pool).await?;
            sqlx::query("INSERT INTO roles(id,tenant_id,organization_id,name,code,built_in) VALUES($1,$2,$3,'Owned Phase12 administrator','org_admin',true)")
                .bind(administrator_role).bind(tenant).bind(administrator_org).execute(&db.pool).await?;
            let administrator_operation=intent(&db.pool,target,tenant,administrator_org,"CREATE",json!({})).await?;
            let mut remote_config=(*state.config).clone();
            remote_config.identity_remote_enabled=true;
            sqlx::query("CREATE TRIGGER owned_phase12_skip_role_insert BEFORE INSERT ON user_roles FOR EACH ROW EXECUTE FUNCTION owned_phase12_skip_account_write()")
                .execute(&db.pool).await?;
            let failed_role:Result<(),AppError>=async {
                let mut tx=db.pool.begin().await?;
                crate::routes::onboarding::attach_contact_administrator(&mut tx,&remote_config,tenant,administrator_org,"org_admin",crate::routes::onboarding::ContactAccount{user_id:target,username:"owned-phase12-account".into(),created:true,initial_password:None}).await?;
                commands::complete_in_tx(&mut tx,administrator_operation).await?;
                tx.commit().await?;
                Ok(())
            }.await;
            ensure!(matches!(failed_role,Err(AppError::Unavailable)),"suppressed role assignment reported administrator initialization success");
            let administrator_status:String=sqlx::query_scalar("SELECT state FROM platform_account_command_intents WHERE id=$1")
                .bind(administrator_operation).fetch_one(&db.pool).await?;
            let administrator_empty:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM memberships WHERE user_id=$1 AND organization_id=$2) AND NOT EXISTS(SELECT 1 FROM user_roles WHERE user_id=$1 AND role_id=$3 AND organization_id=$2)")
                .bind(target).bind(administrator_org).bind(administrator_role).fetch_one(&db.pool).await?;
            ensure!(administrator_status=="UNKNOWN" && administrator_empty,"suppressed administrator followup did not roll back its member and completion");
            sqlx::query("DROP TRIGGER owned_phase12_skip_role_insert ON user_roles").execute(&db.pool).await?;
            let mut tx=db.pool.begin().await?;
            crate::routes::onboarding::attach_contact_administrator(&mut tx,&remote_config,tenant,administrator_org,"org_admin",crate::routes::onboarding::ContactAccount{user_id:target,username:"owned-phase12-account".into(),created:true,initial_password:None}).await?;
            tx.commit().await?;
            sqlx::query("CREATE TRIGGER owned_phase12_skip_role_insert BEFORE INSERT ON user_roles FOR EACH ROW EXECUTE FUNCTION owned_phase12_skip_account_write()")
                .execute(&db.pool).await?;
            let mut tx=db.pool.begin().await?;
            crate::routes::onboarding::attach_contact_administrator(&mut tx,&remote_config,tenant,administrator_org,"org_admin",crate::routes::onboarding::ContactAccount{user_id:target,username:"owned-phase12-account".into(),created:true,initial_password:None}).await?;
            commands::complete_in_tx(&mut tx,administrator_operation).await?;
            tx.commit().await?;
            let administrator_complete:bool=sqlx::query_scalar("SELECT state='COMPLETED' AND EXISTS(SELECT 1 FROM user_roles WHERE user_id=$2 AND role_id=$3 AND organization_id=$4) FROM platform_account_command_intents WHERE id=$1")
                .bind(administrator_operation).bind(target).bind(administrator_role).bind(administrator_org).fetch_one(&db.pool).await?;
            ensure!(administrator_complete,"existing exact role binding was mistaken for a suppressed missing assignment");
            Ok(())
        }.await;
        db.finish(result).await
    }

    #[tokio::test]
    #[ignore = "requires explicit owned PostgreSQL administration database"]
    async fn delayed_account_followup_does_not_revoke_new_generation_and_is_idempotent(
    ) -> Result<()> {
        let db = Database::create().await?;
        let result=async {
            let (state,target,tenant,org)=fixture(&db.pool).await?;
            let old=crate::util::uuid7::new_uuid();let new=crate::util::uuid7::new_uuid();
            let member=crate::util::uuid7::new_uuid();
            sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,'EMPLOYEE','ENABLED',1)").bind(member).bind(target).bind(org).execute(&db.pool).await?;
            for (id,version) in [(old,0_i64),(new,1_i64)] {
                sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) VALUES($1,$2,$3,$4,clock_timestamp()+interval '10 minutes',$5,$6,'EMPLOYEE',1,'urn:saas:legacy-platform','platform-console-legacy',1)").bind(id).bind(target).bind(tenant).bind(org).bind(version).bind(member).execute(&db.pool).await?;
            }
            let operation=intent(&db.pool,target,tenant,org,"REVOKE_ALL",json!({})).await?;
            finalize(&state,operation).await?;
            let rows:Vec<(Uuid,bool)>=sqlx::query_as("SELECT id,revoked_at IS NOT NULL FROM platform_sessions WHERE user_id=$1 ORDER BY account_version").bind(target).fetch_all(&db.pool).await?;
            ensure!(rows==vec![(old,true),(new,false)],"delayed old-generation followup revoked a new session");
            let active:bool=sqlx::query_scalar("SELECT public.platform_session_active($1)").bind(new).fetch_one(&db.pool).await?;
            ensure!(active,"new generation session lost its actual authority after delayed followup");
            finalize(&state,operation).await?;
            let audit:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='OWNED_PHASE12_FOLLOWUP'").fetch_one(&db.pool).await?;
            ensure!(audit==1,"repeated followup duplicated audit");
            Ok(())
        }.await;
        db.finish(result).await
    }

    #[tokio::test]
    #[ignore = "requires explicit owned PostgreSQL administration database"]
    async fn delayed_retirement_only_removes_accepted_membership_ids_and_versions() -> Result<()> {
        let db = Database::create().await?;
        let result=async {
            let (state,target,tenant,org)=fixture(&db.pool).await?;
            let other_org=crate::util::uuid7::new_uuid();let before=crate::util::uuid7::new_uuid();let later=crate::util::uuid7::new_uuid();
            sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,'Owned Phase12 later organization')").bind(other_org).bind(tenant).bind(org).execute(&db.pool).await?;
            for (id,organization) in [(before,org),(later,other_org)] {
                sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,'EMPLOYEE','ENABLED',1)").bind(id).bind(target).bind(organization).execute(&db.pool).await?;
            }
            let operation=intent(&db.pool,target,tenant,org,"RETIRE",json!({"retireMemberships":[{"id":before,"identityVersion":1}]})).await?;
            finalize(&state,operation).await?;
            let remaining:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM memberships WHERE user_id=$1").bind(target).fetch_all(&db.pool).await?;
            ensure!(remaining==vec![later],"retirement cleanup expanded to a later membership");
            Ok(())
        }.await;
        db.finish(result).await
    }
}
