//! Offline transforms for registered UUIDs embedded in otherwise non-UUID data.
//! Source rows remain in the encrypted evidence bundle. A transformed audit is
//! only a historical projection; it is never a newly signed receipt or proof.

mod audit;
mod tasks;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::mapping::TypedMap;
use crate::{Error, Result};

/// Reject duplicate keys at every depth before constructing a Value. The first
/// pass deliberately does not convert numbers; Value's arbitrary-precision
/// second pass therefore preserves integer and decimal precision.
pub(super) fn parse_strict(raw: &str) -> Result<Value> {
    let mut parser = serde_json::Deserializer::from_str(raw);
    NoDuplicates::deserialize(&mut parser)
        .map_err(|_| Error("invalid or duplicate-key JSON evidence"))?;
    parser
        .end()
        .map_err(|_| Error("invalid or duplicate-key JSON evidence"))?;
    serde_json::from_str(raw).map_err(|_| Error("invalid JSON evidence"))
}

struct NoDuplicates;

/// Contains private source identities and intentionally has no Debug output.
/// Observation is order independent: audit creation proves manual provenance,
/// while an actual regions row proves that this entity is still present.
pub(crate) struct SourceCollector {
    region_rows: BTreeSet<Uuid>,
    region_creates: BTreeMap<Uuid, String>,
    region_deletes: BTreeMap<Uuid, String>,
    audit_proofs: Vec<(String, Value, String)>,
}

pub(crate) fn needs_source_collection(kind: &str, table: &str) -> bool {
    let table = table.strip_prefix("public.").unwrap_or(table);
    kind == "platform" && matches!(table, "regions" | "audit_logs")
}

impl SourceCollector {
    pub(crate) fn new() -> Self {
        Self {
            region_rows: BTreeSet::new(),
            region_creates: BTreeMap::new(),
            region_deletes: BTreeMap::new(),
            audit_proofs: Vec::new(),
        }
    }

