//! Optional control plane. Engine authority exists only in the independent agent.
mod config;
mod routes;
mod worker;
use crate::{error::AppError, releases::ReleaseRow};
pub(crate) use routes::routes;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Public policies let sibling delivery configuration refuse overlapping authority.
pub(crate) fn public_targets() -> Result<Vec<deployment_contract::Target>, AppError> {
    Ok(config::load()?
        .targets
        .into_iter()
        .map(|t| t.policy)
        .collect())
}

/// Classify the path as Nginx sees it; stored URLs and HTTP requests stay unchanged.
/// Percent-encoded or repeated separators must not bypass managed-resource gates.
pub(crate) fn canonical_entry(entry: &reqwest::Url) -> String {
    let decoded = percent_encoding::percent_decode_str(entry.path()).collect::<Vec<u8>>();
    let decoded = String::from_utf8_lossy(&decoded).replace('\\', "/");
    let trailing = decoded.ends_with('/') || decoded.ends_with("/.") || decoded.ends_with("/..");
    let mut segments = Vec::new();
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value),
        }
    }
    let mut path = format!("/{}", segments.join("/"));
    if trailing && !path.ends_with('/') {
        path.push('/');
    }
    format!("{}{}", entry.origin().ascii_serialization(), path)
}

/// A managed route owns every file beneath its immutable version directory,
/// including index.html aliases after its target is removed from configuration.
pub(crate) fn version_resource_root(entry: &str) -> Option<String> {
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

pub(super) async fn lock(tx: &mut Transaction<'_, Postgres>, app: Uuid) -> Result<(), AppError> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(app)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    Ok(())
}

/// Under the pointer transaction lock: a public HTTP probe cannot replace an agent receipt.
pub(crate) async fn require_ready(
    platform: &crate::state::AppState,
    tx: &mut Transaction<'_, Postgres>,
    row: &ReleaseRow,
) -> Result<(), AppError> {
    let policy = crate::releases::FetchPolicy::new(&platform.config, &row.environment)?;
    let entry = canonical_entry(&policy.resolve(&row.entry_url, None)?);
    let resource_root = version_resource_root(&entry).unwrap_or_else(|| entry.clone());
    let artifact:Option<(Uuid,String,String)>=sqlx::query_as("SELECT a.id,a.state,a.policy_revision FROM managed_image_artifacts a WHERE a.public_url=$1 OR a.id IN (SELECT artifact_id FROM managed_deployment_bindings WHERE release_id=$2) FOR SHARE")
        .bind(&resource_root).bind(row.id).fetch_optional(&mut **tx).await?;
    let config = config::load()?;
    let managed_url = config.targets.iter().any(|t| {
        t.policy.templates.iter().any(|template| {
            entry.starts_with(&format!(
                "{}/sub-apps/{}/versions/",
                t.policy.public_origin, template.application_code
            ))
        })
    });
    let Some((artifact, state, revision)) = artifact else {
        return if managed_url {
            Err(AppError::conflict("受管版本尚无部署回执"))
        } else {
            Ok(())
        };
    };
    let ready:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_deployment_bindings b JOIN managed_image_artifacts a ON a.id=b.artifact_id JOIN managed_deployment_jobs j ON j.artifact_id=a.id AND j.release_id=b.release_id WHERE b.release_id=$1 AND b.artifact_id=$2 AND a.application_id=$3 AND a.public_url=$4 AND j.operation='DEPLOY' AND j.state='READY')")
        .bind(row.id).bind(artifact).bind(row.application_id).bind(&resource_root).fetch_one(&mut **tx).await?;
    let configured: Option<String> =
        sqlx::query_scalar("SELECT target_id FROM managed_image_artifacts WHERE id=$1")
            .bind(artifact)
            .fetch_optional(&mut **tx)
            .await?;
    let same = config.targets.iter().any(|t| {
        Some(&t.policy.id) == configured.as_ref()
            && deployment_contract::revision(&t.policy) == revision
    });
    if !ready || state != "READY" || !same {
        return Err(AppError::conflict(
            "受管版本未就绪、策略已变化或正在清理，不能激活/回滚",
        ));
    }
    Ok(())
}

