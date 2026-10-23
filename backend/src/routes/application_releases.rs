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
use crate::release_archives;
use crate::releases::{self, FetchPolicy, ReleaseRow, Resource, RELEASE_SELECT};
use crate::state::AppState;
use crate::util::audit;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/applications/{id}/releases", get(list).post(register))
        .route(
            "/api/applications/{id}/releases/storage",
            get(storage_status),
        )
        .route(
            "/api/applications/{id}/releases/upload",
            post(upload).layer(DefaultBodyLimit::max(release_archives::MAX_ARCHIVE_BYTES)),
        )
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

async fn release_audit_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    app: Uuid,
    id: Uuid,
) -> Result<Value, AppError> {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('id',id,'name',version,'applicationId',application_id, \
         'environment',environment,'entryType',entry_type,'resourceCount',jsonb_array_length(resources), \
         'artifactBound',artifact_digest IS NOT NULL,'validationStatus',validation_status,'checkedAt',checked_at) \
         FROM application_releases WHERE application_id=$1 AND id=$2 FOR UPDATE",
    ).bind(app).bind(id).fetch_optional(&mut **tx).await?
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
#[serde(deny_unknown_fields)]
struct StorageQuery {
    version: String,
}

#[derive(sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageReference {
    id: Uuid,
    environment: String,
    artifact_digest: Option<String>,
    entry_url: String,
    active: bool,
}

async fn storage_status(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<StorageQuery>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user).await?;
    let app = uuid(&id)?;
    let version = releases::validate_version(&query.version)?;
    let code: String = sqlx::query_scalar("SELECT code FROM applications WHERE id=$1")
        .bind(app)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    let artifact: Option<(String, Value)> = sqlx::query_as(
        "SELECT artifact_digest,manifest FROM application_release_artifacts WHERE application_id=$1 AND version=$2"
    ).bind(app).bind(&version).fetch_optional(&state.pool).await?;
    let references: Vec<StorageReference> = sqlx::query_as(
        "SELECT r.id,r.environment,r.artifact_digest,r.entry_url, \
         EXISTS(SELECT 1 FROM application_deployments d WHERE d.application_id=r.application_id \
           AND d.environment=r.environment AND d.active_release_id=r.id) AS active \
         FROM application_releases r WHERE r.application_id=$1 AND r.version=$2 ORDER BY r.environment"
    ).bind(app).bind(&version).fetch_all(&state.pool).await?;
    let root = state.config.micro_apps_dir.clone();
    let checked_version = version.clone();
    let disk = tokio::task::spawn_blocking(move || {
        release_archives::inspect_installed(&root, &code, &checked_version)
    })
    .await
    .map_err(|_| AppError::conflict("静态存储检查未完成，请重试"))?;
    let (status, digest, manifest) = match disk {
        Err(_) => ("INCONSISTENT", None, None),
        Ok(None) => ("MISSING", None, None),
        Ok(Some(manifest)) => {
            let digest = manifest.artifact_sha256.clone();
            let value = serde_json::to_value(&manifest)
                .map_err(|_| AppError::conflict("无法读取静态产物清单"))?;
            let inconsistent = artifact
                .as_ref()
                .is_some_and(|(registered, expected)| registered != &digest || expected != &value)
                || references.iter().any(|reference| {
                    reference
                        .artifact_digest
                        .as_ref()
                        .is_some_and(|registered| registered != &digest)
                });
            let registered = references
                .iter()
                .any(|reference| reference.artifact_digest.as_deref() == Some(digest.as_str()));
            (
                if inconsistent {
                    "INCONSISTENT"
                } else if registered {
                    "REGISTERED"
                } else {
                    "INSTALLED_UNREGISTERED"
                },
                Some(digest),
                Some(value),
            )
        }
    };
    Ok(Json(json!({
        "version": version, "status": status, "artifactDigest": digest,
        "registeredArtifactDigest": artifact.as_ref().map(|(digest, _)| digest),
        "manifest": manifest, "references": references,
        "recovery": "仅核对并报告；可重试相同版本的同一产物，不自动删除已安装文件",
    })))
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
    let app_id = uuid(&id)?;
    ensure_application(&state, app_id).await?;
    crate::release_operations::track(
        &state,
        app_id,
        "REGISTER",
        register_inner(State(state.clone()), user, headers, Path(id), ApiJson(body)),
    )
    .await
}