    pub(crate) fn observe(
        &mut self,
        kind: &str,
        table: &str,
        row: &Value,
        evidence_sha256: &str,
    ) -> Result<()> {
        let table = table.strip_prefix("public.").unwrap_or(table);
        if !needs_source_collection(kind, table) {
            return Ok(());
        }
        if !is_digest(evidence_sha256) {
            return Err(Error("invalid source ownership evidence digest"));
        }
        if table == "regions" {
            let raw = string(row, "source_id")?;
            if let Some(id) = compact_candidate(raw)? {
                self.region_rows.insert(id);
            }
            return Ok(());
        }
        let action = string(row, "action")?;
        let raw = string(row, "detail")?;
        if !raw.trim_start().starts_with('{') {
            return Ok(());
        }
        let detail = parse_strict(raw)?;
        if matches!(action, "CREATE_REGION" | "DELETE_REGION") && detail["objectType"] == "region" {
            validate_region_audit_shape(action, &detail)?;
            if let Some(id) = compact_candidate(string(&detail, "objectId")?)? {
                let proofs = if action == "CREATE_REGION" {
                    &mut self.region_creates
                } else {
                    &mut self.region_deletes
                };
                proofs
                    .entry(id)
                    .or_insert_with(|| evidence_sha256.to_owned());
            }
        } else if matches!(
            action,
            "DELETE_APPLICATION"
                | "DELETE_DEPARTMENT"
                | "DELETE_MENU"
                | "DELETE_ROLE"
                | "DELETE_TENANT"
                | "DELETE_ORGANIZATION"
                | "DELETE_USER"
                | "DELETE_CLIENT"
                | "CONTEXT_SWITCH"
                | "LOGOUT"
                | "REVOKE_SESSION"
                | "CONSENT_APPROVE"
                | "CONSENT_DENY"
                | "TOKEN_REVOKED"
                | "TOKEN_ISSUED"
                | "DEVICE_APPROVE"
                | "DEVICE_DENY"
        ) {
            // Closed owner validation is deliberately deferred until finish,
            // where the frozen registry and all ordinary source PKs exist.
            self.audit_proofs
                .push((action.to_owned(), detail, evidence_sha256.to_owned()));
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self, map: &mut TypedMap, registry: &Value) -> Result<()> {
        require_handler(registry, "manual-region-source")?;
        require_handler(registry, "audit-region-source")?;
        for id in &self.region_rows {
            if !self.region_creates.contains_key(id) {
                return Err(Error("compact region source lacks manual ownership proof"));
            }
            // regions has no trusted per-source created_at; the frozen batch
            // timestamp is explicitly recorded by TypedMap's allocator.
            map.allocate("platform.manual_region_source", *id, None)?;
        }
        for id in self.region_creates.keys() {
            if map
                .resolve_historical("platform.manual_region_source", *id)
                .is_ok()
            {
                continue;
            }
            let proof = self
                .region_deletes
                .get(id)
                .ok_or(Error("historical manual region lacks deletion proof"))?;
            map.attest_tombstone(
                "platform.manual_region_source",
                *id,
                None,
                proof,
                "deleted_source_entity",
            )?;
        }
        for (action, detail, proof) in &self.audit_proofs {
            if let Some((authority, id)) = audit::deletion_proof(action, detail, registry)? {
                if map.resolve_historical(authority, id).is_err() {
                    map.attest_tombstone(authority, id, None, proof, "deleted_source_entity")?;
                }
            }
            for (authority, id) in audit::credential_proofs(action, detail, registry)? {
                if map.resolve_historical(authority, id).is_err() {
                    map.attest_tombstone(authority, id, None, proof, "excluded_credential")?;
                }
            }
        }
        Ok(())
    }
}

fn compact_candidate(raw: &str) -> Result<Option<Uuid>> {
    if raw.len() != 32 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let id = Uuid::parse_str(raw).map_err(|_| Error("invalid compact source identity"))?;
    if id.is_nil() || id.get_variant() != uuid::Variant::RFC4122 {
        return Err(Error("invalid compact source identity"));
    }
    Ok(Some(id))
}

fn is_digest(raw: &str) -> bool {
    raw.len() == 64
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl<'de> Deserialize<'de> for NoDuplicates {
    fn deserialize<D: de::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Check;
        impl<'de> Visitor<'de> for Check {
            type Value = NoDuplicates;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON without duplicate object keys")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut keys = BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key) {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                    map.next_value::<NoDuplicates>()?;
                }
                Ok(NoDuplicates)
            }
            fn visit_seq<S: SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> std::result::Result<Self::Value, S::Error> {
                while seq.next_element::<NoDuplicates>()?.is_some() {}
                Ok(NoDuplicates)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates)
            }
            fn visit_string<E: de::Error>(self, _: String) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates)
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates)
            }
            fn visit_none<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates)
            }
        }
        deserializer.deserialize_any(Check)
    }
}

