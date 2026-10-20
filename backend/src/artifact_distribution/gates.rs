use super::{
    audit, config, lock, transport,
    worker::{Job, SELECT},
};
use crate::{
    error::AppError,
    releases::{FetchPolicy, ReleaseRow},
    state::AppState,
};
use axum::http::HeaderMap;
use deployment_contract::zip;
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

fn entry(state: &AppState, row: &ReleaseRow) -> Result<String, AppError> {
    Ok(crate::managed_deployments::canonical_entry(
        &FetchPolicy::new(&state.config, &row.environment)?.resolve(&row.entry_url, None)?,
    ))
}
fn root(entry: &str) -> Option<String> {
    let url = reqwest::Url::parse(entry).ok()?;
    let parts: Vec<_> = url.path().split('/').collect();
    if parts.len() < 6
        || parts[1] != "sub-apps"
        || parts[3] != "versions"
        || parts[2].is_empty()
        || parts[4].is_empty()
    {
        return None;
    }
    Some(format!(
        "{}/sub-apps/{}/versions/{}/",
        url.origin().ascii_serialization(),
        parts[2],
        parts[4]
    ))
}
fn configured(cfg: &config::Config, entry: &str) -> bool {
    cfg.groups.iter().any(|g| {
        g.policy.applications.iter().any(|app| {
            entry.starts_with(&format!(
                "{}/sub-apps/{app}/versions/",
                g.policy.public_origin
            ))
        })
    })
}
async fn owned(state: &AppState, row: &ReleaseRow) -> Result<bool, AppError> {
    let entry = entry(state, row)?;
    if configured(&config::load(state)?, &entry) {
        return Ok(true);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM zip_distribution_jobs WHERE release_id=$1 OR public_root=$2)",
    )
    .bind(row.id)
    .bind(root(&entry))
    .fetch_one(&state.pool)
    .await?)
}
pub(crate) async fn defer_upload_probe(
    state: &AppState,
    row: &ReleaseRow,
) -> Result<bool, AppError> {
    owned(state, row).await
}

fn matching<'a>(
    cfg: &'a config::Config,
    j: &Job,
    row: &ReleaseRow,
    entry: &str,
) -> Result<&'a config::ConfiguredGroup, AppError> {
    let g = cfg.groups.iter().find(|g| {
        g.policy.id == j.group_id
            && zip::revision(&g.policy).ok().as_ref() == Some(&j.policy_revision)
    });
    if row.id != j.release_id
        || row.application_id != j.application_id
        || row.version != j.version
        || row.entry_type != "HTML"
        || row.artifact_digest.as_ref() != Some(&j.artifact_digest)
        || entry != format!("{}index.html", j.public_root)
    {
        return Err(AppError::conflict("发布未绑定自身的规范ZIP分发任务"));
    }
    g.ok_or_else(|| AppError::conflict("ZIP分发目标配置缺失或拓扑已变化"))
}

/// Each probe gets a fenced nonce; no database connection is held across node I/O.
pub(crate) async fn verify_nodes(state: &AppState, row: &ReleaseRow) -> Result<(), AppError> {
    if !owned(state, row).await? {
        return Ok(());
    }
    let cfg = config::load(state)?;
    let j: Job = sqlx::query_as(&format!("{SELECT} WHERE j.release_id=$1"))
        .bind(row.id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::conflict("必需节点尚无ZIP分发回执"))?;
    let group = matching(&cfg, &j, row, &entry(state, row)?)?;
    if j.state != "READY" {
        return Err(AppError::conflict("必需节点分发未完成，请读取任务并重试"));
    }
    let nonce = Uuid::new_v4();
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, j.application_id).await?;
    let changed = sqlx::query("UPDATE zip_distribution_jobs SET verification_nonce=$2,verified_at=NULL WHERE id=$1 AND state='READY' AND attempt=$3").bind(j.id).bind(nonce).bind(j.attempt).execute(&mut *tx).await?.rows_affected();
    if changed != 1 {
        return Err(AppError::conflict("分发任务已变化"));
    }
    audit(
        &mut tx,
        j.application_id,
        None,
        "ZIP_VERIFY_BEGIN",
        json!({"jobId":j.id,"nonce":nonce}),
    )
    .await?;
    tx.commit().await?;
    let manifest = j.manifest()?;
    let mut results = Vec::new();
    for n in &group.policy.nodes {
        results.push((
            n.id.clone(),
            transport::node(group, &j.work(&n.id, nonce), &manifest, None).await,
        ));
    }
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, j.application_id).await?;
    let valid:bool=sqlx::query_scalar("SELECT state='READY' AND attempt=$2 AND verification_nonce=$3 FROM zip_distribution_jobs WHERE id=$1 FOR UPDATE").bind(j.id).bind(j.attempt).bind(nonce).fetch_one(&mut *tx).await?;
    if !valid {
        return Err(AppError::conflict("核验已被另一任务取代"));
    }
    let current = config::load(state)?;
    let ready = results.iter().all(|(_, r)| r.is_ok())
        && matching(&current, &j, row, &entry(state, row)?).is_ok();
    for (id, result) in &results {
        sqlx::query("UPDATE zip_distribution_nodes SET state=$3,receipt=$4,error_code=$5,verified_at=CASE WHEN $3='READY' THEN now() ELSE NULL END WHERE job_id=$1 AND node_id=$2")
            .bind(j.id).bind(id).bind(if result.is_ok(){"READY"}else{"FAILED"}).bind(result.as_ref().ok().map(|r|json!(r))).bind(result.as_ref().err().copied()).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE zip_distribution_jobs SET state=$2,verified_at=CASE WHEN $3 THEN now() ELSE NULL END,error_code=CASE WHEN $3 THEN NULL ELSE 'NODE_VERIFICATION_FAILED' END,updated_at=now() WHERE id=$1").bind(j.id).bind(if ready{"READY"}else{"FAILED"}).bind(ready).execute(&mut *tx).await?;
    audit(
        &mut tx,
        j.application_id,
        None,
        "ZIP_VERIFY_FINISH",
        json!({"jobId":j.id,"nonce":nonce,"ready":ready}),
    )
    .await?;
    tx.commit().await?;
    if ready {
        Ok(())
    } else {
        Err(AppError::conflict(
            "必需节点缺失、损坏、离线或认证失败，禁止预检和切换",
        ))
    }
}