async fn register_inner(
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
    let app_id = uuid(&id)?;
    ensure_application(&state, app_id).await?;
    crate::release_operations::track(
        &state,
        app_id,
        "UPLOAD",
        upload_inner(
            State(state.clone()),
            user,
            headers,
            Path(id),
            Query(query),
            archive,
        ),
    )
    .await
}

async fn upload_inner(
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
        "SELECT code,base_path,entry_type,runtime_mode FROM applications WHERE id=$1",
    )
    .bind(app)
    .fetch_optional(&state.pool)
    .await?;
    let (code, base_path, app_entry_type, runtime_mode) =
        row.ok_or_else(|| AppError::not_found("应用不存在"))?;
    if code.len() < 2
        || code.len() > 32
        || !code
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(AppError::bad_request("应用编码不适合静态版本目录"));
    }
    if base_path.trim_end_matches('/') != format!("/micro/{code}") {
        return Err(AppError::bad_request("只有独立微应用可以上传前端压缩包"));
    }
    if query.entry_type != "HTML" || app_entry_type != "HTML" || runtime_mode != "TRUSTED" {
        return Err(AppError::bad_request(
            "仅 qiankun（HTML + TRUSTED）微应用支持上传版本包",
        ));
    }
    let environment = releases::validate_environment(&query.environment)?;
    let version = releases::validate_version(&query.version)?;
    let format = query.format;
    let entry_type = query.entry_type.clone();
    let root = state.config.micro_apps_dir.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        release_archives::prepare(&archive, &format, &entry_type)
    })
    .await
    .map_err(|_| AppError::bad_request("压缩包处理失败"))??;
    let entry_url = format!("/sub-apps/{code}/versions/{version}/{}", prepared.entry);
    let body = RegisterBody {
        environment,
        version: version.clone(),
        entry_type: query.entry_type,
        entry_url,
        health_url: None,
        resources: prepared.resources.clone(),
    };
    let artifact = UploadArtifact {
        digest: prepared.artifact_sha256.clone(),
        manifest: serde_json::to_value(&prepared.manifest)
            .map_err(|_| AppError::conflict("无法生成静态产物清单"))?,
    };
    let installed = tokio::task::spawn_blocking(move || {
        release_archives::write(&prepared, &root, &code, &version)
    })
    .await
    .map_err(|_| AppError::bad_request("版本文件写入失败"))??;
    if installed.artifact_sha256 != artifact.digest {
        return Err(AppError::conflict("已安装产物与本次上传摘要不一致"));
    }
    tracing::info!(event = "zip_artifact_installed", application_id = %app,
        artifact_digest = %installed.artifact_sha256, created = installed.created);
    // Installed bytes are immutable and may already be referenced by another
    // request/environment. A failed or uncertain database operation never deletes them.
    let Json(row) = register_definition(&state, &user, &headers, app, body, Some(artifact)).await?;
    let id = row["id"]
        .as_str()
        .and_then(|value| value.parse::<Uuid>().ok())
        .ok_or_else(|| AppError::conflict("版本已登记，但无法读取其 ID"))?;
    let registered = release(&state, app, id).await?;
    if crate::artifact_distribution::defer_upload_probe(&state, &registered).await? {
        return Ok(Json(json!(registered)));
    }
    let checked = check_and_save(&state, &user, &headers, app, id).await?;
    Ok(Json(json!(checked)))
}

