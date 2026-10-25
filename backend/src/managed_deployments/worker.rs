use super::{audit, cleanup_protected, config, lock};
use crate::util::uuid7::ParseUuid7;
use crate::{error::AppError, extractors::ApiJson, state::AppState};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Utc};
use deployment_contract::{container_name, revision, route_config, sha256, Receipt, Work};
use serde_json::{json, Value};
use uuid::Uuid;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/deployment-agent/{target}/claim", post(claim))
        .route(
            "/api/deployment-agent/{target}/jobs/{job}/receipt",
            post(receipt),
        )
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    artifact_id: Uuid,
    release_id: Uuid,
    application_id: Uuid,
    application_code: String,
    environment: String,
    version: String,
    target_id: String,
    template_id: String,
    image_ref: String,
    policy_revision: String,
    operation: String,
    state: String,
    attempt: i32,
    nonce: Option<Uuid>,
    lease_until: Option<DateTime<Utc>>,
    receipt: Option<Value>,
}
const SELECT:&str="SELECT j.id,j.artifact_id,j.release_id,a.application_id,apps.code AS application_code,r.environment,a.version,a.target_id,a.template_id,a.image_ref,a.policy_revision,j.operation,j.state,j.attempt,j.nonce,j.lease_until,j.receipt FROM managed_deployment_jobs j JOIN managed_image_artifacts a ON a.id=j.artifact_id JOIN application_releases r ON r.id=j.release_id JOIN applications apps ON apps.id=a.application_id";
impl Row {
    fn work(&self) -> Work {
        Work {
            job_id: self.id.to_string(),
            release_id: self.release_id.to_string(),
            application_code: self.application_code.clone(),
            environment: self.environment.clone(),
            version: self.version.clone(),
            target_id: self.target_id.clone(),
            template_id: self.template_id.clone(),
            image_ref: self.image_ref.clone(),
            policy_revision: self.policy_revision.clone(),
            operation: self.operation.clone(),
            attempt: self.attempt,
            nonce: self.nonce.map(|v| v.to_string()).unwrap_or_default(),
        }
    }

    fn claim_work(
        &mut self,
        policy: &deployment_contract::Target,
        protected_cleanup: bool,
    ) -> Result<Work, &'static str> {
        if self.policy_revision != revision(policy) || protected_cleanup {
            return Err("POLICY_REJECTED");
        }
        let attempt = self
            .attempt
            .checked_add(1)
            .filter(|value| *value <= 1000)
            .ok_or("ATTEMPTS_EXHAUSTED")?;
        let mut work = self.work();
        let nonce = crate::util::uuid7::new_uuid();
        work.attempt = attempt;
        work.nonce = nonce.to_string();
        if !deployment_contract::work_allowed(&work, policy) {
            return Err("POLICY_REJECTED");
        }
        self.attempt = attempt;
        self.nonce = Some(nonce);
        Ok(work)
    }
}

async fn claim(
    State(state): State<AppState>,
    Path(target): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let config = config::load()?;
    let allowed = config::authenticate(&config, &target, &headers)?;
    let candidate:Option<(Uuid,Uuid)>=sqlx::query_as("SELECT j.id,a.application_id FROM managed_deployment_jobs j JOIN managed_image_artifacts a ON a.id=j.artifact_id WHERE a.target_id=$1 AND (j.state='QUEUED' OR (j.state='RUNNING' AND j.lease_until<now())) ORDER BY j.created_at,j.id LIMIT 1")
        .bind(&target).fetch_optional(&state.pool).await?;
    let Some((id, app)) = candidate else {
        return Ok(Json(Value::Null));
    };
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, app).await?;
    let mut row: Row = sqlx::query_as(&format!("{SELECT} WHERE j.id=$1 FOR UPDATE OF j,a"))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if row.application_id != app || row.target_id != target {
        return Err(AppError::not_found("任务不存在"));
    }
    let claimable:bool=sqlx::query_scalar("SELECT state='QUEUED' OR (state='RUNNING' AND lease_until<now()) FROM managed_deployment_jobs WHERE id=$1").bind(id).fetch_one(&mut *tx).await?;
    if !claimable {
        return Ok(Json(Value::Null));
    }
    let invalid_policy = row.policy_revision != revision(&allowed.policy);
    let protected_cleanup = row.operation == "CLEANUP"
        && cleanup_protected(&mut tx, row.artifact_id, row.release_id, Some(row.id)).await?;
    let work = match row.claim_work(&allowed.policy, protected_cleanup) {
        Ok(work) => work,
        Err(code) => {
            // End an unexecutable queue head so that other versions can still progress.
            // CLEANING remains fenced: an expired worker might have touched the engine.
            sqlx::query("UPDATE managed_deployment_jobs SET state='FAILED',error_code=$2,lease_until=NULL,updated_at=now() WHERE id=$1")
            .bind(id).bind(code).execute(&mut *tx).await?;
            audit(&mut tx, app, None, "MANAGED_DEPLOYMENT_REJECT", &json!({"jobId":id,"targetId":target,"policyChanged":invalid_policy,"protectedCleanup":protected_cleanup,"errorCode":code})).await?;
            tx.commit().await?;
            return Ok(Json(Value::Null));
        }
    };
    sqlx::query("UPDATE managed_deployment_jobs SET state='RUNNING',attempt=$2,nonce=$3,lease_until=now()+interval '10 minutes',receipt=NULL,error_code=NULL,updated_at=now() WHERE id=$1")
        .bind(id).bind(row.attempt).bind(row.nonce).execute(&mut *tx).await?;
    audit(
        &mut tx,
        app,
        None,
        "MANAGED_DEPLOYMENT_CLAIM",
        &json!({"jobId":id,"targetId":target,"attempt":row.attempt}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!(work)))
}

