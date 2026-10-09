use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::releases::{self, FetchPolicy, ReleaseRow, Resource, RELEASE_SELECT};
use crate::release_archives;
use crate::state::AppState;
use crate::util::audit;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/applications/{id}/releases", get(list).post(register))
        .route("/api/applications/{id}/releases/upload", post(upload)
            .layer(DefaultBodyLimit::max(release_archives::MAX_ARCHIVE_BYTES)))
        .route(
            "/api/applications/{id}/releases/{release_id}/preflight",
            post(preflight),
        )
        .route(
            "/api/applications/{id}/releases/{release_id}/activate",
            post(activate),
        )
        .route("/api/applications/{id}/deployments", get(deployments))
        .route(
            "/api/applications/{id}/deployments/{environment}/rollback",
            post(rollback),
        )
}

async fn authorize(state: &AppState, user: &CurrentUser) -> Result<(), AppError> {
    RequirePermission("system:application")
        .check(state, user)
        .await?;
    state.require_platform(user).await
}

fn uuid(value: &str) -> Result<Uuid, AppError> {
    value
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))
}

async fn ensure_application(state: &AppState, id: Uuid) -> Result<(), AppError> {
    if !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM applications WHERE id = $1)")
        .bind(id)
        .fetch_one(&state.pool)
        .await?
    {
        return Err(AppError::not_found("应用不存在"));
    }
    Ok(())
}

async fn release(state: &AppState, app: Uuid, id: Uuid) -> Result<ReleaseRow, AppError> {
    sqlx::query_as(&format!(
        "{RELEASE_SELECT} FROM application_releases r WHERE r.application_id=$1 AND r.id=$2"
    ))
    .bind(app)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("该应用的发布版本不存在"))
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user).await?;
    let app = uuid(&id)?;
    ensure_application(&state, app).await?;
    let environment = query
        .get("environment")
        .map(|s| releases::validate_environment(s))
        .transpose()?;
    let rows: Vec<ReleaseRow> = sqlx::query_as(&format!("{RELEASE_SELECT} FROM application_releases r \
        WHERE r.application_id=$1 AND ($2::text IS NULL OR r.environment=$2) ORDER BY r.created_at DESC, r.id"))
        .bind(app).bind(environment).fetch_all(&state.pool).await?;
    Ok(Json(json!(rows)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegisterBody {
    environment: String,
    version: String,
    entry_type: String,
    entry_url: String,
    health_url: Option<String>,
    #[serde(default)]
    resources: Vec<Resource>,
}

async fn register(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RegisterBody>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user).await?;
    let app = uuid(&id)?;
    ensure_application(&state, app).await?;
    register_fields(&state, &user, &headers, app, body).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadQuery {
    environment: String,
    version: String,
    entry_type: String,
    format: String,
}

async fn upload(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<UploadQuery>,
    archive: Bytes,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user).await?;
    let app = uuid(&id)?;
    let row: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT code,base_path,entry_type,runtime_mode FROM applications WHERE id=$1"
    ).bind(app).fetch_optional(&state.pool).await?;
    let (code, base_path, app_entry_type, runtime_mode) = row.ok_or_else(|| AppError::not_found("应用不存在"))?;
    if code.len() < 2 || code.len() > 32
        || !code.bytes().next().is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !code.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-') {
        return Err(AppError::bad_request("应用编码不适合静态版本目录"));
    }
    if base_path.trim_end_matches('/') != format!("/micro/{code}") {
        return Err(AppError::bad_request("只有独立微应用可以上传前端压缩包"));
    }
    if query.entry_type != "HTML" || app_entry_type != "HTML" || runtime_mode != "IFRAME" {
        return Err(AppError::bad_request("仅 HTML iframe 微应用支持上传版本包；ESM 随宿主构建发布"));
    }
    let environment = releases::validate_environment(&query.environment)?;
    let version = releases::validate_version(&query.version)?;
    let format = query.format;
    let entry_type = query.entry_type.clone();
    let root = state.config.micro_apps_dir.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        release_archives::prepare(&archive, &format, &entry_type)
    }).await.map_err(|_| AppError::bad_request("压缩包处理失败"))??;
    let entry_url = format!("/sub-apps/{code}/versions/{version}/{}", prepared.entry);
    let body = RegisterBody {
        environment,
        version: version.clone(),
        entry_type: query.entry_type,
        entry_url,
        health_url: None,
        resources: prepared.resources.clone(),
    };
    let registered_environment = body.environment.clone();
    let registered_version = body.version.clone();
    let (destination, created) = tokio::task::spawn_blocking(move || {
        release_archives::write(&prepared, &root, &code, &version)
    }).await.map_err(|_| AppError::bad_request("版本文件写入失败"))??;
    let registered = register_fields(&state, &user, &headers, app, body).await;
    if registered.is_err() && created {
        // A database commit may have succeeded even if the final response lookup failed.
        // Keep files unless the database confirms this release was not registered.
        let exists: Result<bool, _> = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM application_releases WHERE application_id=$1 AND environment=$2 AND version=$3)"
        ).bind(app).bind(registered_environment).bind(registered_version)
            .fetch_one(&state.pool).await;
        if matches!(exists, Ok(false)) {
            tokio::task::spawn_blocking(move || release_archives::remove_uploaded(&destination))
                .await.ok();
        }
    }
    let Json(row) = registered?;
    let id = row["id"].as_str().and_then(|value| value.parse::<Uuid>().ok())
        .ok_or_else(|| AppError::conflict("版本已登记，但无法读取其 ID"))?;
    let checked = check_and_save(&state, app, id).await?;
    Ok(Json(json!(checked)))
}