async fn register_fields(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    app: Uuid,
    body: RegisterBody,
) -> Result<Json<Value>, AppError> {
    register_definition(state, user, headers, app, body, None).await
}

struct UploadArtifact {
    digest: String,
    manifest: Value,
}

async fn register_definition(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    app: Uuid,
    body: RegisterBody,
    artifact: Option<UploadArtifact>,
) -> Result<Json<Value>, AppError> {
    let environment = releases::validate_environment(&body.environment)?;
    let version = releases::validate_version(&body.version)?;
    if body.entry_type != "HTML" {
        return Err(AppError::bad_request(
            "新发布仅支持 qiankun HTML 入口；历史 ESM 版本只读存档",
        ));
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
    let mut resources = body
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
    if artifact.is_some() {
        resources.sort_by(|left, right| (&left.url, &left.kind).cmp(&(&right.url, &right.kind)));
    }
    let resources = json!(resources);
    let mut tx = state.pool.begin().await?;
    // URL registration and ZIP promotion use the same lock so a URL-only row
    // cannot enter an already bound ZIP version while its artifact is adopted.
    // No filesystem or network work happens inside this short transaction.
    let current: (String, String, String, String) =
        sqlx::query_as("SELECT entry_type,runtime_mode,code,COALESCE(base_path,'') FROM applications WHERE id=$1 FOR UPDATE")
            .bind(app)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| AppError::not_found("应用不存在"))?;
    if current.0 != "HTML"
        || current.1 != "TRUSTED"
        || current.3.trim_end_matches('/') != format!("/micro/{}", current.2)
    {
        return Err(AppError::conflict(
            "请先将独立微应用明确配置为 qiankun（HTML + TRUSTED），内置平台页面不支持微应用发布",
        ));
    }
    // Cleanup fences new references until the remote engine operation finishes.
    // The application lock serializes this check with cleanup submission.
    let canonical_entry = crate::managed_deployments::canonical_entry(&entry);
    let managed_root = crate::managed_deployments::version_resource_root(&canonical_entry)
        .unwrap_or(canonical_entry);
    let managed_state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM managed_image_artifacts WHERE public_url=$1 FOR SHARE",
    )
    .bind(managed_root)
    .fetch_optional(&mut *tx)
    .await?;
    if managed_state.as_deref() == Some("CLEANING") {
        return Err(AppError::conflict(
            "该版本资源正在清理，不能创建或采用发布引用",
        ));
    }
    if artifact.is_none() && sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM application_release_artifacts WHERE application_id=$1 AND version=$2)"
    ).bind(app).bind(&version).fetch_one(&mut *tx).await? {
        return Err(AppError::conflict("该版本已绑定静态产物，请上传相同版本包以提升到其他环境"));
    }
    if let Some(artifact) = &artifact {
        sqlx::query("INSERT INTO application_release_artifacts(application_id,version,artifact_digest,manifest) \
            VALUES($1,$2,$3,$4) ON CONFLICT(application_id,version) DO NOTHING")
            .bind(app).bind(&version).bind(&artifact.digest).bind(&artifact.manifest).execute(&mut *tx).await?;
        let existing: (String, Value) = sqlx::query_as(
            "SELECT artifact_digest,manifest FROM application_release_artifacts WHERE application_id=$1 AND version=$2"
        ).bind(app).bind(&version).fetch_one(&mut *tx).await?;
        if existing.0 != artifact.digest || existing.1 != artifact.manifest {
            return Err(AppError::conflict(
                "该版本已绑定不同静态产物，请使用新版本号",
            ));
        }
        let rows: Vec<ReleaseRow> = sqlx::query_as(&format!(
            "{RELEASE_SELECT} FROM application_releases r WHERE r.application_id=$1 AND r.version=$2"
        )).bind(app).bind(&version).fetch_all(&mut *tx).await?;
        let mut matching = None;
        for row in rows {
            if !same_upload_definition(&row, &body.entry_type, &entry_url, &health_url, &resources)
                || row
                    .artifact_digest
                    .as_deref()
                    .is_some_and(|digest| digest != artifact.digest)
            {
                return Err(AppError::conflict(
                    "该版本的已有发布定义与静态产物不一致，不能自动采用",
                ));
            }
            if row.environment == environment {
                if row.artifact_digest.is_none() {
                    let before = release_audit_snapshot(&mut tx, app, row.id).await?;
                    let adopted = sqlx::query("UPDATE application_releases SET artifact_digest=$2 WHERE id=$1 AND artifact_digest IS NULL")
                        .bind(row.id).bind(&artifact.digest).execute(&mut *tx).await?;
                    if adopted.rows_affected() != 1 {
                        return Err(AppError::Server(anyhow::anyhow!(
                            "release artifact was not adopted"
                        )));
                    }
                    let detail = audit::change_detail(
                        "application_release",
                        row.id,
                        before,
                        release_audit_snapshot(&mut tx, app, row.id).await?,
                    );
                    write_audit(
                        &mut tx,
                        user,
                        headers,
                        app,
                        "ADOPT_APPLICATION_RELEASE_ARTIFACT",
                        &detail,
                    )
                    .await?;
                }
                matching = Some(row.id);
            }
        }
        if matching.is_some() {
            let outcome = tx.commit().await;
            return Ok(Json(json!(
                reconcile_upload_commit(
                    &state.pool,
                    app,
                    &environment,
                    &version,
                    &artifact.digest,
                    outcome
                )
                .await?
            )));
        }
    }
    let id: Uuid = sqlx::query_scalar("INSERT INTO application_releases \
        (application_id, environment, version, entry_type, entry_url, health_url, resources, created_by, artifact_digest) \
        VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id")
        .bind(app).bind(&environment).bind(&version).bind(body.entry_type).bind(entry_url)
        .bind(health_url).bind(resources).bind(user.user_id).bind(artifact.as_ref().map(|value| &value.digest))
        .fetch_one(&mut *tx).await?;
    let detail = audit::change_detail(
        "application_release",
        id,
        Value::Null,
        release_audit_snapshot(&mut tx, app, id).await?,
    );
    write_audit(
        &mut tx,
        user,
        headers,
        app,
        "REGISTER_APPLICATION_RELEASE",
        &detail,
    )
    .await?;
    let outcome = tx.commit().await;
    if let Some(artifact) = artifact {
        return Ok(Json(json!(
            reconcile_upload_commit(
                &state.pool,
                app,
                &environment,
                &version,
                &artifact.digest,
                outcome
            )
            .await?
        )));
    }
    outcome?;
    Ok(Json(json!(release(state, app, id).await?)))
}