async fn receipt(
    State(state): State<AppState>,
    Path((target, job)): Path<(String, String)>,
    headers: HeaderMap,
    ApiJson(receipt): ApiJson<Receipt>,
) -> Result<Json<Value>, AppError> {
    let config = config::load()?;
    let allowed = config::authenticate(&config, &target, &headers)?;
    let id: Uuid = job
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("任务ID无效"))?;
    let app:Option<Uuid>=sqlx::query_scalar("SELECT a.application_id FROM managed_deployment_jobs j JOIN managed_image_artifacts a ON a.id=j.artifact_id WHERE j.id=$1 AND a.target_id=$2").bind(id).bind(&target).fetch_optional(&state.pool).await?;
    let app = app.ok_or_else(|| AppError::not_found("任务不存在"))?;
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, app).await?;
    let row: Row = sqlx::query_as(&format!("{SELECT} WHERE j.id=$1 FOR UPDATE OF j,a"))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    let value = serde_json::to_value(&receipt).map_err(|_| AppError::bad_request("回执无效"))?;
    if row.nonce.map(|v| v.to_string()).as_deref() != Some(&receipt.nonce)
        || receipt.attempt != row.attempt
        || row.policy_revision != revision(&allowed.policy)
    {
        return Err(AppError::conflict("过期尝试或策略回执"));
    }
    if row.state != "RUNNING" {
        return if row.receipt == Some(value) {
            Ok(Json(json!({"reused":true})))
        } else {
            Err(AppError::conflict("任务已结束且回执不一致"))
        };
    }
    if row.lease_until.is_none_or(|v| v <= Utc::now()) {
        return Err(AppError::conflict("任务租约已过期"));
    }
    let work = row.work();
    if receipt.container_name
        != container_name(
            &target,
            &work.application_code,
            &work.version,
            &work.image_ref,
        )
    {
        return Err(AppError::bad_request("回执容器身份不一致"));
    }
    let complete = if row.operation == "DEPLOY" {
        "READY"
    } else {
        "CLEANED"
    };
    if receipt.state != complete && receipt.state != "FAILED" {
        return Err(AppError::bad_request("回执状态无效"));
    }
    if receipt.state == "READY" {
        let image = receipt.image_id.as_deref().unwrap_or_default();
        if !image.starts_with("sha256:")
            || image.len() != 71
            || !image[7..]
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            || receipt.route_sha256 != Some(sha256(route_config(&work).as_bytes()))
            || receipt.error_code.is_some()
        {
            return Err(AppError::bad_request("就绪回执缺少镜像/路由证据"));
        }
        sqlx::query("UPDATE managed_image_artifacts SET state='READY',ever_ready=true,image_id=$2,container_name=$3,route_sha256=$4 WHERE id=$1")
            .bind(row.artifact_id).bind(&receipt.image_id).bind(&receipt.container_name).bind(&receipt.route_sha256).execute(&mut *tx).await?;
    } else if receipt.state == "CLEANED" {
        if receipt.image_id.is_some()
            || receipt.route_sha256.is_some()
            || receipt.error_code.is_some()
        {
            return Err(AppError::bad_request("清理回执不允许额外证据字段"));
        }
        if cleanup_protected(&mut tx, row.artifact_id, row.release_id, Some(row.id)).await? {
            return Err(AppError::conflict("清理期间出现受保护引用"));
        }
        sqlx::query("UPDATE managed_image_artifacts SET state='REMOVED' WHERE id=$1")
            .bind(row.artifact_id)
            .execute(&mut *tx)
            .await?;
    } else {
        if receipt.image_id.is_some() || receipt.route_sha256.is_some() {
            return Err(AppError::bad_request("失败回执不允许成功证据字段"));
        }
        let code = receipt.error_code.as_deref().unwrap_or_default();
        if ![
            "POLICY_REJECTED",
            "ENGINE_FAILED",
            "IMAGE_UNAVAILABLE",
            "UNHEALTHY",
            "ROUTE_FAILED",
            "OWNERSHIP_CONFLICT",
            "CLEANUP_FAILED",
            "ARTIFACT_MISSING",
        ]
        .contains(&code)
        {
            return Err(AppError::bad_request("错误回执代码无效"));
        }
    }
    sqlx::query("UPDATE managed_deployment_jobs SET state=$2,receipt=$3,error_code=$4,lease_until=NULL,updated_at=now() WHERE id=$1")
        .bind(id).bind(&receipt.state).bind(value).bind(&receipt.error_code).execute(&mut *tx).await?;
    audit(&mut tx,app,None,"MANAGED_DEPLOYMENT_RECEIPT",&json!({"jobId":id,"targetId":target,"state":receipt.state,"attempt":receipt.attempt,"errorCode":receipt.error_code})).await?;
    tx.commit().await?;
    Ok(Json(json!({"reused":false})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use deployment_contract::{Target, Template};

    fn fixture() -> (Row, Target) {
        let policy = Target {
            id: "edge".into(),
            name: "Edge".into(),
            public_origin: "https://edge.test".into(),
            repositories: vec!["registry.test/demo".into()],
            templates: vec![Template {
                id: "static".into(),
                application_code: "demo".into(),
                health_path: "/healthz".into(),
            }],
        };
        let row = Row {
            id: crate::util::uuid7::new_uuid(),
            artifact_id: crate::util::uuid7::new_uuid(),
            release_id: crate::util::uuid7::new_uuid(),
            application_id: crate::util::uuid7::new_uuid(),
            application_code: "demo".into(),
            environment: "development".into(),
            version: "v1".into(),
            target_id: policy.id.clone(),
            template_id: "static".into(),
            image_ref: format!("registry.test/demo@sha256:{}", "a".repeat(64)),
            policy_revision: revision(&policy),
            operation: "DEPLOY".into(),
            state: "QUEUED".into(),
            attempt: 0,
            nonce: None,
            lease_until: None,
            receipt: None,
        };
        (row, policy)
    }

    #[test]
    fn exhausted_or_invalid_jobs_are_terminal_rejections_without_mutating_the_lease() {
        for attempt in [1000, i32::MAX] {
            let (mut row, policy) = fixture();
            row.attempt = attempt;
            assert_eq!(
                row.claim_work(&policy, false).unwrap_err(),
                "ATTEMPTS_EXHAUSTED"
            );
            assert_eq!(row.attempt, attempt);
            assert!(row.nonce.is_none());
        }
        let (mut row, policy) = fixture();
        row.image_ref = "outside.test/demo:latest".into();
        assert_eq!(
            row.claim_work(&policy, false).unwrap_err(),
            "POLICY_REJECTED"
        );
        assert_eq!(row.attempt, 0);
        assert!(row.nonce.is_none());
    }

    #[test]
    fn last_allowed_claim_has_a_fenced_nonce_and_preserves_work_identity() {
        let (mut row, policy) = fixture();
        row.attempt = 999;
        let work = row.claim_work(&policy, false).unwrap();
        assert_eq!(work.attempt, 1000);
        assert_eq!(work.job_id, row.id.to_string());
        assert_eq!(work.nonce, row.nonce.unwrap().to_string());
        assert!(deployment_contract::work_allowed(&work, &policy));
        assert_eq!(
            row.claim_work(&policy, false).unwrap_err(),
            "ATTEMPTS_EXHAUSTED"
        );
    }
}