pub(crate) async fn require_ready(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    row: &ReleaseRow,
) -> Result<(), AppError> {
    let entry = entry(state, row)?;
    let cfg = config::load(state)?;
    let historical: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM zip_distribution_jobs WHERE release_id=$1 OR public_root=$2)",
    )
    .bind(row.id)
    .bind(root(&entry))
    .fetch_one(&mut **tx)
    .await?;
    if !historical && !configured(&cfg, &entry) {
        return Ok(());
    }
    let j: Job = sqlx::query_as(&format!("{SELECT} WHERE j.release_id=$1 FOR SHARE OF j"))
        .bind(row.id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::conflict("ZIP节点回执缺失"))?;
    let group = matching(&cfg, &j, row, &entry)?;
    let receipts: Vec<(String, String, Option<Value>)> = sqlx::query_as(
        "SELECT node_id,state,receipt FROM zip_distribution_nodes WHERE job_id=$1 FOR SHARE",
    )
    .bind(j.id)
    .fetch_all(&mut **tx)
    .await?;
    let fresh = j
        .verified_at
        .is_some_and(|t| (chrono::Utc::now() - t).num_seconds() < 60);
    let valid = j.state == "READY" && fresh && proofs(&j, group, &receipts);
    if valid {
        Ok(())
    } else {
        Err(AppError::conflict("ZIP逐节点核验证据缺失或过期"))
    }
}

/// Nginx sends raw $request_uri, never the already-decoded $uri.
pub(super) async fn route_ready(state: &AppState, headers: &HeaderMap) -> Result<bool, AppError> {
    let Some(uri) = headers.get("x-original-uri").and_then(|v| v.to_str().ok()) else {
        return Ok(false);
    };
    if !uri.starts_with('/') || uri.starts_with("//") || uri.contains('\\') {
        return Ok(false);
    }
    let url = reqwest::Url::parse(&format!(
        "{}{}",
        state.config.micro_gateway_origin.trim_end_matches('/'),
        uri
    ))
    .map_err(|_| AppError::Unavailable)?;
    let canonical = crate::managed_deployments::canonical_entry(&url);
    let Some(root) = root(&canonical) else {
        return Ok(false);
    };
    let jobs: Vec<Job> = sqlx::query_as(&format!("{SELECT} WHERE j.public_root=$1"))
        .bind(root)
        .fetch_all(&state.pool)
        .await?;
    if jobs.is_empty() {
        return Ok(false);
    }
    let cfg = config::load(state);
    for j in jobs {
        let protected:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM application_deployments d JOIN application_releases r ON r.id=d.active_release_id OR r.id=d.previous_release_id WHERE r.application_id=$1 AND r.version=$2)").bind(j.application_id).bind(&j.version).fetch_one(&state.pool).await?;
        if protected && j.ever_ready {
            return Ok(true);
        }
        if let Ok(cfg) = &cfg {
            if let Some(group) = cfg.groups.iter().find(|g| {
                g.policy.id == j.group_id
                    && zip::revision(&g.policy).ok().as_ref() == Some(&j.policy_revision)
            }) {
                let receipts = sqlx::query_as(
                    "SELECT node_id,state,receipt FROM zip_distribution_nodes WHERE job_id=$1",
                )
                .bind(j.id)
                .fetch_all(&state.pool)
                .await?;
                if j.state == "READY" && j.verified_at.is_some() && proofs(&j, group, &receipts) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn proofs(j: &Job, g: &config::ConfiguredGroup, rows: &[(String, String, Option<Value>)]) -> bool {
    let (Some(nonce), Ok(manifest)) = (j.verification_nonce, j.manifest()) else {
        return false;
    };
    rows.len() == g.policy.nodes.len()
        && g.policy.nodes.iter().all(|n| {
            rows.iter().any(|(id, status, raw)| {
                if id != &n.id || status != "READY" {
                    return false;
                }
                let Some(r) = raw
                    .as_ref()
                    .and_then(|v| serde_json::from_value::<zip::Receipt>(v.clone()).ok())
                else {
                    return false;
                };
                ["INSTALL", "VERIFY"].contains(&r.operation.as_str())
                    && zip::receipt_matches(&r, &j.work(&n.id, nonce), &manifest, &r.operation)
            })
        })
}
#[cfg(test)]
mod tests {
    #[test]
    fn normalized_aliases_keep_historical_ownership() {
        let origin = "https://host.test";
        for path in [
            "/sub-apps/stores/versions/v1/index.html",
            "/sub-apps/%73tores/versions/v1/index.html",
            "/sub-apps//stores/versions/other/../v1/index.html",
            "/sub-apps/stores/versions/v1/assets/app.js",
        ] {
            let u = reqwest::Url::parse(&format!("{origin}{path}")).unwrap();
            assert_eq!(
                super::root(&crate::managed_deployments::canonical_entry(&u)),
                Some(format!("{origin}/sub-apps/stores/versions/v1/"))
            );
        }
        assert!(super::root("https://host.test/unmanaged/index.html").is_none());
    }
}