fn same_upload_definition(
    row: &ReleaseRow,
    entry_type: &str,
    entry_url: &str,
    health_url: &Option<String>,
    resources: &Value,
) -> bool {
    let normalize = |value: &Value| {
        let mut values: Vec<Resource> = serde_json::from_value(value.clone()).ok()?;
        values.sort_by(|left, right| (&left.url, &left.kind).cmp(&(&right.url, &right.kind)));
        Some(json!(values))
    };
    row.entry_type == entry_type
        && row.entry_url == entry_url
        && &row.health_url == health_url
        && normalize(&row.resources) == normalize(resources)
}

/// Re-read the primary database after COMMIT, including when its acknowledgement
/// was lost. Absence at this instant never authorizes deleting installed bytes.
pub(crate) async fn reconcile_upload_commit(
    pool: &sqlx::PgPool,
    app: Uuid,
    environment: &str,
    version: &str,
    digest: &str,
    outcome: Result<(), sqlx::Error>,
) -> Result<ReleaseRow, AppError> {
    let observed: Result<Option<ReleaseRow>, _> = sqlx::query_as(&format!(
        "{RELEASE_SELECT} FROM application_releases r WHERE r.application_id=$1 AND r.environment=$2 AND r.version=$3"
    )).bind(app).bind(environment).bind(version).fetch_optional(pool).await;
    match observed {
        Ok(Some(row)) if row.artifact_digest.as_deref() == Some(digest) => Ok(row),
        Ok(Some(_)) => Err(AppError::conflict(
            "该环境版本已登记不同产物，请查询发布状态",
        )),
        Ok(None) | Err(_) if outcome.is_err() => Err(AppError::ReleaseResultUnknown(
            "静态产物已保留，数据库登记结果待确认；请查询存储状态并重试相同版本包".into(),
        )),
        Err(error) => Err(error.into()),
        Ok(None) => Err(AppError::ReleaseResultUnknown(
            "登记后暂未查到发布记录；静态产物已保留，请查询后重试".into(),
        )),
    }
}