async fn register_fields(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    app: Uuid,
    body: RegisterBody,
) -> Result<Json<Value>, AppError> {
    let environment = releases::validate_environment(&body.environment)?;
    let version = releases::validate_version(&body.version)?;
    if !["HTML", "ESM"].contains(&body.entry_type.as_str()) {
        return Err(AppError::bad_request("entryType 仅支持 HTML / ESM"));
    }
    if body.resources.len() > 48 {
        return Err(AppError::bad_request("显式 resources 清单最多 48 个资源"));
    }
    let policy = FetchPolicy::new(&state.config, &environment)?;
    let entry = policy.resolve(&body.entry_url, None)?;
    releases::version_root(&entry, &version)?;
    let entry_url = policy.stored_url(&body.entry_url, &entry);
    let health_url = body
        .health_url
        .as_deref()
        .map(|raw| {
            let url = policy.resolve(raw, None)?;
            Ok::<_, AppError>(policy.stored_url(raw, &url))
        })
        .transpose()?;
    let resources = body
        .resources
        .into_iter()
        .map(|resource| {
            if !["SCRIPT", "STYLE", "ASSET"].contains(&resource.kind.as_str()) {
                return Err(AppError::bad_request(
                    "resource.kind 仅支持 SCRIPT / STYLE / ASSET",
                ));
            }
            let url = policy.resolve(&resource.url, Some(&entry))?;
            Ok(Resource {
                url: url.to_string(),
                kind: resource.kind,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut tx = state.pool.begin().await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO application_releases \
        (application_id, environment, version, entry_type, entry_url, health_url, resources, created_by) \
        VALUES($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id")
        .bind(app).bind(&environment).bind(&version).bind(body.entry_type).bind(entry_url)
        .bind(health_url).bind(json!(resources)).bind(user.user_id).fetch_one(&mut *tx).await?;
    write_audit(
        &mut tx,
        user,
        headers,
        app,
        "REGISTER_APPLICATION_RELEASE",
        &format!("application={app} environment={environment} version={version} release={id}"),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!(release(state, app, id).await?)))
}

async fn check_and_save(state: &AppState, app: Uuid, id: Uuid) -> Result<ReleaseRow, AppError> {
    let mut row = release(state, app, id).await?;
    let result = releases::preflight(&state.config, &row).await;
    let passed = result["passed"].as_bool().unwrap_or(false);
    let status = if passed { "PASSED" } else { "FAILED" };
    let checked_at: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE application_releases \
        SET validation_status=$3, validation_result=$4, checked_at=now() \
        WHERE application_id=$1 AND id=$2 RETURNING checked_at",
    )
    .bind(app)
    .bind(id)
    .bind(status)
    .bind(&result)
    .fetch_one(&state.pool)
    .await?;
    // Return this invocation's result, never a concurrent preflight's persisted status.
    // Its runtime mode is the exact policy used and is checked under the deployment lock.
    row.validation_status = status.into();
    row.validation_result = result;
    row.checked_at = Some(checked_at);
    row.active = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM application_deployments \
        WHERE application_id=$1 AND environment=$2 AND active_release_id=$3)",
    )
    .bind(app)
    .bind(&row.environment)
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    Ok(row)
}

async fn preflight(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path((app, id)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user).await?;
    let app = uuid(&app)?;
    let id = uuid(&id)?;
    let row = check_and_save(&state, app, id).await?;
    // A failed resource check is a persisted result, not a lost HTTP error response.
    audit::write_for_app(
        &state.pool,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "PREFLIGHT_APPLICATION_RELEASE",
            detail: &format!(
                "application={app} release={id} result={}",
                row.validation_status
            ),
            ip: &audit::client_ip(&headers),
            user_agent: &audit::ua_of(&headers),
        },
        app,
    )
    .await;
    Ok(Json(json!(row)))
}

