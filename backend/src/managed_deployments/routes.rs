use super::{audit, cleanup_protected, config, lock};
use crate::{
    error::AppError,
    extractors::{ApiJson, CurrentUser, RequirePermission},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use deployment_contract::{image_allowed, revision, version_path, Intent};
use serde_json::{json, Value};
use uuid::Uuid;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/deployment-targets", get(targets))
        .route(
            "/api/applications/{app}/managed-deployments",
            get(list).post(create),
        )
        .route(
            "/api/applications/{app}/managed-deployments/{job}/retry",
            post(retry),
        )
        .route(
            "/api/applications/{app}/managed-deployments/{job}/cleanup",
            post(cleanup),
        )
        .merge(super::worker::routes())
}

fn uuid(s: &str) -> Result<Uuid, AppError> {
    s.parse().map_err(|_| AppError::bad_request("ID格式错误"))
}
async fn authorize(state: &AppState, user: &CurrentUser, write: bool) -> Result<(), AppError> {
    state.require_platform(user).await?;
    RequirePermission(if write {
        "system:deployment:write"
    } else {
        "system:deployment:read"
    })
    .check(state, user)
    .await
}

async fn targets(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user, false).await?;
    Ok(Json(json!(config::load()?
        .targets
        .into_iter()
        .map(|t| t.policy)
        .collect::<Vec<_>>())))
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(app): Path<String>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user, false).await?;
    let rows:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',j.id,'releaseId',j.release_id,'version',a.version,'environment',r.environment,'targetId',a.target_id,'templateId',a.template_id,'imageRef',a.image_ref,'publicUrl',a.public_url,'operation',j.operation,'state',j.state,'attempt',j.attempt,'errorCode',j.error_code,'createdAt',j.created_at,'updatedAt',j.updated_at) FROM managed_deployment_jobs j JOIN managed_image_artifacts a ON a.id=j.artifact_id JOIN application_releases r ON r.id=j.release_id WHERE a.application_id=$1 ORDER BY j.created_at DESC,j.id LIMIT 100")
        .bind(uuid(&app)?).fetch_all(&state.pool).await?;
    Ok(Json(json!(rows)))
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(app): Path<String>,
    ApiJson(intent): ApiJson<Intent>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user, true).await?;
    let app = uuid(&app)?;
    let release = uuid(&intent.release_id)?;
    let key = uuid(&intent.request_id)?;
    let config = config::load()?;
    let target = config
        .targets
        .iter()
        .find(|t| t.policy.id == intent.target_id)
        .ok_or_else(|| AppError::bad_request("部署目标不在白名单"))?;
    if !image_allowed(&intent.image_ref, &target.policy.repositories) {
        return Err(AppError::bad_request(
            "镜像必须来自可信仓库并使用固定sha256摘要",
        ));
    }
    let payload = deployment_contract::sha256(
        serde_json::to_string(&intent)
            .map_err(|_| AppError::bad_request("部署意图无效"))?
            .as_bytes(),
    );
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, app).await?;
    let existing:Option<(Uuid,String,Uuid)>=sqlx::query_as("SELECT j.id,j.payload_sha256,a.application_id FROM managed_deployment_jobs j JOIN managed_image_artifacts a ON a.id=j.artifact_id WHERE j.request_id=$1").bind(key).fetch_optional(&mut *tx).await?;
    if let Some((id, hash, owner)) = existing {
        if hash != payload || owner != app {
            return Err(AppError::conflict("请求标识已绑定不同部署意图"));
        }
        return Ok(Json(json!({"id":id,"reused":true})));
    }
    let row:Option<(String,String,String,String)>=sqlx::query_as("SELECT a.code,r.version,r.entry_url,r.entry_type FROM applications a JOIN application_releases r ON r.application_id=a.id WHERE a.id=$1 AND r.id=$2 FOR UPDATE OF r")
        .bind(app).bind(release).fetch_optional(&mut *tx).await?;
    let (code, version, entry, entry_type) =
        row.ok_or_else(|| AppError::not_found("发布版本不存在"))?;
    if !deployment_contract::segment(&code, 48)
        || !target
            .policy
            .templates
            .iter()
            .any(|t| t.id == intent.template_id && t.application_code == code)
    {
        return Err(AppError::bad_request("模板不允许部署该应用"));
    }
    let public = format!(
        "{}{}",
        target.policy.public_origin,
        version_path(&code, &version)
    );
    if entry != public || entry_type != "HTML" {
        return Err(AppError::bad_request(format!(
            "该模板只允许固定HTML版本入口：{public}"
        )));
    }
    let rev = revision(&target.policy);
    let foreign_url: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_image_artifacts WHERE public_url=$1 AND (application_id<>$2 OR target_id<>$3))")
        .bind(&public).bind(app).bind(&intent.target_id).fetch_one(&mut *tx).await?;
    if foreign_url {
        return Err(AppError::conflict("公开版本入口已绑定其他目标"));
    }
    sqlx::query("INSERT INTO managed_image_artifacts(application_id,version,target_id,template_id,image_ref,policy_revision,public_url) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(application_id,version,target_id) DO NOTHING")
        .bind(app).bind(&version).bind(&intent.target_id).bind(&intent.template_id).bind(&intent.image_ref).bind(&rev).bind(&public).execute(&mut *tx).await?;
    let (artifact,image,template,policy,status):(Uuid,String,String,String,String)=sqlx::query_as("SELECT id,image_ref,template_id,policy_revision,state FROM managed_image_artifacts WHERE application_id=$1 AND version=$2 AND target_id=$3 FOR UPDATE")
        .bind(app).bind(&version).bind(&intent.target_id).fetch_one(&mut *tx).await?;
    if image != intent.image_ref
        || template != intent.template_id
        || policy != rev
        || status == "CLEANING"
    {
        return Err(AppError::conflict(
            "同一版本不能替换镜像/策略，或版本正在清理",
        ));
    }
    let bound: Option<Uuid> = sqlx::query_scalar(
        "SELECT artifact_id FROM managed_deployment_bindings WHERE release_id=$1",
    )
    .bind(release)
    .fetch_optional(&mut *tx)
    .await?;
    if bound.is_some_and(|id| id != artifact) {
        return Err(AppError::conflict("发布已绑定另一部署产物"));
    }
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_deployment_jobs WHERE artifact_id=$1 AND state IN ('QUEUED','RUNNING'))").bind(artifact).fetch_one(&mut *tx).await?;
    if pending {
        return Err(AppError::conflict("该版本已有在途任务，请读取现有任务"));
    }
    sqlx::query("INSERT INTO managed_deployment_bindings(release_id,artifact_id) VALUES($1,$2) ON CONFLICT(release_id) DO NOTHING").bind(release).bind(artifact).execute(&mut *tx).await?;
    let job:Uuid=sqlx::query_scalar("INSERT INTO managed_deployment_jobs(artifact_id,release_id,request_id,payload_sha256,operation,created_by) VALUES($1,$2,$3,$4,'DEPLOY',$5) RETURNING id")
        .bind(artifact).bind(release).bind(key).bind(payload).bind(user.user_id).fetch_one(&mut *tx).await?;
    audit(&mut tx,app,Some(&user),"MANAGED_DEPLOYMENT_CREATE",&json!({"jobId":job,"releaseId":release,"targetId":intent.target_id,"imageRef":intent.image_ref})).await?;
    tx.commit().await?;
    Ok(Json(json!({"id":job,"reused":false})))
}