/// Mutations commit to the caller only after all owner invariants pass.
pub(crate) fn transform_row(
    kind: &str,
    table: &str,
    row: &mut Value,
    map: &TypedMap,
    registry: &Value,
) -> Result<()> {
    let table = table.strip_prefix("public.").unwrap_or(table);
    if !row.is_object() {
        return Err(Error("structured row is not an object"));
    }
    let mut candidate = row.clone();
    tasks::transform(kind, table, &mut candidate, map, registry)?;
    match (kind, table) {
        ("platform", "audit_logs") => transform_audit(&mut candidate, map, registry)?,
        ("platform", "regions") => {
            require_handler(registry, "manual-region-source")?;
            let source = string(&candidate, "source_id")?;
            candidate["source_id"] = Value::String(region_id(source, map, false)?);
        }
        ("platform", "notification_publications") => publication(&mut candidate, map, registry)?,
        ("work_orders", "work_order_reminder_intents") => reminder(&mut candidate, map, registry)?,
        ("work_orders", "work_order_create_idempotency") => {
            require_handler(registry, "work-order-create-response")?;
            let response = string(&candidate, "response_json")?;
            let value = parse_strict(response)?;
            validate_create_response(&value)?;
            // The receipt stores no original Create DTO. Current ticket state
            // and a response snapshot are not a registered original request.
            return Err(Error("original work-order create request unavailable"));
        }
        _ => {}
    }
    if kind == "platform" {
        let route_columns: &[&str] = match table {
            "notifications" => &["link"],
            "menus" => &["path", "link_url"],
            "applications" | "application_url_entries" => &["entry_url"],
            "application_releases" => &["entry_url", "health_url"],
            "users" => &["avatar"],
            "platform_settings" => &["logo_url"],
            _ => &[],
        };
        for column in route_columns {
            if let Some(Value::String(original)) = candidate.get(*column) {
                // The work-orders bootstrap manifest registers a router pattern
                // here. Its parameter names are menu configuration, not entity
                // references; concrete notification/reminder links still go
                // through rewrite_route's UUID and mapping checks.
                if table == "menus"
                    && *column == "path"
                    && original == "/micro/work-orders/tickets/:id"
                {
                    require_route(registry, "other-links")?;
                    continue;
                }
                let rewritten = rewrite_route(original, map, registry)?;
                candidate[*column] = Value::String(rewritten);
            } else if candidate.get(*column).is_some_and(|value| !value.is_null()) {
                return Err(Error("registered URL column is not text"));
            }
        }
    }
    *row = candidate;
    Ok(())
}

fn transform_audit(row: &mut Value, map: &TypedMap, registry: &Value) -> Result<()> {
    require_handler(registry, "audit-change-envelope")?;
    let action = string(row, "action")?;
    let original = string(row, "detail")?;
    let first = original.trim_start().chars().next();
    if first != Some('{') {
        // Only the owner's known text producers are accepted. Their exact
        // bytes remain evidence; arbitrary legacy text needs an attestation.
        let legacy_owner = registry["structuredReferences"]
            .as_array()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry["id"] == "audit-legacy-text")
            });
        if legacy_owner
            .and_then(|owner| owner["actions"].as_array())
            .is_some_and(|actions| {
                actions
                    .iter()
                    .any(|registered| registered.as_str() == Some(action))
            })
        {
            return Ok(());
        }
        return Err(Error("unregistered legacy audit producer"));
    }
    let mut detail = parse_strict(original)?;
    if action == "APPLICATION_SETUP_ACCESS" {
        application_setup(&mut detail, map, registry)?;
    } else if !tasks::audit(action, &mut detail, map)? {
        match detail.get("objectType").and_then(Value::as_str) {
            Some("region" | "regionVersion") => region_audit(action, &mut detail, map, registry)?,
            _ => audit::transform(action, &mut detail, map, registry)?,
        }
    }
    row["detail"] = Value::String(encode(&detail)?);
    Ok(())
}

fn application_setup(detail: &mut Value, map: &TypedMap, registry: &Value) -> Result<()> {
    require_handler(registry, "audit-application-setup")?;
    closed(detail, &["target", "menuIds", "added", "mode"], &[])?;
    if detail["mode"] != "ADDITIVE" || !detail["added"].as_u64().is_some_and(|count| count > 0) {
        return Err(Error("invalid application setup audit mode"));
    }
    closed(
        &detail["target"],
        &["tenantId", "organizationId", "roleId"],
        &[],
    )?;
    for (field, authority) in [
        ("tenantId", "platform.tenant"),
        ("organizationId", "platform.organization"),
        ("roleId", "platform.role"),
    ] {
        map_value(&mut detail["target"], field, authority, map, true)?;
    }
    let mut seen = BTreeSet::new();
    for menu in detail["menuIds"]
        .as_array_mut()
        .ok_or(Error("invalid application setup menu list"))?
    {
        let old = parse_uuid(
            menu.as_str()
                .ok_or(Error("invalid application setup menu reference"))?,
        )?;
        if !seen.insert(old) {
            return Err(Error("duplicate application setup menu reference"));
        }
        *menu = Value::String(map.resolve_historical("platform.menu", old)?.to_string());
    }
    Ok(())
}

