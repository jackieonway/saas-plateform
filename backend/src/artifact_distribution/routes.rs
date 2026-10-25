use super::{audit, authorize, config, gates, lock, uuid};
use crate::{
    error::AppError,
    extractors::{ApiJson, CurrentUser},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use deployment_contract::{
    sha256,
    zip::{self, Intent, Manifest},
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/distribution-targets", get(targets))
        .route(
            "/api/applications/{app}/zip-distributions",
            get(list).post(create),
        )
        .route(
            "/api/applications/{app}/zip-distributions/{job}/retry",
            post(retry),
        )
        .route(
            "/api/applications/{app}/zip-distributions/{job}/verify",
            post(verify),
        )
        .route("/api/zip-distribution/route-ready", get(route_ready))
}
async fn targets(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user, false).await?;
    Ok(Json(json!(config::load(&state)?
        .groups
        .into_iter()
        .map(|g| g.policy)
        .collect::<Vec<_>>())))
}
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(app): Path<String>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user, false).await?;
    let app = uuid(&app)?;
    let jobs:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',j.id,'releaseId',j.release_id,'version',r.version,'environment',r.environment,'groupId',j.group_id,'artifactDigest',j.artifact_digest,'state',j.state,'attempt',j.attempt,'errorCode',j.error_code,'verifiedAt',j.verified_at,'updatedAt',j.updated_at,'nodes',COALESCE((SELECT jsonb_agg(jsonb_build_object('id',n.node_id,'state',n.state,'attempt',n.attempt,'errorCode',n.error_code,'verifiedAt',n.verified_at) ORDER BY n.node_id) FROM zip_distribution_nodes n WHERE n.job_id=j.id),'[]'::jsonb)) FROM zip_distribution_jobs j JOIN application_releases r ON r.id=j.release_id WHERE j.application_id=$1 ORDER BY j.created_at DESC,j.id LIMIT 100").bind(app).fetch_all(&state.pool).await?;
    let candidates:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('releaseId',id,'version',version,'environment',environment,'artifactDigest',artifact_digest,'entryUrl',entry_url) FROM application_releases WHERE application_id=$1 AND entry_type='HTML' AND artifact_digest IS NOT NULL ORDER BY created_at DESC,id LIMIT 100").bind(app).fetch_all(&state.pool).await?;
    Ok(Json(json!({"jobs":jobs,"candidates":candidates})))
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
    let cfg = config::load(&state)?;
    let group = cfg
        .groups
        .iter()
        .find(|g| g.policy.id == intent.group_id)
        .ok_or_else(|| AppError::bad_request("分发目标不在白名单"))?;
    let hash =
        sha256(&serde_json::to_vec(&intent).map_err(|_| AppError::bad_request("分发意图无效"))?);
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, app).await?;
    let existing: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT id,application_id,payload_sha256 FROM zip_distribution_jobs WHERE request_id=$1",
    )
    .bind(key)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((id, owner, bound)) = existing {
        if owner != app || bound != hash {
            return Err(AppError::conflict("请求标识已绑定另一分发意图"));
        }
        return Ok(Json(json!({"id":id,"reused":true})));
    }
    let row:Option<(String,String,String,String,Option<String>,Value)>=sqlx::query_as("SELECT a.code,r.version,r.entry_url,r.environment,r.artifact_digest,COALESCE(f.manifest,'null'::jsonb) FROM application_releases r JOIN applications a ON a.id=r.application_id LEFT JOIN application_release_artifacts f ON f.application_id=r.application_id AND f.version=r.version AND f.artifact_digest=r.artifact_digest WHERE r.id=$1 AND r.application_id=$2 AND r.entry_type='HTML' FOR UPDATE OF r").bind(release).bind(app).fetch_optional(&mut *tx).await?;
    let (code, version, entry, environment, digest, manifest) =
        row.ok_or_else(|| AppError::not_found("HTML发布版本不存在"))?;
    let digest = digest.ok_or_else(|| AppError::bad_request("请先通过ZIP上传绑定规范产物"))?;
    let manifest: Manifest =
        serde_json::from_value(manifest).map_err(|_| AppError::conflict("规范产物清单不可用"))?;
    zip::validate_manifest(&manifest).map_err(|_| AppError::conflict("规范产物摘要不一致"))?;
    let policy = crate::releases::FetchPolicy::new(&state.config, &environment)?;
    let canonical = crate::managed_deployments::canonical_entry(&policy.resolve(&entry, None)?);
    if digest != manifest.artifact_sha256
        || !group.policy.applications.contains(&code)
        || canonical
            != format!(
                "{}/sub-apps/{code}/versions/{version}/index.html",
                group.policy.public_origin
            )
    {
        return Err(AppError::bad_request("目标不允许该应用或ZIP固定版本入口"));
    }
    let image: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM managed_deployment_bindings WHERE release_id=$1)",
    )
    .bind(release)
    .fetch_one(&mut *tx)
    .await?;
    if image {
        return Err(AppError::conflict(
            "该发布已绑定镜像交付，不能同时绑定ZIP分发",
        ));
    }
    let bound: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM zip_distribution_jobs WHERE release_id=$1)",
    )
    .bind(release)
    .fetch_one(&mut *tx)
    .await?;
    if bound {
        return Err(AppError::conflict(
            "该版本已有分发任务，请读取状态、核验或重试原任务",
        ));
    }
    let revision = zip::revision(&group.policy).map_err(|_| AppError::Unavailable)?;
    let root = format!(
        "{}/sub-apps/{code}/versions/{version}/",
        group.policy.public_origin
    );
    let id:Uuid=sqlx::query_scalar("INSERT INTO zip_distribution_jobs(application_id,release_id,group_id,artifact_digest,manifest,policy_revision,policy,request_id,payload_sha256,created_by,public_root, id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11, $12) RETURNING id")
        .bind(app).bind(release).bind(&group.policy.id).bind(&digest).bind(json!(manifest)).bind(&revision).bind(json!(group.policy)).bind(key).bind(hash).bind(user.user_id).bind(root).bind(crate::util::uuid7::new_uuid()).fetch_one(&mut *tx).await?;
    for node in &group.policy.nodes {
        sqlx::query("INSERT INTO zip_distribution_nodes(job_id,node_id) VALUES($1,$2)")
            .bind(id)
            .bind(&node.id)
            .execute(&mut *tx)
            .await?;
    }
    audit(&mut tx,app,Some(&user),"ZIP_DISTRIBUTION_CREATE",json!({"jobId":id,"releaseId":release,"groupId":group.policy.id,"artifactDigest":digest,"requiredNodes":group.policy.nodes.len()})).await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"reused":false})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