async fn retry(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((app, job)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user, true).await?;
    let app = uuid(&app)?;
    let job = uuid(&job)?;
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, app).await?;
    let row:Option<(Uuid,Uuid,String,String)>=sqlx::query_as("SELECT j.artifact_id,j.release_id,j.state,j.operation FROM managed_deployment_jobs j JOIN managed_image_artifacts a ON a.id=j.artifact_id WHERE j.id=$1 AND a.application_id=$2 FOR UPDATE OF j,a")
        .bind(job).bind(app).fetch_optional(&mut *tx).await?;
    let (artifact, release, status, operation) =
        row.ok_or_else(|| AppError::not_found("任务不存在"))?;
    if status != "FAILED" {
        return Err(AppError::conflict(
            "仅失败任务可重试；在途或完成任务请读取状态",
        ));
    }
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_deployment_jobs WHERE artifact_id=$1 AND state IN ('QUEUED','RUNNING'))").bind(artifact).fetch_one(&mut *tx).await?;
    if pending {
        return Err(AppError::conflict("该版本已有在途任务"));
    }
    if operation == "CLEANUP" && cleanup_protected(&mut tx, artifact, release, Some(job)).await? {
        return Err(AppError::conflict("版本仍有受保护引用，不能清理"));
    }
    if operation == "CLEANUP" {
        sqlx::query("UPDATE managed_image_artifacts SET state='CLEANING' WHERE id=$1")
            .bind(artifact)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE managed_deployment_jobs SET state='QUEUED',nonce=NULL,lease_until=NULL,receipt=NULL,error_code=NULL,updated_at=now() WHERE id=$1").bind(job).execute(&mut *tx).await?;
    audit(
        &mut tx,
        app,
        Some(&user),
        "MANAGED_DEPLOYMENT_RETRY",
        &json!({"jobId":job}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":job})))
}

async fn cleanup(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((app, job)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user, true).await?;
    let app = uuid(&app)?;
    let job = uuid(&job)?;
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, app).await?;
    let row:Option<(Uuid,Uuid,String)>=sqlx::query_as("SELECT j.artifact_id,j.release_id,a.state FROM managed_deployment_jobs j JOIN managed_image_artifacts a ON a.id=j.artifact_id WHERE j.id=$1 AND a.application_id=$2 FOR UPDATE OF a")
        .bind(job).bind(app).fetch_optional(&mut *tx).await?;
    let (artifact, release, status) = row.ok_or_else(|| AppError::not_found("任务不存在"))?;
    if status == "REMOVED" {
        return Ok(Json(json!({"removed":true})));
    }
    if status == "CLEANING" || cleanup_protected(&mut tx, artifact, release, None).await? {
        return Err(AppError::conflict(
            "版本被active/previous、其他环境、在途任务或保留窗口保护",
        ));
    }
    let id:Uuid=sqlx::query_scalar("INSERT INTO managed_deployment_jobs(artifact_id,release_id,request_id,payload_sha256,operation,created_by) VALUES($1,$2,gen_random_uuid(),'cleanup','CLEANUP',$3) RETURNING id")
        .bind(artifact).bind(release).bind(user.user_id).fetch_one(&mut *tx).await?;
    sqlx::query("UPDATE managed_image_artifacts SET state='CLEANING' WHERE id=$1")
        .bind(artifact)
        .execute(&mut *tx)
        .await?;
    audit(
        &mut tx,
        app,
        Some(&user),
        "MANAGED_DEPLOYMENT_CLEANUP",
        &json!({"jobId":id,"artifactId":artifact}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id})))
}
