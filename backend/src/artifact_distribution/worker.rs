use super::{
    audit,
    config::{self, ConfiguredGroup},
    lock, transport,
};
use crate::{error::AppError, state::AppState};
use chrono::{DateTime, Utc};
use deployment_contract::zip::{self, Manifest, Receipt, Work};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Clone, sqlx::FromRow)]
pub(super) struct Job {
    pub id: Uuid,
    pub application_id: Uuid,
    pub release_id: Uuid,
    pub group_id: String,
    pub public_root: String,
    pub artifact_digest: String,
    pub manifest: Value,
    pub policy_revision: String,
    pub state: String,
    pub attempt: i32,
    pub nonce: Option<Uuid>,
    pub verification_nonce: Option<Uuid>,
    pub verified_at: Option<DateTime<Utc>>,
    pub ever_ready: bool,
    pub application_code: String,
    pub version: String,
}
pub(super) const SELECT:&str="SELECT j.id,j.application_id,j.release_id,j.group_id,j.public_root,j.artifact_digest,j.manifest,j.policy_revision,j.policy,j.state,j.attempt,j.nonce,j.lease_until,j.verification_nonce,j.verified_at,j.ever_ready,a.code AS application_code,r.version FROM zip_distribution_jobs j JOIN applications a ON a.id=j.application_id JOIN application_releases r ON r.id=j.release_id";
impl Job {
    pub fn manifest(&self) -> Result<Manifest, AppError> {
        let m: Manifest =
            serde_json::from_value(self.manifest.clone()).map_err(|_| AppError::Unavailable)?;
        zip::validate_manifest(&m).map_err(|_| AppError::Unavailable)?;
        if m.artifact_sha256 != self.artifact_digest {
            return Err(AppError::Unavailable);
        }
        Ok(m)
    }
    pub fn work(&self, node: &str, nonce: Uuid) -> Work {
        Work {
            schema_version: 1,
            job_id: self.id.to_string(),
            attempt: self.attempt,
            nonce: nonce.to_string(),
            group_id: self.group_id.clone(),
            policy_revision: self.policy_revision.clone(),
            node_id: node.to_owned(),
            application_code: self.application_code.clone(),
            version: self.version.clone(),
            artifact_sha256: self.artifact_digest.clone(),
        }
    }
}
pub(crate) fn start(state: AppState) {
    tokio::spawn(async move {
        loop {
            if std::env::var_os("ZIP_DISTRIBUTION_CONFIG_FILE").is_some()
                && tick(&state).await.is_err()
            {
                tracing::warn!(
                    error_code = "ZIP_DISTRIBUTOR_UNAVAILABLE",
                    "ZIP distributor cycle failed"
                );
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}
async fn claim(state: &AppState) -> Result<Option<(Job, ConfiguredGroup)>, AppError> {
    let cfg = config::load(state)?;
    if cfg.groups.is_empty() {
        return Ok(None);
    }
    let next:Option<(Uuid,Uuid)>=sqlx::query_as("SELECT id,application_id FROM zip_distribution_jobs WHERE state='QUEUED' OR (state='RUNNING' AND lease_until<now()) ORDER BY created_at,id LIMIT 1").fetch_optional(&state.pool).await?;
    let Some((id, app)) = next else {
        return Ok(None);
    };
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, app).await?;
    let mut j: Job = sqlx::query_as(&format!("{SELECT} WHERE j.id=$1 FOR UPDATE OF j"))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    let available:bool=sqlx::query_scalar("SELECT state='QUEUED' OR (state='RUNNING' AND lease_until<now()) FROM zip_distribution_jobs WHERE id=$1").bind(id).fetch_one(&mut *tx).await?;
    if !available {
        return Ok(None);
    }
    let group = cfg.groups.into_iter().find(|g| {
        g.policy.id == j.group_id
            && zip::revision(&g.policy).ok().as_ref() == Some(&j.policy_revision)
    });
    if group.is_none() || j.attempt >= 1000 {
        sqlx::query("UPDATE zip_distribution_jobs SET state='FAILED',error_code='POLICY_CHANGED',lease_until=NULL,verified_at=NULL,verification_nonce=NULL,updated_at=now() WHERE id=$1").bind(id).execute(&mut *tx).await?;
        audit(
            &mut tx,
            app,
            None,
            "ZIP_DISTRIBUTION_REJECT",
            json!({"jobId":id,"errorCode":"POLICY_CHANGED"}),
        )
        .await?;
        tx.commit().await?;
        return Ok(None);
    }
    let group = group.ok_or(AppError::Unavailable)?;
    j.attempt += 1;
    j.nonce = Some(Uuid::new_v4());
    j.state = "RUNNING".into();
    sqlx::query("UPDATE zip_distribution_jobs SET state='RUNNING',attempt=$2,nonce=$3,lease_until=now()+interval '10 minutes',verification_nonce=NULL,verified_at=NULL,error_code=NULL,updated_at=now() WHERE id=$1").bind(id).bind(j.attempt).bind(j.nonce).execute(&mut *tx).await?;
    sqlx::query("UPDATE zip_distribution_nodes SET state='PENDING',attempt=$2,receipt=NULL,error_code=NULL,verified_at=NULL WHERE job_id=$1").bind(id).bind(j.attempt).execute(&mut *tx).await?;
    audit(
        &mut tx,
        app,
        None,
        "ZIP_DISTRIBUTION_CLAIM",
        json!({"jobId":id,"attempt":j.attempt,"nonce":j.nonce}),
    )
    .await?;
    tx.commit().await?;
    Ok(Some((j, group)))
}
pub(super) async fn guarded(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    j: &Job,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar("SELECT state='RUNNING' AND attempt=$2 AND nonce=$3 AND lease_until>now() FROM zip_distribution_jobs WHERE id=$1 FOR UPDATE").bind(j.id).bind(j.attempt).bind(j.nonce).fetch_one(&mut **tx).await?)
}
async fn save_node(
    state: &AppState,
    j: &Job,
    node: &str,
    result: Result<Receipt, &'static str>,
) -> Result<bool, AppError> {
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, j.application_id).await?;
    if !guarded(&mut tx, j).await? {
        return Ok(false);
    }
    let (receipt, error) = match result {
        Ok(r) => (Some(json!(r)), None),
        Err(e) => (None, Some(e)),
    };
    sqlx::query("UPDATE zip_distribution_nodes SET state=$3,receipt=$4,error_code=$5,verified_at=CASE WHEN $5::text IS NULL THEN now() ELSE NULL END,attempt=$6 WHERE job_id=$1 AND node_id=$2")
        .bind(j.id).bind(node).bind(if error.is_none(){"READY"}else{"FAILED"}).bind(&receipt).bind(error).bind(j.attempt).execute(&mut *tx).await?;
    audit(
        &mut tx,
        j.application_id,
        None,
        "ZIP_DISTRIBUTION_NODE",
        json!({"jobId":j.id,"nodeId":node,"attempt":j.attempt,"receipt":receipt,"errorCode":error}),
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}
async fn tick(state: &AppState) -> Result<(), AppError> {
    let Some((j, group)) = claim(state).await? else {
        return Ok(());
    };
    let manifest = j.manifest()?;
    let body = transport::payload(state, &j, &manifest).await;
    for node in &group.policy.nodes {
        let result = if let Ok(body) = &body {
            let work = j.work(&node.id, j.nonce.ok_or(AppError::Unavailable)?);
            transport::node(&group, &work, &manifest, Some(body.clone())).await
        } else {
            Err("SOURCE_UNAVAILABLE")
        };
        if !save_node(state, &j, &node.id, result).await? {
            return Ok(());
        }
    }
    let mut tx = state.pool.begin().await?;
    lock(&mut tx, j.application_id).await?;
    if !guarded(&mut tx, &j).await? {
        return Ok(());
    }
    let rows:Vec<(String,String,Option<Value>)>=sqlx::query_as("SELECT node_id,state,receipt FROM zip_distribution_nodes WHERE job_id=$1 ORDER BY node_id FOR SHARE").bind(j.id).fetch_all(&mut *tx).await?;
    let complete = rows.len() == group.policy.nodes.len()
        && group.policy.nodes.iter().all(|n| {
            rows.iter().any(|(id, status, receipt)| {
                id == &n.id
                    && status == "READY"
                    && receipt.as_ref().is_some_and(|r| {
                        r["work"]["nonce"] == j.nonce.map(|v| v.to_string()).unwrap_or_default()
                    })
            })
        });
    let current = config::load(state)?;
    let same = current.groups.iter().any(|g| {
        g.policy.id == j.group_id
            && zip::revision(&g.policy).ok().as_ref() == Some(&j.policy_revision)
    });
    let ready = complete && same;
    sqlx::query("UPDATE zip_distribution_jobs SET state=$2,lease_until=NULL,verification_nonce=CASE WHEN $3 THEN nonce ELSE NULL END,verified_at=CASE WHEN $3 THEN now() ELSE NULL END,ever_ready=ever_ready OR $3,error_code=CASE WHEN $3 THEN NULL ELSE 'NODES_INCOMPLETE' END,updated_at=now() WHERE id=$1")
        .bind(j.id).bind(if ready{"READY"}else{"FAILED"}).bind(ready).execute(&mut *tx).await?;
    audit(&mut tx,j.application_id,None,if ready{"ZIP_DISTRIBUTION_READY"}else{"ZIP_DISTRIBUTION_FAILED"},json!({"jobId":j.id,"attempt":j.attempt,"allRequiredNodesReady":ready,"policyUnchanged":same})).await?;
    tx.commit().await?;
    Ok(())
}