fn publication(row: &mut Value, map: &TypedMap, registry: &Value) -> Result<()> {
    require_handler(registry, "notification-publication-target")?;
    if !valid_key(string(row, "idempotency_key")?) {
        return Err(Error("invalid publication deduplication key"));
    }
    let original = row
        .get("request_body")
        .ok_or(Error("missing publication request body"))?;
    closed(
        original,
        &["target", "type", "title", "content", "link"],
        &[],
    )?;
    let canonical = encode(original)?;
    if string(row, "request_hash")? != sha256(canonical.as_bytes()) {
        return Err(Error("publication source hash mismatch"));
    }
    let title = string(original, "title")?;
    let content = string(original, "content")?;
    let notification_type = string(original, "type")?;
    if title.is_empty()
        || title.trim() != title
        || title.chars().count() > 200
        || title.contains('\0')
        || content.contains('\0')
        || content.chars().count() > 8192
        || !matches!(
            notification_type,
            "INFO" | "SUCCESS" | "WARNING" | "SECURITY"
        )
        || string(original, "link")?.chars().count() > 300
    {
        return Err(Error("invalid normalized publication request"));
    }
    let mut body = original.clone();
    let target = body
        .get_mut("target")
        .ok_or(Error("missing publication target"))?;
    match string(target, "kind")? {
        "USER" => {
            closed(target, &["kind", "userId", "organizationId"], &[])?;
            map_value(target, "userId", "platform.user", map, false)?;
            map_value(
                target,
                "organizationId",
                "platform.organization",
                map,
                false,
            )?;
        }
        "ORGANIZATION" => {
            closed(target, &["kind", "organizationId"], &[])?;
            map_value(
                target,
                "organizationId",
                "platform.organization",
                map,
                false,
            )?;
        }
        "TENANT" => closed(target, &["kind"], &[])?,
        _ => return Err(Error("unknown publication target discriminator")),
    }
    body["link"] = Value::String(rewrite_route(string(&body, "link")?, map, registry)?);
    let changed = encode(&body)?;
    row["request_body"] = body;
    row["request_hash"] = Value::String(sha256(changed.as_bytes()));
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reminder {
    idempotency_key: String,
    target: ReminderTarget,
    #[serde(rename = "type")]
    notification_type: String,
    title: String,
    content: String,
    link: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReminderTarget {
    kind: String,
    user_id: String,
    organization_id: String,
}

fn reminder(row: &mut Value, map: &TypedMap, registry: &Value) -> Result<()> {
    require_handler(registry, "work-order-reminder-request")?;
    let original = string(row, "request_json")?;
    if original.len() > 65_536 || sha256(original.as_bytes()) != string(row, "request_hash")? {
        return Err(Error("reminder source hash mismatch"));
    }
    let parsed = parse_strict(original)?;
    let mut body: Reminder =
        serde_json::from_value(parsed).map_err(|_| Error("unknown reminder payload schema"))?;
    let key = string(row, "idempotency_key")?;
    let number = body
        .title
        .strip_prefix("工单提醒 ")
        .ok_or(Error("invalid reminder content"))?;
    if !valid_key(key)
        || body.idempotency_key != key
        || body.target.kind != "USER"
        || body.notification_type != "INFO"
        || number.is_empty()
        || number.chars().count() > 40
        || number.contains('\0')
        || body.content != format!("请查看工单 {number} 的详情并处理。")
        || encode(&body)? != original
    {
        return Err(Error("invalid canonical reminder payload"));
    }
    let source_user = parse_uuid(&body.target.user_id)?;
    let source_org = parse_uuid(&body.target.organization_id)?;
    let source_ticket = body
        .link
        .strip_prefix("/micro/work-orders/tickets/")
        .ok_or(Error("unregistered reminder ticket route"))?;
    let source_ticket = parse_uuid(source_ticket)?;
    if body.link != format!("/micro/work-orders/tickets/{source_ticket}") {
        return Err(Error("noncanonical reminder ticket route"));
    }
    let user = map.resolve("platform.user", source_user)?;
    let org = map.resolve("platform.organization", source_org)?;
    let ticket = map.resolve("work_orders.ticket", source_ticket)?;
    matches_column(row, "recipient_user_id", source_user, user)?;
    matches_column(row, "target_organization_id", source_org, org)?;
    matches_column(row, "ticket_id", source_ticket, ticket)?;
    body.target.user_id = user.to_string();
    body.target.organization_id = org.to_string();
    body.link = rewrite_route(&body.link, map, registry)?;
    let updated = encode(&body)?;
    row["request_hash"] = Value::String(sha256(updated.as_bytes()));
    row["request_json"] = Value::String(updated);
    Ok(())
}

fn validate_create_response(value: &Value) -> Result<()> {
    closed(
        value,
        &[
            "id",
            "number",
            "title",
            "description",
            "priority",
            "status",
            "tenantId",
            "organizationId",
            "organizationName",
            "ownerUserId",
            "ownerDisplayName",
            "assigneeUserId",
            "assigneeDisplayName",
            "version",
            "createdAt",
            "updatedAt",
        ],
        &[],
    )?;
    for field in ["id", "tenantId", "organizationId", "ownerUserId"] {
        parse_uuid(string(value, field)?)?;
    }
    if !value["assigneeUserId"].is_null() {
        parse_uuid(string(value, "assigneeUserId")?)?;
    }
    Ok(())
}

fn region_id(source: &str, map: &TypedMap, historical: bool) -> Result<String> {
    if let Some(old) = compact_candidate(source)? {
        // Shape only selects the proof requirement. Only an independently
        // registered manual entity/tombstone can authorize this conversion.
        let new = if historical {
            map.resolve_historical("platform.manual_region_source", old)?
        } else {
            map.resolve("platform.manual_region_source", old)?
        };
        return Ok(new.simple().to_string());
    }
    if Uuid::parse_str(source).is_ok() {
        return Err(Error("unresolved region source provenance"));
    }
    Ok(source.to_owned())
}

fn region_audit(action: &str, detail: &mut Value, map: &TypedMap, registry: &Value) -> Result<()> {
    closed(detail, &["objectType", "objectId", "before", "after"], &[])?;
    let object = string(detail, "objectId")?.to_owned();
    match string(detail, "objectType")? {
        "regionVersion" => {
            require_handler(registry, "audit-region-version")?;
            let (side, empty, fields): (&str, &str, &[&str]) = match action {
                "CREATE_REGION_VERSION" => {
                    ("after", "before", &["version", "label", "sourceVersion"])
                }
                "DELETE_REGION_VERSION" => (
                    "before",
                    "after",
                    &["version", "label", "sourceVersion", "regionCount"],
                ),
                _ => return Err(Error("unknown region-version audit action")),
            };
            if !detail[empty].is_null() {
                return Err(Error("invalid region-version audit shape"));
            }
            closed(&detail[side], fields, &[])?;
            if string(&detail[side], "version")? != object
                || !detail[side]["label"].is_string()
                || !(detail[side]["sourceVersion"].is_null()
                    || detail[side]["sourceVersion"].is_string())
                || (action == "DELETE_REGION_VERSION" && !detail[side]["regionCount"].is_u64())
            {
                return Err(Error("invalid region-version audit shape"));
            }
        }
        "region" => {
            require_handler(registry, "audit-region-source")?;
            validate_region_audit_shape(action, detail)?;
            let (before, after) = match action {
                "CREATE_REGION" => (false, true),
                "UPDATE_REGION" => (true, true),
                "DELETE_REGION" => (true, false),
                _ => return Err(Error("unknown region audit action")),
            };
            let new = region_id(&object, map, true)?;
            for (side, present) in [("before", before), ("after", after)] {
                if !present {
                    if !detail[side].is_null() {
                        return Err(Error("invalid region audit shape"));
                    }
                    continue;
                }
                closed(
                    &detail[side],
                    &[
                        "id",
                        "version",
                        "code",
                        "parentCode",
                        "name",
                        "namePinyin",
                        "nameShort",
                        "level",
                        "type",
                        "typeName",
                        "postCode",
                        "sourceUrl",
                    ],
                    &[],
                )?;
                if string(&detail[side], "id")? != object {
                    return Err(Error("region audit identity mismatch"));
                }
                detail[side]["id"] = Value::String(new.clone());
            }
            detail["objectId"] = Value::String(new);
        }
        _ => return Err(Error("unknown region audit object")),
    }
    Ok(())
}

fn validate_region_audit_shape(action: &str, detail: &Value) -> Result<()> {
    closed(detail, &["objectType", "objectId", "before", "after"], &[])?;
    if string(detail, "objectType")? != "region" {
        return Err(Error("unknown region audit object"));
    }
    let object_id = string(detail, "objectId")?;
    let (before, after) = match action {
        "CREATE_REGION" => (false, true),
        "UPDATE_REGION" => (true, true),
        "DELETE_REGION" => (true, false),
        _ => return Err(Error("unknown region audit action")),
    };
    for (side, present) in [("before", before), ("after", after)] {
        let item = &detail[side];
        if !present {
            if !item.is_null() {
                return Err(Error("invalid region audit shape"));
            }
            continue;
        }
        closed(
            item,
            &[
                "id",
                "version",
                "code",
                "parentCode",
                "name",
                "namePinyin",
                "nameShort",
                "level",
                "type",
                "typeName",
                "postCode",
                "sourceUrl",
            ],
            &[],
        )?;
        if string(item, "id")? != object_id {
            return Err(Error("region audit identity mismatch"));
        }
        for key in [
            "version",
            "code",
            "parentCode",
            "name",
            "namePinyin",
            "nameShort",
        ] {
            string(item, key)?;
        }
        if !item["level"]
            .as_u64()
            .is_some_and(|level| (1..=6).contains(&level))
        {
            return Err(Error("invalid region audit level"));
        }
        for key in ["type", "typeName", "postCode", "sourceUrl"] {
            if !item[key].is_null() && !item[key].is_string() {
                return Err(Error("invalid region audit optional field"));
            }
        }
    }
    Ok(())
}

/// Match registered path segments, never replace text or arbitrary query values.
pub(super) fn rewrite_route(original: &str, map: &TypedMap, registry: &Value) -> Result<String> {
    if original.is_empty() {
        return Ok(String::new());
    }
    let path_end = original.find(['?', '#']).unwrap_or(original.len());
    let (path, suffix) = original.split_at(path_end);
    if let Some(source) = path.strip_prefix("/micro/work-orders/tickets/") {
        require_route(registry, "work-order-ticket-link")?;
        let old = parse_uuid(source)?;
        if contains_uuid(&decoded_for_detection(suffix)?) {
            return Err(Error("unregistered UUID URL query reference"));
        }
        let target = map.resolve("work_orders.ticket", old)?;
        return Ok(format!("/micro/work-orders/tickets/{target}{suffix}"));
    }
    if let Some(rest) = path.strip_prefix("/micro-proxy/") {
        require_route(registry, "platform-url-entry")?;
        let pieces: Vec<_> = rest.split('/').collect();
        if pieces.len() < 3
            || pieces[0].is_empty()
            || !pieces[0]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        {
            return Err(Error("invalid registered proxy entry route"));
        }
        let old = parse_uuid(pieces[1])?;
        if contains_uuid(&decoded_for_detection(suffix)?)
            || pieces[2..].iter().any(|piece| contains_uuid(piece))
        {
            return Err(Error("unregistered UUID proxy route reference"));
        }
        let new = map.resolve("platform.application_url_revision", old)?;
        return Ok(format!(
            "/micro-proxy/{}/{new}/{}{suffix}",
            pieces[0],
            pieces[2..].join("/")
        ));
    }
    let decoded = decoded_for_detection(original)?;
    if contains_uuid(&decoded) {
        // Absolute URLs with identity-looking segments need an explicit owner
        // declaration. Treating every HTTP URL as external would hide local
        // application references and stale identities.
        let opaque = registry
            .get("opaqueExternalUrlOrigins")
            .and_then(Value::as_array)
            .is_some_and(|origins| {
                reqwest::Url::parse(original).ok().is_some_and(|url| {
                    origins.iter().any(|origin| {
                        origin.as_str() == Some(url.origin().ascii_serialization().as_str())
                    })
                })
            });
        if !opaque {
            return Err(Error(
                "unregistered UUID-bearing URL requires owner handler",
            ));
        }
    }
    Ok(original.to_owned())
}

fn decoded_for_detection(value: &str) -> Result<String> {
    let mut current = value.to_owned();
    for _ in 0..8 {
        let bytes = current.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut index = 0;
        let mut changed = false;
        while index < bytes.len() {
            if bytes[index] == b'%' && index + 2 < bytes.len() {
                if let (Some(high), Some(low)) = (
                    (bytes[index + 1] as char).to_digit(16),
                    (bytes[index + 2] as char).to_digit(16),
                ) {
                    out.push((high * 16 + low) as u8);
                    index += 3;
                    changed = true;
                    continue;
                }
            }
            out.push(bytes[index]);
            index += 1;
        }
        if !changed {
            return Ok(current);
        }
        current = String::from_utf8(out).map_err(|_| Error("invalid encoded URL reference"))?;
    }
    Err(Error("URL encoding exceeds registered owner depth"))
}

fn contains_uuid(value: &str) -> bool {
    if value.as_bytes().windows(36).any(|window| {
        window[8] == b'-'
            && window[13] == b'-'
            && window[18] == b'-'
            && window[23] == b'-'
            && std::str::from_utf8(window).is_ok_and(|candidate| Uuid::parse_str(candidate).is_ok())
    }) {
        return true;
    }
    value
        .split(|character: char| !character.is_ascii_hexdigit() && character != '-')
        .any(|part| part.len() == 32 && Uuid::parse_str(part).is_ok())
}

fn closed(value: &Value, required: &[&str], optional: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .ok_or(Error("registered JSON object required"))?;
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err(Error("unknown registered JSON schema"));
    }
    Ok(())
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(Error("registered text field missing or invalid"))
}

fn parse_uuid(raw: &str) -> Result<Uuid> {
    if raw.len() != 36
        || raw.as_bytes().get(8) != Some(&b'-')
        || raw.as_bytes().get(13) != Some(&b'-')
        || raw.as_bytes().get(18) != Some(&b'-')
        || raw.as_bytes().get(23) != Some(&b'-')
    {
        return Err(Error("invalid source UUID syntax"));
    }
    let value = Uuid::parse_str(raw).map_err(|_| Error("invalid source UUID syntax"))?;
    if value.is_nil() || value.get_variant() != uuid::Variant::RFC4122 {
        return Err(Error("invalid source entity UUID variant"));
    }
    Ok(value)
}

fn map_value(
    value: &mut Value,
    field: &str,
    authority: &str,
    map: &TypedMap,
    historical: bool,
) -> Result<()> {
    let old = parse_uuid(string(value, field)?)?;
    let new = if historical {
        map.resolve_historical(authority, old)?
    } else {
        map.resolve(authority, old)?
    };
    value[field] = Value::String(new.to_string());
    Ok(())
}

fn matches_column(row: &Value, field: &str, old: Uuid, mapped: Uuid) -> Result<()> {
    let actual = parse_uuid(string(row, field)?)?;
    if actual != old && actual != mapped {
        return Err(Error("structured reference disagrees with row identity"));
    }
    Ok(())
}

fn require_handler(registry: &Value, id: &str) -> Result<()> {
    require_registration(registry, "structuredReferences", id)
}
fn require_route(registry: &Value, id: &str) -> Result<()> {
    require_registration(registry, "routeReferences", id)
}
fn require_registration(registry: &Value, collection: &str, id: &str) -> Result<()> {
    let count = registry
        .get(collection)
        .and_then(Value::as_array)
        .ok_or(Error("missing UUID handler registry"))?
        .iter()
        .filter(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
        .count();
    if count != 1 {
        return Err(Error("missing or ambiguous UUID handler registration"));
    }
    Ok(())
}

fn encode<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error("structured evidence serialization failed"))
}

fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

fn sha256(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