async fn check_and_save(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    app: Uuid,
    id: Uuid,
) -> Result<ReleaseRow, AppError> {
    let mut row = release(state, app, id).await?;
    let compatible: bool = sqlx::query_scalar("SELECT entry_type='HTML' AND runtime_mode='TRUSTED' AND rtrim(base_path,'/')='/micro/' || code FROM applications WHERE id=$1")
        .bind(app).fetch_one(&state.pool).await?;
    if row.entry_type != "HTML" || row.runtime_mode != "TRUSTED" || !compatible {
        return Err(AppError::conflict(
            "历史 iframe/ESM 版本只读存档，不能预检、激活或回滚；请重新发布 qiankun HTML 入口",
        ));
    }
    crate::artifact_distribution::verify_nodes(state, &row).await?;
    let result = if verify_release_artifact(state, &row).await? {
        releases::preflight(&state.config, &row).await
    } else {
        json!({"passed":false,"checkedAt":Utc::now(),"checks":[{
            "url":row.entry_url,"kind":"ARTIFACT","passed":false,
            "message":"静态产物缺失、损坏或摘要不一致；请查询存储状态，不可启用或回滚"}]})
    };
    let passed = result["passed"].as_bool().unwrap_or(false);
    let status = if passed { "PASSED" } else { "FAILED" };
    // Resource/network checks stay outside locks. Persist this observation and
    // its audit atomically, independently of any later deployment-pointer tx.
    let mut tx = state.pool.begin().await?;
    let still_compatible = sqlx::query_scalar::<_, bool>("SELECT entry_type='HTML' AND runtime_mode='TRUSTED' AND rtrim(base_path,'/')='/micro/' || code FROM applications WHERE id=$1 FOR UPDATE")
        .bind(app)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    if !still_compatible {
        return Err(AppError::conflict(
            "预检期间应用已改为历史或内置入口，不能更新发布状态",
        ));
    }
    let before = release_audit_snapshot(&mut tx, app, id).await?;
    let checked_at: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE application_releases \
        SET validation_status=$3, validation_result=$4, checked_at=now() \
        WHERE application_id=$1 AND id=$2 RETURNING checked_at",
    )
    .bind(app)
    .bind(id)
    .bind(status)
    .bind(&result)
    .fetch_one(&mut *tx)
    .await?;
    let detail = audit::change_detail(
        "application_release_validation",
        id,
        before,
        release_audit_snapshot(&mut tx, app, id).await?,
    );
    write_audit(
        &mut tx,
        user,
        headers,
        app,
        "PREFLIGHT_APPLICATION_RELEASE",
        &detail,
    )
    .await?;
    tx.commit().await?;
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