pub(super) async fn cleanup_protected(
    tx: &mut Transaction<'_, Postgres>,
    artifact: Uuid,
    release: Uuid,
    exclude: Option<Uuid>,
) -> Result<bool, AppError> {
    let protected:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_image_artifacts a JOIN application_releases selected ON selected.id=$2 WHERE a.id=$1 AND (EXISTS(SELECT 1 FROM application_releases r JOIN application_deployments d ON d.active_release_id=r.id OR d.previous_release_id=r.id WHERE r.application_id=a.application_id AND r.version=a.version) OR EXISTS(SELECT 1 FROM application_releases r WHERE r.application_id=a.application_id AND r.version=a.version AND r.environment<>selected.environment) OR EXISTS(SELECT 1 FROM application_release_operations o WHERE o.application_id=a.application_id) OR EXISTS(SELECT 1 FROM managed_deployment_jobs j WHERE j.artifact_id=a.id AND j.state IN ('QUEUED','RUNNING') AND ($3::uuid IS NULL OR j.id<>$3)) OR (a.ever_ready AND (a.created_at>now()-interval '30 days' OR (SELECT count(*) FROM managed_image_artifacts newer WHERE newer.application_id=a.application_id AND newer.state<>'REMOVED' AND (newer.created_at,newer.id)>(a.created_at,a.id))<5))))")
        .bind(artifact).bind(release).bind(exclude).fetch_one(&mut **tx).await?;
    Ok(protected)
}

pub(super) async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    app: Uuid,
    user: Option<&crate::extractors::CurrentUser>,
    action: &str,
    detail: &serde_json::Value,
) -> Result<(), AppError> {
    let safe = detail.to_string();
    crate::util::audit::write_in_tx_for_app(
        tx,
        crate::util::audit::AuditInfo {
            user_id: user.map(|u| u.user_id),
            username: user.map_or("deployment-agent", |u| u.username.as_str()),
            tenant_id: user.map(|u| u.tenant_id),
            organization_id: user.map(|u| u.org_id),
            action,
            detail: &safe,
            ip: "internal",
            user_agent: "managed-deployment",
        },
        app,
    )
    .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_directory_and_files_share_the_managed_resource_identity() {
        for path in [
            "/sub-apps/stores/versions/v1/",
            "/sub-apps/stores/versions/v1/index.html",
            "/sub-apps/stores/versions/v1/assets/app.js",
            "/sub-apps/%73tores/versions/v1/%69ndex.html",
            "/sub-apps/stores/versions/v1/assets/../index.html",
        ] {
            let entry = reqwest::Url::parse(&format!("https://host.test{path}")).unwrap();
            assert_eq!(
                super::version_resource_root(&super::canonical_entry(&entry)),
                Some("https://host.test/sub-apps/stores/versions/v1/".into())
            );
        }
        assert_eq!(
            super::version_resource_root("https://host.test/sub-apps/stores/versions/v10/"),
            Some("https://host.test/sub-apps/stores/versions/v10/".into())
        );
        assert!(super::version_resource_root("https://host.test/external/index.html").is_none());
    }

    #[test]
    fn gateway_path_aliases_share_the_managed_identity() {
        let expected = "https://host.test/sub-apps/stores/versions/1.0.0/";
        for path in [
            "/sub-apps/stores/versions/1.0.0/",
            "/sub-apps/%73tores/versions/1.0.0/",
            "/sub-apps//stores/versions/1.0.0/",
            "/sub-apps/stores%2fversions/1.0.0/",
            "/sub-apps/stores/extra%2f..%2fversions/1.0.0/",
        ] {
            let entry = reqwest::Url::parse(&format!("https://host.test{path}")).unwrap();
            assert_eq!(super::canonical_entry(&entry), expected);
        }
        let other =
            reqwest::Url::parse("https://host.test/sub-apps/other/versions/1.0.0/").unwrap();
        assert_ne!(super::canonical_entry(&other), expected);
    }
}