async fn retry(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((app, job)): Path<(String, String)>,
    ApiJson(_): ApiJson<Empty>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user, true).await?;
    let app = uuid(&app)?;
    let id = uuid(&job)?;
    let cfg = config::load(&state)?;
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, app).await?;
    let row:Option<(String,String,String)>=sqlx::query_as("SELECT state,group_id,policy_revision FROM zip_distribution_jobs WHERE id=$1 AND application_id=$2 FOR UPDATE").bind(id).bind(app).fetch_optional(&mut *tx).await?;
    let (status, group, revision) = row.ok_or_else(|| AppError::not_found("分发任务不存在"))?;
    if status != "FAILED" {
        return Err(AppError::conflict(
            "仅失败任务可重试；在途或完成任务请读取状态或核验",
        ));
    }
    if !cfg
        .groups
        .iter()
        .any(|g| g.policy.id == group && zip::revision(&g.policy).ok().as_ref() == Some(&revision))
    {
        return Err(AppError::conflict("目标拓扑已变化，请使用新的版本与目标"));
    }
    sqlx::query("UPDATE zip_distribution_jobs SET state='QUEUED',nonce=NULL,lease_until=NULL,verification_nonce=NULL,verified_at=NULL,error_code=NULL,updated_at=now() WHERE id=$1").bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE zip_distribution_nodes SET state='PENDING',receipt=NULL,error_code=NULL,verified_at=NULL WHERE job_id=$1").bind(id).execute(&mut *tx).await?;
    audit(
        &mut tx,
        app,
        Some(&user),
        "ZIP_DISTRIBUTION_RETRY",
        json!({"jobId":id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id})))
}
async fn verify(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((app, job)): Path<(String, String)>,
    ApiJson(_): ApiJson<Empty>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user, true).await?;
    let app = uuid(&app)?;
    let id = uuid(&job)?;
    let release: Option<Uuid> = sqlx::query_scalar(
        "SELECT release_id FROM zip_distribution_jobs WHERE id=$1 AND application_id=$2",
    )
    .bind(id)
    .bind(app)
    .fetch_optional(&state.pool)
    .await?;
    let release = release.ok_or_else(|| AppError::not_found("分发任务不存在"))?;
    let row: crate::releases::ReleaseRow = sqlx::query_as(&format!(
        "{} FROM application_releases r WHERE r.id=$1",
        crate::releases::RELEASE_SELECT
    ))
    .bind(release)
    .fetch_one(&state.pool)
    .await?;
    gates::verify_nodes(&state, &row).await?;
    Ok(Json(json!({"id":id,"verified":true})))
}
async fn route_ready(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    Ok(if gates::route_ready(&state, &headers).await? {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::FORBIDDEN
    })
}
