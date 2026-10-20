//! Controlled data-grant management. HTTP parsing and SQL are kept at their
//! boundaries; this module owns authorization, optimistic versions and diffs.

mod discovery;
pub(crate) mod policy;
mod store;

pub(crate) use discovery::{OptionsQuery, OptionsResponse};

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    error::AppError,
    extractors::CurrentUser,
    state::AppState,
    util::{audit, hash},
};
use policy::{Capability, GrantScope, Policy, READ_PERMISSION, WRITE_PERMISSION};
use store::{GrantRow, Runtime};

const MAX_GRANTS: usize = 128;
const MAX_ORGANIZATIONS: usize = 10_000;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Discover only targets readable through the same controlled grant boundary.
pub(crate) async fn options(
    state: &AppState,
    actor: &CurrentUser,
    query: OptionsQuery,
) -> Result<OptionsResponse, AppError> {
    discovery::options(state, actor, query).await
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Target {
    pub tenant_id: Uuid,
    pub organization_id: Uuid,
    pub user_id: Uuid,
    pub membership_id: Uuid,
    pub application_id: Uuid,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GrantKey {
    resource: String,
    action: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Upsert {
    resource: String,
    action: String,
    scope: GrantScope,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ChangeRequest {
    target: Target,
    expected_version: String,
    upsert: Vec<Upsert>,
    remove: Vec<GrantKey>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrantView {
    /// Null only for a proposed insertion in a preview, never for stored rows.
    id: Option<Uuid>,
    resource: String,
    action: String,
    scope: GrantScope,
    effective_scope: GrantScope,
    organization_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    target: Target,
    version: String,
    grants: Vec<GrantView>,
    capabilities: Vec<Capability>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Changes {
    added: Vec<GrantView>,
    updated: Vec<GrantView>,
    removed: Vec<GrantView>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Preview {
    target: Target,
    version: String,
    before: Vec<GrantView>,
    after: Vec<GrantView>,
    changes: Changes,
}

#[derive(Clone)]
struct DraftGrant {
    id: Option<Uuid>,
    resource: String,
    action: String,
    scope: GrantScope,
}

struct Plan {
    after: Vec<DraftGrant>,
    added: Vec<DraftGrant>,
    updated: Vec<DraftGrant>,
    removed: Vec<DraftGrant>,
}

#[derive(Clone, Copy)]
enum Operation {
    Read,
    Preview,
    Write,
}

impl Operation {
    fn needs_read(self) -> bool {
        matches!(self, Self::Read | Self::Preview)
    }
    fn needs_write(self) -> bool {
        matches!(self, Self::Write | Self::Preview)
    }
}

/// Read a current, explicitly manageable direct member/application snapshot.
pub(crate) async fn read(
    state: &AppState,
    actor: &CurrentUser,
    target: Target,
) -> Result<Snapshot, AppError> {
    validate_target(&target)?;
    preliminary_access(state, actor, Operation::Read).await?;
    let policy = &state.config.data_grant_policy;
    let registered = policy_permissions(policy, &target)?;
    let mut tx = state.pool.begin().await?;
    store::lock_target(&mut tx, actor, &target).await?;
    let rows = store::grants(&mut tx, &target).await?;
    let runtime = store::runtime(&mut tx, actor, &target, &registered).await?;
    authorize(actor, &target, policy, &runtime, Operation::Read)?;
    let snapshot = snapshot(&target, &rows, policy, &runtime)?;
    ensure_size(&snapshot)?;
    tx.commit().await?;
    Ok(snapshot)
}

/// Preview uses exactly the write policy and optimistic conflict check. It
/// additionally requires read permission/boundaries and never mutates storage.
pub(crate) async fn preview(
    state: &AppState,
    actor: &CurrentUser,
    request: ChangeRequest,
) -> Result<Preview, AppError> {
    validate_request(&request)?;
    preliminary_access(state, actor, Operation::Preview).await?;
    let policy = &state.config.data_grant_policy;
    let registered = policy_permissions(policy, &request.target)?;
    let mut tx = state.pool.begin().await?;
    store::lock_target(&mut tx, actor, &request.target).await?;
    let rows = store::grants(&mut tx, &request.target).await?;
    let runtime = store::runtime(&mut tx, actor, &request.target, &registered).await?;
    authorize(actor, &request.target, policy, &runtime, Operation::Preview)?;
    let before = snapshot(&request.target, &rows, policy, &runtime)?;
    check_version(&request.expected_version, &before.version)?;
    validate_pairs(&request, policy, &runtime)?;
    let plan = plan(&rows, &request)?;
    let result = Preview {
        target: request.target.clone(),
        version: before.version,
        before: before.grants,
        after: views(&request.target, &plan.after, &runtime)?,
        changes: Changes {
            added: views(&request.target, &plan.added, &runtime)?,
            updated: views(&request.target, &plan.updated, &runtime)?,
            removed: views(&request.target, &plan.removed, &runtime)?,
        },
    };
    ensure_size(&result)?;
    tx.commit().await?;
    Ok(result)
}

/// Apply only the requested delta, preserving membership and retained grant IDs.
/// The final authorization check, result budget and audit precede one commit.
pub(crate) async fn change(
    state: &AppState,
    actor: &CurrentUser,
    request: ChangeRequest,
    ip: &str,
    user_agent: &str,
) -> Result<Snapshot, AppError> {
    validate_request(&request)?;
    preliminary_access(state, actor, Operation::Write).await?;
    let policy = &state.config.data_grant_policy;
    let registered = policy_permissions(policy, &request.target)?;
    let mut tx = state.pool.begin().await?;
    store::lock_target(&mut tx, actor, &request.target).await?;
    let rows = store::grants(&mut tx, &request.target).await?;
    let runtime = store::runtime(&mut tx, actor, &request.target, &registered).await?;
    authorize(actor, &request.target, policy, &runtime, Operation::Write)?;
    let before = snapshot(&request.target, &rows, policy, &runtime)?;
    check_version(&request.expected_version, &before.version)?;
    validate_pairs(&request, policy, &runtime)?;
    let plan = plan(&rows, &request)?;
    if plan.added.is_empty() && plan.updated.is_empty() && plan.removed.is_empty() {
        ensure_size(&before)?;
        tx.commit().await?;
        return Ok(before);
    }
    // A preview is not required by the protocol, so enforce the same response
    // budget here before allocating/inserting a potentially oversized result.
    let proposed = views(&request.target, &plan.after, &runtime)?;
    ensure_size(&proposed)?;
    for added in &plan.added {
        store::insert(
            &mut tx,
            &request.target,
            &added.resource,
            &added.action,
            added.scope.as_str(),
        )
        .await?;
    }
    for updated in &plan.updated {
        let id = updated
            .id
            .ok_or_else(|| AppError::Server(anyhow::anyhow!("stored grant has no ID")))?;
        store::update(&mut tx, id, updated.scope.as_str()).await?;
    }
    for removed in &plan.removed {
        let id = removed
            .id
            .ok_or_else(|| AppError::Server(anyhow::anyhow!("stored grant has no ID")))?;
        store::remove(&mut tx, id).await?;
    }
    // FK/row waits can outlast expiry or a concurrent management revocation.
    // Re-read actual state after those waits, never reuse the route extractor.
    let final_runtime = store::runtime(&mut tx, actor, &request.target, &registered).await?;
    authorize(
        actor,
        &request.target,
        policy,
        &final_runtime,
        Operation::Write,
    )?;
    validate_pairs(&request, policy, &final_runtime)?;
    if runtime.fingerprint != final_runtime.fingerprint {
        return Err(AppError::conflict("授权边界已变化，请重新查询和预览"));
    }
    let after_rows = store::grants(&mut tx, &request.target).await?;
    if signatures(&drafts(&after_rows)?) != signatures(&plan.after) {
        return Err(AppError::conflict("数据授权未按预期保存，请重新查询"));
    }
    let after = snapshot(&request.target, &after_rows, policy, &final_runtime)?;
    ensure_size(&after)?;
    let detail = audit::change_detail(
        "membership_data_grants",
        request.target.membership_id,
        json!({"target":request.target,"version":before.version,"grants":audit_rows(&rows)}),
        json!({"target":request.target,"version":after.version,"grants":audit_rows(&after_rows)}),
    );
    audit::write_in_tx_for_app(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(actor.user_id),
            username: &actor.username,
            tenant_id: Some(request.target.tenant_id),
            organization_id: Some(request.target.organization_id),
            action: "DATA_GRANT_CHANGE",
            detail: &detail,
            ip,
            user_agent,
        },
        request.target.application_id,
    )
    .await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(after)
}

async fn preliminary_access(
    state: &AppState,
    actor: &CurrentUser,
    operation: Operation,
) -> Result<(), AppError> {
    // Early gates avoid disclosing target existence to a caller with no grant
    // permission. They are repeated from the transaction after all lock waits.
    if operation.needs_read() {
        state.require(actor, READ_PERMISSION).await?;
    }
    if operation.needs_write() {
        state.require_platform(actor).await?;
        state.require(actor, WRITE_PERMISSION).await?;
    }
    Ok(())
}

fn authorize(
    actor: &CurrentUser,
    target: &Target,
    policy: &Policy,
    runtime: &Runtime,
    operation: Operation,
) -> Result<(), AppError> {
    for (needed, permission, write) in [
        (operation.needs_read(), READ_PERMISSION, false),
        (operation.needs_write(), WRITE_PERMISSION, true),
    ] {
        if !needed {
            continue;
        }
        if !runtime.permissions.iter().any(|value| value == permission) {
            return Err(AppError::forbidden("缺少独立数据授权管理权限"));
        }
        if write && !runtime.actor_platform_root {
            return Err(AppError::forbidden("仅平台租户根组织可修改数据授权"));
        }
        if !write
            && !runtime.actor_platform_root
            && (actor.tenant_id != target.tenant_id
                || !runtime.actor_subtree.contains(&target.organization_id))
        {
            return Err(AppError::forbidden("目标成员超出当前组织只读管理范围"));
        }
        let allowed = policy.management_scopes.iter().any(|boundary| {
            boundary.tenant_id == target.tenant_id
                && (boundary.organization_id == target.organization_id
                    || (boundary.include_descendants
                        && runtime.target_ancestors.contains(&boundary.organization_id)))
                && if write { boundary.write } else { boundary.read }
        });
        if !allowed {
            return Err(AppError::forbidden("目标成员超出显式数据授权管理范围"));
        }
    }
    Ok(())
}

fn validate_target(target: &Target) -> Result<(), AppError> {
    if [
        target.tenant_id,
        target.organization_id,
        target.user_id,
        target.membership_id,
        target.application_id,
    ]
    .iter()
    .any(Uuid::is_nil)
    {
        return Err(AppError::bad_request("目标标识不能是空 UUID"));
    }
    Ok(())
}

fn validate_request(request: &ChangeRequest) -> Result<(), AppError> {
    validate_target(&request.target)?;
    if request.expected_version.len() != 64
        || !request
            .expected_version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(AppError::bad_request(
            "expectedVersion 必须是查询结果中的版本字符串",
        ));
    }
    let count = request.upsert.len().saturating_add(request.remove.len());
    if count == 0 || count > MAX_GRANTS {
        return Err(AppError::bad_request("差量操作总数必须在 1 到 128 之间"));
    }
    let mut keys = BTreeSet::new();
    for (resource, action) in request
        .upsert
        .iter()
        .map(|entry| (&entry.resource, &entry.action))
        .chain(
            request
                .remove
                .iter()
                .map(|entry| (&entry.resource, &entry.action)),
        )
    {
        if resource.is_empty()
            || resource.len() > 100
            || action.is_empty()
            || action.len() > 100
            || !keys.insert((resource, action))
        {
            return Err(AppError::bad_request(
                "资源动作必须有效且不能重复或同时增删",
            ));
        }
    }
    Ok(())
}

fn policy_permissions(policy: &Policy, target: &Target) -> Result<Vec<String>, AppError> {
    let application = policy
        .applications
        .iter()
        .find(|entry| entry.application_id == target.application_id)
        .ok_or_else(|| AppError::forbidden("目标应用未登记数据授权管理能力"))?;
    Ok(application
        .capabilities
        .iter()
        .map(|entry| format!("{}:{}", entry.resource, entry.action))
        .collect())
}

fn capabilities(policy: &Policy, target: &Target, runtime: &Runtime) -> Vec<Capability> {
    policy
        .applications
        .iter()
        .find(|entry| entry.application_id == target.application_id)
        .map(|entry| {
            entry
                .capabilities
                .iter()
                .filter(|capability| {
                    runtime
                        .registered
                        .contains(&format!("{}:{}", capability.resource, capability.action))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn validate_pairs(
    request: &ChangeRequest,
    policy: &Policy,
    runtime: &Runtime,
) -> Result<(), AppError> {
    let allowed = capabilities(policy, &request.target, runtime);
    for entry in &request.upsert {
        let capability = allowed
            .iter()
            .find(|capability| {
                capability.resource == entry.resource && capability.action == entry.action
            })
            .ok_or_else(|| AppError::bad_request("资源动作未在目标应用登记"))?;
        if !capability.scopes.contains(&entry.scope) {
            return Err(AppError::bad_request("数据范围未在资源动作能力中明确允许"));
        }
    }
    for entry in &request.remove {
        if !allowed.iter().any(|capability| {
            capability.resource == entry.resource && capability.action == entry.action
        }) {
            return Err(AppError::bad_request("资源动作未在目标应用登记"));
        }
    }
    Ok(())
}

fn check_version(expected: &str, actual: &str) -> Result<(), AppError> {
    if expected != actual {
        return Err(AppError::conflict("数据授权版本已变化，请重新查询和预览"));
    }
    Ok(())
}

fn drafts(rows: &[GrantRow]) -> Result<Vec<DraftGrant>, AppError> {
    if rows.len() > MAX_GRANTS {
        return Err(AppError::bad_request("当前授权超过 128 项管理上限"));
    }
    rows.iter()
        .map(|row| {
            Ok(DraftGrant {
                id: Some(row.id),
                resource: row.resource.clone(),
                action: row.action.clone(),
                scope: GrantScope::from_str_value(&row.scope).ok_or_else(|| {
                    AppError::Server(anyhow::anyhow!("invalid stored grant scope"))
                })?,
            })
        })
        .collect()
}

fn plan(rows: &[GrantRow], request: &ChangeRequest) -> Result<Plan, AppError> {
    let mut after: BTreeMap<_, _> = drafts(rows)?
        .into_iter()
        .map(|entry| ((entry.resource.clone(), entry.action.clone()), entry))
        .collect();
    let mut result = Plan {
        after: Vec::new(),
        added: Vec::new(),
        updated: Vec::new(),
        removed: Vec::new(),
    };
    for entry in &request.upsert {
        let key = (entry.resource.clone(), entry.action.clone());
        if let Some(current) = after.get_mut(&key) {
            if current.scope != entry.scope {
                current.scope = entry.scope;
                result.updated.push(current.clone());
            }
        } else {
            let added = DraftGrant {
                id: None,
                resource: entry.resource.clone(),
                action: entry.action.clone(),
                scope: entry.scope,
            };
            result.added.push(added.clone());
            after.insert(key, added);
        }
    }
    for entry in &request.remove {
        if let Some(removed) = after.remove(&(entry.resource.clone(), entry.action.clone())) {
            result.removed.push(removed);
        }
    }
    if after.len() > MAX_GRANTS {
        return Err(AppError::bad_request("变更后授权不能超过 128 项"));
    }
    result.after = after.into_values().collect();
    for entries in [&mut result.added, &mut result.updated, &mut result.removed] {
        entries.sort_unstable_by(|left, right| {
            (&left.resource, &left.action).cmp(&(&right.resource, &right.action))
        });
    }
    Ok(result)
}

fn signatures(entries: &[DraftGrant]) -> Vec<(&str, &str, &str)> {
    entries
        .iter()
        .map(|entry| {
            (
                entry.resource.as_str(),
                entry.action.as_str(),
                entry.scope.as_str(),
            )
        })
        .collect()
}

fn views(
    target: &Target,
    entries: &[DraftGrant],
    runtime: &Runtime,
) -> Result<Vec<GrantView>, AppError> {
    let mut minimum_bytes = 0usize;
    entries
        .iter()
        .map(|entry| {
            let effective_scope =
                if entry.scope == GrantScope::Tenant && !runtime.target_platform_root {
                    GrantScope::OrgSubtree
                } else {
                    entry.scope
                };
            let organization_ids = match effective_scope {
                GrantScope::Org => std::slice::from_ref(&target.organization_id),
                GrantScope::OrgSubtree => runtime.subtree.as_slice(),
                GrantScope::SelfOnly if !runtime.target_platform_root => runtime.subtree.as_slice(),
                _ => &[],
            };
            minimum_bytes = minimum_bytes.saturating_add(organization_ids.len().saturating_mul(39));
            if minimum_bytes > MAX_RESPONSE_BYTES {
                return Err(AppError::bad_request("数据授权响应超过 1 MiB 上限"));
            }
            Ok(GrantView {
                id: entry.id,
                resource: entry.resource.clone(),
                action: entry.action.clone(),
                scope: entry.scope,
                effective_scope,
                organization_ids: organization_ids.to_vec(),
            })
        })
        .collect()
}

fn snapshot(
    target: &Target,
    rows: &[GrantRow],
    policy: &Policy,
    runtime: &Runtime,
) -> Result<Snapshot, AppError> {
    let version=hash::sha256_hex(&json!({"schema":1,"target":target,"grants":rows,"policy":policy,"runtime":runtime.fingerprint}).to_string());
    Ok(Snapshot {
        target: target.clone(),
        version,
        grants: views(target, &drafts(rows)?, runtime)?,
        capabilities: capabilities(policy, target, runtime),
    })
}

fn audit_rows(rows: &[GrantRow]) -> Vec<Value> {
    rows.iter().map(|row| json!({"id":row.id,"resource":row.resource,"action":row.action,"scope":row.scope})).collect()
}

fn ensure_size(value: &impl Serialize) -> Result<(), AppError> {
    struct Limit(usize);
    impl Write for Limit {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.saturating_add(bytes.len()) > MAX_RESPONSE_BYTES {
                return Err(io::Error::other("response limit"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Limit(0), value)
        .map_err(|_| AppError::bad_request("数据授权响应超过 1 MiB 上限"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_budget_counts_json_encoding_and_accepts_the_exact_limit() {
        // JSON strings add two quote bytes. The budget counts serialized wire
        // bytes, so neither character length nor the in-memory type is enough.
        assert!(ensure_size(&"x".repeat(MAX_RESPONSE_BYTES - 2)).is_ok());
        assert!(matches!(
            ensure_size(&"x".repeat(MAX_RESPONSE_BYTES - 1)),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            ensure_size(&"\n".repeat(MAX_RESPONSE_BYTES / 2)),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn effective_scope_uses_the_target_boundary_even_for_a_platform_actor() {
        let target = Target {
            tenant_id: Uuid::from_u128(1),
            organization_id: Uuid::from_u128(2),
            user_id: Uuid::from_u128(3),
            membership_id: Uuid::from_u128(4),
            application_id: Uuid::from_u128(5),
        };
        let mut runtime = Runtime {
            actor_platform_root: true,
            target_platform_root: false,
            permissions: Vec::new(),
            target_ancestors: vec![target.organization_id],
            subtree: vec![target.organization_id, Uuid::from_u128(6)],
            actor_subtree: Vec::new(),
            registered: BTreeSet::new(),
            fingerprint: Value::Null,
        };
        let grant = DraftGrant {
            id: Some(Uuid::from_u128(7)),
            resource: "ticket".into(),
            action: "read".into(),
            scope: GrantScope::Tenant,
        };
        let business = views(&target, std::slice::from_ref(&grant), &runtime).unwrap();
        assert_eq!(business[0].scope, GrantScope::Tenant);
        assert_eq!(business[0].effective_scope, GrantScope::OrgSubtree);
        assert_eq!(business[0].organization_ids, runtime.subtree);
        assert_eq!(business[0].id, grant.id);
        runtime.target_platform_root = true;
        let platform = views(&target, &[grant], &runtime).unwrap();
        assert_eq!(platform[0].effective_scope, GrantScope::Tenant);
        assert!(platform[0].organization_ids.is_empty());
    }
}