async fn verify_release_artifact(state: &AppState, row: &ReleaseRow) -> Result<bool, AppError> {
    let Some(digest) = &row.artifact_digest else {
        return Ok(true);
    };
    let stored: Option<(String, Value)> = sqlx::query_as(
        "SELECT a.code,f.manifest FROM applications a JOIN application_release_artifacts f \
         ON f.application_id=a.id WHERE a.id=$1 AND f.version=$2 AND f.artifact_digest=$3",
    )
    .bind(row.application_id)
    .bind(&row.version)
    .bind(digest)
    .fetch_optional(&state.pool)
    .await?;
    let Some((code, expected)) = stored else {
        return Ok(false);
    };
    let root = state.config.micro_apps_dir.clone();
    let version = row.version.clone();
    let digest = digest.clone();
    let actual = tokio::task::spawn_blocking(move || {
        release_archives::verify_installed(&root, &code, &version, &digest)
    })
    .await;
    Ok(
        matches!(actual, Ok(Ok(manifest)) if serde_json::to_value(&manifest).ok().as_ref() == Some(&expected)),
    )
}

async fn preflight(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path((app, id)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user).await?;
    let app_id = uuid(&app)?;
    ensure_application(&state, app_id).await?;
    crate::release_operations::track(
        &state,
        app_id,
        "PREFLIGHT",
        preflight_inner(State(state.clone()), user, headers, Path((app, id))),
    )
    .await
}

async fn preflight_inner(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path((app, id)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &user).await?;
    let app = uuid(&app)?;
    let id = uuid(&id)?;
    // Failed checks are persisted diagnostics with their own committed audit.
    let row = check_and_save(&state, &user, &headers, app, id).await?;
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
    let app_id = uuid(&app)?;
    ensure_application(&state, app_id).await?;
    crate::release_operations::track(
        &state,
        app_id,
        "ACTIVATE",
        activate_inner(State(state.clone()), user, headers, Path((app, id))),
    )
    .await
}

async fn activate_inner(
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
    let checked = check_and_save(&state, &user, &headers, app, id).await?;
    if checked.validation_status != "PASSED" {
        return Err(AppError::conflict(
            "启用前重新预检失败，诊断已保存到该发布版本",
        ));
    }
    let mut tx = state.pool.begin().await?;
    lock_application(&mut tx, app, &row.entry_type, &checked.runtime_mode).await?;
    crate::managed_deployments::require_ready(&state, &mut tx, &checked).await?;
    crate::artifact_distribution::require_ready(&state, &mut tx, &checked).await?;
    require_passed(&mut tx, app, &row.environment, id).await?;
    let before: Option<DeploymentRow> = sqlx::query_as(&format!(
        "{DEPLOYMENT_SELECT} WHERE d.application_id=$1 AND d.environment=$2",
    ))
    .bind(app)
    .bind(&row.environment)
    .fetch_optional(&mut *tx)
    .await?;
    let changed = sqlx::query("INSERT INTO application_deployments(application_id,environment,active_release_id,updated_by) \
        VALUES($1,$2,$3,$4) ON CONFLICT(application_id,environment) DO UPDATE \
        SET previous_release_id=CASE WHEN application_deployments.active_release_id=EXCLUDED.active_release_id \
            THEN application_deployments.previous_release_id ELSE application_deployments.active_release_id END, \
        active_release_id=EXCLUDED.active_release_id,updated_at=now(),updated_by=EXCLUDED.updated_by")
        .bind(app).bind(&row.environment).bind(id).bind(user.user_id).execute(&mut *tx).await?;
    if changed.rows_affected() != 1 {
        return Err(AppError::Server(anyhow::anyhow!(
            "deployment pointer was not persisted"
        )));
    }
    let deployment = current_deployment(&mut tx, app, &row.environment).await?;
    crate::application_entries::clear_override(&mut tx, &user, &headers, app, &row.environment)
        .await?;
    let before = before
        .as_ref()
        .map(deployment_audit_summary)
        .unwrap_or(Value::Null);
    let after = deployment_audit_summary(&deployment);
    if before != after {
        let detail = audit::change_detail("application_deployment", app, before, after);
        write_audit(
            &mut tx,
            &user,
            &headers,
            app,
            "ACTIVATE_APPLICATION_RELEASE",
            &detail,
        )
        .await?;
    }
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
    let app_id = uuid(&app)?;
    ensure_application(&state, app_id).await?;
    crate::release_operations::track(
        &state,
        app_id,
        "ROLLBACK",
        rollback_inner(
            State(state.clone()),
            user,
            headers,
            Path((app, environment)),
        ),
    )
    .await
}