#[derive(sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeploymentRow {
    application_id: Uuid,
    environment: String,
    active_release_id: Uuid,
    previous_release_id: Option<Uuid>,
    version: String,
    entry_type: String,
    entry_url: String,
    updated_at: DateTime<Utc>,
}

const DEPLOYMENT_SELECT: &str = "SELECT d.application_id, d.environment, d.active_release_id, d.previous_release_id, \
    r.version, r.entry_type, r.entry_url, d.updated_at FROM application_deployments d \
    JOIN application_releases r ON r.application_id=d.application_id AND r.environment=d.environment AND r.id=d.active_release_id ";

async fn deployments(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(app): Path<String>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user).await?;
    let app = uuid(&app)?;
    ensure_application(&state, app).await?;
    let rows: Vec<DeploymentRow> = sqlx::query_as(&format!(
        "{DEPLOYMENT_SELECT} WHERE d.application_id=$1 ORDER BY d.environment"
    ))
    .bind(app)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!(rows)))
}

async fn activate(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path((app, id)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user).await?;
    let app = uuid(&app)?;
    let id = uuid(&id)?;
    let row = release(&state, app, id).await?;
    if row.validation_status != "PASSED" {
        return Err(AppError::conflict(
            "该版本尚未通过预检，请先执行 preflight 并查看诊断",
        ));
    }
    let checked = check_and_save(&state, app, id).await?;
    if checked.validation_status != "PASSED" {
        return Err(AppError::conflict(
            "启用前重新预检失败，诊断已保存到该发布版本",
        ));
    }
    let mut tx = state.pool.begin().await?;
    lock_application(&mut tx, app, &row.entry_type, &checked.runtime_mode).await?;
    require_passed(&mut tx, app, &row.environment, id).await?;
    sqlx::query("INSERT INTO application_deployments(application_id,environment,active_release_id,updated_by) \
        VALUES($1,$2,$3,$4) ON CONFLICT(application_id,environment) DO UPDATE \
        SET previous_release_id=CASE WHEN application_deployments.active_release_id=EXCLUDED.active_release_id \
            THEN application_deployments.previous_release_id ELSE application_deployments.active_release_id END, \
        active_release_id=EXCLUDED.active_release_id,updated_at=now(),updated_by=EXCLUDED.updated_by")
        .bind(app).bind(&row.environment).bind(id).bind(user.user_id).execute(&mut *tx).await?;
    write_audit(
        &mut tx,
        &user,
        &headers,
        app,
        "ACTIVATE_APPLICATION_RELEASE",
        &format!(
            "application={app} environment={} version={} release={id}",
            row.environment, row.version
        ),
    )
    .await?;
    let deployment = current_deployment(&mut tx, app, &row.environment).await?;
    tx.commit().await?;
    Ok(Json(json!(deployment)))
}

