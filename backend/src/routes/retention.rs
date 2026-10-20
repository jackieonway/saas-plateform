//! Static-release impact preview; deliberately no apply/delete route.
use crate::{
    error::AppError,
    extractors::{CurrentUser, RequirePermission},
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use saas_platform_backend::retention::{self, Policy, Version};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub(super) fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/applications/{id}/releases/retention-preview",
        get(preview),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    minimum_age_days: Option<u32>,
    keep_latest: Option<usize>,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    application_id: Uuid,
    environment: String,
    version: String,
    created_at: DateTime<Utc>,
    entry_url: String,
    resources: Value,
    artifact_digest: Option<String>,
    active: bool,
    previous: bool,
    in_flight: bool,
    platform_owned: bool,
}

async fn preview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(raw): Path<String>,
    Query(options): Query<Options>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:application")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let app: Uuid = raw
        .parse()
        .map_err(|_| AppError::bad_request("ID格式错误"))?;
    let age = options.minimum_age_days.unwrap_or(30);
    let keep = options.keep_latest.unwrap_or(5);
    let policy =
        Policy::new(age, keep).map_err(|_| AppError::bad_request("候选保留参数超出范围"))?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM applications WHERE id=$1)")
        .bind(app)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Err(AppError::not_found("应用不存在"));
    }
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT transaction_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    // Global inventory prevents hiding a shared URL/digest behind another app or
    // environment. The endpoint returns only the selected application's IDs.
    let rows:Vec<Row>=sqlx::query_as("SELECT r.id,r.application_id,r.environment,r.version,r.created_at,r.entry_url,r.resources,r.artifact_digest, \
        EXISTS(SELECT 1 FROM application_deployments d WHERE d.active_release_id=r.id) AS active, \
        EXISTS(SELECT 1 FROM application_deployments d WHERE d.previous_release_id=r.id) AS previous, \
        EXISTS(SELECT 1 FROM application_release_operations o WHERE o.application_id=r.application_id) AS in_flight, \
        EXISTS(SELECT 1 FROM application_release_artifacts a WHERE a.application_id=r.application_id AND a.version=r.version AND a.artifact_digest=r.artifact_digest) AS platform_owned \
        FROM application_releases r ORDER BY r.id LIMIT 5001").fetch_all(&mut *tx).await?;
    if rows.len() > 5000 {
        return Err(AppError::Unavailable);
    }
    tx.commit().await?;
    let origin = if state.config.public_app_url.is_empty() {
        &state.config.micro_gateway_origin
    } else {
        &state.config.public_app_url
    };
    let base = reqwest::Url::parse(origin).map_err(|_| AppError::Unavailable)?;
    let input = rows
        .iter()
        .map(|row| {
            let mut keys = Vec::new();
            if let Some(digest) = &row.artifact_digest {
                keys.push(format!("digest:{digest}"));
            }
            let entry = base
                .join(&row.entry_url)
                .map_err(|_| AppError::Unavailable)?;
            keys.push(format!("url:{entry}"));
            let resources = row.resources.as_array().ok_or(AppError::Unavailable)?;
            if resources.len() > 48 {
                return Err(AppError::Unavailable);
            }
            for resource in resources {
                let raw = resource["url"].as_str().ok_or(AppError::Unavailable)?;
                let url = entry.join(raw).map_err(|_| AppError::Unavailable)?;
                keys.push(format!("url:{url}"));
            }
            Ok::<_, AppError>(Version {
                id: row.id,
                application: row.application_id,
                environment: row.environment.clone(),
                version: row.version.clone(),
                created_at: row.created_at,
                artifact_keys: keys,
                active: row.active,
                previous: row.previous,
                in_flight: row.in_flight,
                platform_owned: row.platform_owned,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let decisions = tokio::task::spawn_blocking(move || retention::preview(&input, policy, now))
        .await
        .map_err(|_| AppError::Unavailable)?;
    let records=rows.iter().zip(decisions).filter(|(row,_)|row.application_id==app).map(|(row,decision)|json!({"releaseId":decision.id,"environment":row.environment,"version":row.version,"artifactKind":if row.platform_owned{"ZIP"}else{"EXTERNAL_URL"},"disposition":if decision.reasons.is_empty(){"MANUAL_REVIEW"}else{"PROTECTED"},"reasons":decision.reasons})).collect::<Vec<_>>();
    let protected = records
        .iter()
        .filter(|row| row["disposition"] == "PROTECTED")
        .count();
    let output = json!({"mode":"DRY_RUN","applicationId":app,"observedAt":now,"policy":{"minimumAgeDays":age,"keepLatest":keep,"approval":"ISOLATED_CANDIDATE_ONLY"},"applyAvailable":false,"deletionPermitted":false,"externalTaskInventory":"NOT_AUTHORITATIVE_NO_APPLY","total":records.len(),"protected":protected,"manualReview":records.len()-protected,"records":records});
    if serde_json::to_vec(&output)
        .map_err(|_| AppError::Unavailable)?
        .len()
        > 1024 * 1024
    {
        return Err(AppError::Unavailable);
    }
    Ok(Json(output))
}