async fn rollback_inner(
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
    let checked = check_and_save(&state, &user, &headers, app, previous).await?;
    if checked.validation_status != "PASSED" {
        return Err(AppError::conflict(
            "上一版本重新预检失败，诊断已保存，部署指针保持当前版本",
        ));
    }
    let mut tx = state.pool.begin().await?;
    lock_application(&mut tx, app, &checked.entry_type, &checked.runtime_mode).await?;
    crate::managed_deployments::require_ready(&state, &mut tx, &checked).await?;
    crate::artifact_distribution::require_ready(&state, &mut tx, &checked).await?;
    let current = current_deployment(&mut tx, app, &environment).await?;
    if current.active_release_id != active || current.previous_release_id != Some(previous) {
        return Err(AppError::conflict("部署在预检期间发生变化，请重新请求回滚"));
    }
    require_passed(&mut tx, app, &environment, previous).await?;
    let changed = sqlx::query("UPDATE application_deployments SET active_release_id=$3,previous_release_id=$4,updated_at=now(),updated_by=$5 \
        WHERE application_id=$1 AND environment=$2")
        .bind(app).bind(&environment).bind(previous).bind(active).bind(user.user_id).execute(&mut *tx).await?;
    if changed.rows_affected() != 1 {
        return Err(AppError::Server(anyhow::anyhow!(
            "rollback pointer was not persisted"
        )));
    }
    let deployment = current_deployment(&mut tx, app, &environment).await?;
    crate::application_entries::clear_override(&mut tx, &user, &headers, app, &environment).await?;
    let detail = audit::change_detail(
        "application_deployment",
        app,
        deployment_audit_summary(&current),
        deployment_audit_summary(&deployment),
    );
    write_audit(
        &mut tx,
        &user,
        &headers,
        app,
        "ROLLBACK_APPLICATION_RELEASE",
        &detail,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!(deployment)))
}

async fn lock_application(
    tx: &mut Transaction<'_, Postgres>,
    app: Uuid,
    entry_type: &str,
    checked_runtime_mode: &str,
) -> Result<(), AppError> {
    let row: Option<(String, String, String, String, String)> =
        sqlx::query_as("SELECT status,runtime_mode,entry_type,code,COALESCE(base_path,'') FROM applications WHERE id=$1 FOR UPDATE")
            .bind(app)
            .fetch_optional(&mut **tx)
            .await?;
    let (status, runtime_mode, app_entry_type, code, base_path) =
        row.ok_or_else(|| AppError::not_found("应用不存在"))?;
    if status != "ENABLED" {
        return Err(AppError::conflict("停用应用不能启用或回滚部署"));
    }
    if runtime_mode != checked_runtime_mode {
        return Err(AppError::conflict(
            "预检期间应用运行模式发生变化，请重新执行预检和部署",
        ));
    }
    if entry_type != "HTML"
        || app_entry_type != "HTML"
        || runtime_mode != "TRUSTED"
        || base_path.trim_end_matches('/') != format!("/micro/{code}")
    {
        return Err(AppError::conflict(
            "仅 qiankun（HTML + TRUSTED）独立微应用可激活或回滚；历史 iframe/ESM 版本只读存档",
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

fn deployment_audit_summary(row: &DeploymentRow) -> Value {
    json!({"id":row.application_id,"name":row.version,"environment":row.environment,
        "activeReleaseId":row.active_release_id,"previousReleaseId":row.previous_release_id,
        "entryType":row.entry_type})
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
    audit::write_in_tx_for_app(
        tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action,
            detail,
            ip: &audit::client_ip(headers),
            user_agent: &audit::ua_of(headers),
        },
        application_id,
    )
    .await
}