async fn rollback(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path((app, environment)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user).await?;
    let app = uuid(&app)?;
    let environment = releases::validate_environment(&environment)?;
    let snapshot: Option<(Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT active_release_id,previous_release_id \
        FROM application_deployments WHERE application_id=$1 AND environment=$2",
    )
    .bind(app)
    .bind(&environment)
    .fetch_optional(&state.pool)
    .await?;
    let (active, previous) = snapshot.ok_or_else(|| AppError::not_found("该环境尚未部署"))?;
    let previous = previous.ok_or_else(|| AppError::conflict("没有可回滚的上一版本"))?;
    let checked = check_and_save(&state, app, previous).await?;
    if checked.validation_status != "PASSED" {
        return Err(AppError::conflict(
            "上一版本重新预检失败，诊断已保存，部署指针保持当前版本",
        ));
    }
    let mut tx = state.pool.begin().await?;
    lock_application(&mut tx, app, &checked.entry_type, &checked.runtime_mode).await?;
    let current = current_deployment(&mut tx, app, &environment).await?;
    if current.active_release_id != active || current.previous_release_id != Some(previous) {
        return Err(AppError::conflict("部署在预检期间发生变化，请重新请求回滚"));
    }
    require_passed(&mut tx, app, &environment, previous).await?;
    sqlx::query("UPDATE application_deployments SET active_release_id=$3,previous_release_id=$4,updated_at=now(),updated_by=$5 \
        WHERE application_id=$1 AND environment=$2")
        .bind(app).bind(&environment).bind(previous).bind(active).bind(user.user_id).execute(&mut *tx).await?;
    write_audit(
        &mut tx,
        &user,
        &headers,
        app,
        "ROLLBACK_APPLICATION_RELEASE",
        &format!("application={app} environment={environment} from={active} to={previous}"),
    )
    .await?;
    let deployment = current_deployment(&mut tx, app, &environment).await?;
    tx.commit().await?;
    Ok(Json(json!(deployment)))
}

async fn lock_application(
    tx: &mut Transaction<'_, Postgres>,
    app: Uuid,
    entry_type: &str,
    checked_runtime_mode: &str,
) -> Result<(), AppError> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT status,runtime_mode FROM applications WHERE id=$1 FOR UPDATE")
            .bind(app)
            .fetch_optional(&mut **tx)
            .await?;
    let (status, runtime_mode) = row.ok_or_else(|| AppError::not_found("应用不存在"))?;
    if status != "ENABLED" {
        return Err(AppError::conflict("停用应用不能启用或回滚部署"));
    }
    if runtime_mode != checked_runtime_mode {
        return Err(AppError::conflict(
            "预检期间应用运行模式发生变化，请重新执行预检和部署",
        ));
    }
    if entry_type == "ESM" && runtime_mode != "TRUSTED" {
        return Err(AppError::conflict(
            "IFRAME 应用仅允许 HTML 入口；ESM 必须由平台管理员明确标记为 TRUSTED",
        ));
    }
    Ok(())
}

async fn require_passed(
    tx: &mut Transaction<'_, Postgres>,
    app: Uuid,
    environment: &str,
    id: Uuid,
) -> Result<(), AppError> {
    let status: Option<String> = sqlx::query_scalar(
        "SELECT validation_status FROM application_releases \
        WHERE application_id=$1 AND environment=$2 AND id=$3 FOR UPDATE",
    )
    .bind(app)
    .bind(environment)
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    if status.as_deref() != Some("PASSED") {
        return Err(AppError::conflict("版本预检状态已变化，请重新检查资源"));
    }
    Ok(())
}

async fn current_deployment(
    tx: &mut Transaction<'_, Postgres>,
    app: Uuid,
    environment: &str,
) -> Result<DeploymentRow, AppError> {
    sqlx::query_as(&format!(
        "{DEPLOYMENT_SELECT} WHERE d.application_id=$1 AND d.environment=$2"
    ))
    .bind(app)
    .bind(environment)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::not_found("该环境尚未部署"))
}

/// Pointer changes and their audit record commit atomically.
async fn write_audit(
    tx: &mut Transaction<'_, Postgres>,
    user: &CurrentUser,
    headers: &HeaderMap,
    application_id: Uuid,
    action: &str,
    detail: &str,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO audit_logs \
        (user_id,username,tenant_id,tenant_name,organization_id,organization_name,action,detail,ip,user_agent,request_id,application_id) \
        VALUES($1,$2,$3,(SELECT name FROM tenants WHERE id=$3),$4,(SELECT name FROM organizations WHERE id=$4),$5,$6,$7,$8,$9,$10)")
        .bind(user.user_id).bind(&user.username).bind(user.tenant_id).bind(user.org_id)
        .bind(action).bind(detail).bind(audit::client_ip(headers)).bind(audit::ua_of(headers))
        .bind(crate::observability::request_id()).bind(application_id)
        .execute(&mut **tx).await?;
    Ok(())
}
