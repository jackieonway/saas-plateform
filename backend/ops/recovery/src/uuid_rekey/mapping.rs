//! Private, offline UUID identity mappings. This module never opens a database.
//!
//! A mapping is allocated from an authoritative source row, or from a separately
//! attested historical tombstone. Ordinary references can only resolve source
//! rows; historical resolution must be explicitly requested by the caller.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;
use uuid::{NoContext, Timestamp, Uuid, Variant};

use crate::{Error, Result};

const SCHEMA_VERSION: u32 = 1;
const MAX_UUID_MILLIS: u64 = (1_u64 << 48) - 1;
const COLLISION_ATTEMPTS: usize = 32;

// Deliberately no Debug: a mapping contains private identity relationships.
pub(crate) struct TypedMap {
    frozen_ms: u64,
    allowed_authorities: BTreeSet<String>,
    entries: BTreeMap<(String, Uuid), Entry>,
    targets: BTreeMap<(String, Uuid), Uuid>,
}

struct Entry {
    new_uuid: Uuid,
    source_created_at_ms: Option<i64>,
    timestamp_ms: u64,
    timestamp_reason: TimestampReason,
    provenance: Provenance,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TimestampReason {
    TrustedCreatedAt,
    MissingCreatedAt,
    NegativeCreatedAt,
    CreatedAtAfterFreeze,
    PreservedV7,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TombstoneReason {
    DeletedSourceEntity,
    ExcludedCredential,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Provenance {
    SourceEntity,
    HistoricalTombstone {
        reason: TombstoneReason,
        #[serde(rename = "evidenceSha256")]
        evidence_sha256: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireMap {
    schema_version: u32,
    frozen_ms: u64,
    entries: Vec<WireEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireEntry {
    authority: String,
    old_uuid: String,
    new_uuid: String,
    source_created_at_ms: Option<i64>,
    timestamp_ms: u64,
    timestamp_reason: TimestampReason,
    provenance: Provenance,
}

impl TypedMap {
    pub(crate) fn new(frozen_ms: u64, allowed_authorities: BTreeSet<String>) -> Result<Self> {
        if frozen_ms > MAX_UUID_MILLIS
            || allowed_authorities.is_empty()
            || allowed_authorities
                .iter()
                .any(|authority| !valid_authority(authority))
        {
            return Err(Error("invalid UUID mapping policy"));
        }
        Ok(Self {
            frozen_ms,
            allowed_authorities,
            entries: BTreeMap::new(),
            targets: BTreeMap::new(),
        })
    }

    /// Allocate only after the caller has proved an authoritative source row.
    /// Repeated appearances retain the first allocation and its time evidence.
    pub(crate) fn allocate(
        &mut self,
        authority: &str,
        old: Uuid,
        created_at_ms: Option<i64>,
    ) -> Result<Uuid> {
        self.allocate_with_provenance(authority, old, created_at_ms, Provenance::SourceEntity)
    }

    /// Explicit historical allocation. A 64-digit evidence digest is required;
    /// callers cannot accidentally obtain this behavior by resolving a reference.
    pub(crate) fn attest_tombstone(
        &mut self,
        authority: &str,
        old: Uuid,
        created_at_ms: Option<i64>,
        evidence_sha256: &str,
        reason: &str,
    ) -> Result<Uuid> {
        let reason = match reason {
            "deleted_source_entity" => TombstoneReason::DeletedSourceEntity,
            "excluded_credential" => TombstoneReason::ExcludedCredential,
            _ => return Err(Error("invalid UUID tombstone attestation")),
        };
        if !valid_digest(evidence_sha256) {
            return Err(Error("invalid UUID tombstone attestation"));
        }
        self.allocate_with_provenance(
            authority,
            old,
            created_at_ms,
            Provenance::HistoricalTombstone {
                reason,
                evidence_sha256: evidence_sha256.to_owned(),
            },
        )
    }

    /// Resolve a live/business reference. Tombstones cannot satisfy its closure.
    pub(crate) fn resolve(&self, authority: &str, old: Uuid) -> Result<Uuid> {
        let entry = self.lookup(authority, old)?;
        if entry.provenance != Provenance::SourceEntity {
            return Err(Error("UUID reference requires a source entity"));
        }
        Ok(entry.new_uuid)
    }

    /// Only historical evidence handlers should use this wider resolution mode.
    pub(crate) fn resolve_historical(&self, authority: &str, old: Uuid) -> Result<Uuid> {
        Ok(self.lookup(authority, old)?.new_uuid)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Public evidence may contain authority counts, never identity pairs.
    pub(crate) fn authority_counts(&self) -> BTreeMap<&str, usize> {
        let mut counts = BTreeMap::new();
        for (authority, _) in self.entries.keys() {
            *counts.entry(authority.as_str()).or_insert(0) += 1;
        }
        counts
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Accept a resumed allocation only for the exact independently collected
    /// source closure. Random v7 payloads may differ from a fresh allocation;
    /// source identities, provenance, and timestamp evidence may not.
    pub(crate) fn same_source_coverage(&self, expected: &Self) -> Result<()> {
        if self.frozen_ms != expected.frozen_ms
            || self.allowed_authorities != expected.allowed_authorities
            || self.entries.len() != expected.entries.len()
        {
            return Err(Error("UUID mapping source coverage mismatch"));
        }
        for ((key, entry), (expected_key, expected_entry)) in
            self.entries.iter().zip(&expected.entries)
        {
            if key != expected_key
                || entry.source_created_at_ms != expected_entry.source_created_at_ms
                || entry.timestamp_ms != expected_entry.timestamp_ms
                || entry.timestamp_reason != expected_entry.timestamp_reason
                || entry.provenance != expected_entry.provenance
                // Existing v7 identities have no random reallocation allowance.
                || (is_v7(key.1)
                    && (entry.new_uuid != key.1 || expected_entry.new_uuid != key.1))
            {
                return Err(Error("UUID mapping source coverage mismatch"));
            }
        }
        Ok(())
    }

    /// Serialize only into the caller's encrypted/private migration package.
    #[cfg(test)]
    pub(crate) fn to_value(&self) -> Result<Value> {
        serde_json::to_value(self.wire()).map_err(|_| Error("UUID mapping serialization failed"))
    }

    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&self.wire()).map_err(|_| Error("UUID mapping serialization failed"))
    }

    /// The caller supplies its frozen batch policy; private input cannot widen it.
    #[cfg(test)]
    pub(crate) fn from_value(
        value: Value,
        expected_frozen_ms: u64,
        allowed_authorities: BTreeSet<String>,
    ) -> Result<Self> {
        let wire =
            serde_json::from_value(value).map_err(|_| Error("invalid UUID mapping format"))?;
        Self::from_wire(wire, expected_frozen_ms, allowed_authorities)
    }

    /// Prefer this API for persisted JSON: serde also rejects duplicate fields.
    pub(crate) fn from_bytes(
        bytes: &[u8],
        expected_frozen_ms: u64,
        allowed_authorities: BTreeSet<String>,
    ) -> Result<Self> {
        let wire =
            serde_json::from_slice(bytes).map_err(|_| Error("invalid UUID mapping format"))?;
        Self::from_wire(wire, expected_frozen_ms, allowed_authorities)
    }

    fn allocate_with_provenance(
        &mut self,
        authority: &str,
        old: Uuid,
        created_at_ms: Option<i64>,
        provenance: Provenance,
    ) -> Result<Uuid> {
        self.check_source(authority, old)?;
        let key = (authority.to_owned(), old);
        if let Some(entry) = self.entries.get(&key) {
            if entry.provenance != provenance {
                return Err(Error("UUID mapping provenance conflict"));
            }
            return Ok(entry.new_uuid);
        }
        let (timestamp_ms, timestamp_reason) = if is_v7(old) {
            let ms = timestamp_ms(old)?;
            if ms > self.frozen_ms {
                return Err(Error("UUID source timestamp exceeds frozen batch"));
            }
            (ms, TimestampReason::PreservedV7)
        } else {
            choose_timestamp(created_at_ms, self.frozen_ms)
        };
        let new_uuid = if timestamp_reason == TimestampReason::PreservedV7 {
            if self.targets.contains_key(&(authority.to_owned(), old)) {
                return Err(Error("UUID mapping target collision"));
            }
            old
        } else {
            let mut allocated = None;
            for _ in 0..COLLISION_ATTEMPTS {
                // uuid's standard v7 API obtains its random payload from CSPRNG.
                let candidate = Uuid::new_v7(Timestamp::from_unix(
                    NoContext,
                    timestamp_ms / 1000,
                    ((timestamp_ms % 1000) * 1_000_000) as u32,
                ));
                if !self
                    .targets
                    .contains_key(&(authority.to_owned(), candidate))
                {
                    allocated = Some(candidate);
                    break;
                }
            }
            allocated.ok_or(Error("UUID mapping allocation exhausted"))?
        };
        self.targets.insert((authority.to_owned(), new_uuid), old);
        self.entries.insert(
            key,
            Entry {
                new_uuid,
                source_created_at_ms: created_at_ms,
                timestamp_ms,
                timestamp_reason,
                provenance,
            },
        );
        Ok(new_uuid)
    }

    fn check_source(&self, authority: &str, old: Uuid) -> Result<()> {
        if !self.allowed_authorities.contains(authority) {
            return Err(Error("unknown UUID authority"));
        }
        if old.is_nil()
            || old.get_variant() != Variant::RFC4122
            || !(1..=8).contains(&old.get_version_num())
        {
            return Err(Error("invalid UUID source identifier"));
        }
        Ok(())
    }

    fn lookup(&self, authority: &str, old: Uuid) -> Result<&Entry> {
        self.check_source(authority, old)?;
        self.entries
            .get(&(authority.to_owned(), old))
            .ok_or(Error("unresolved typed UUID reference"))
    }

    fn wire(&self) -> WireMap {
        WireMap {
            schema_version: SCHEMA_VERSION,
            frozen_ms: self.frozen_ms,
            entries: self
                .entries
                .iter()
                .map(|((authority, old), entry)| WireEntry {
                    authority: authority.clone(),
                    old_uuid: old.to_string(),
                    new_uuid: entry.new_uuid.to_string(),
                    source_created_at_ms: entry.source_created_at_ms,
                    timestamp_ms: entry.timestamp_ms,
                    timestamp_reason: entry.timestamp_reason,
                    provenance: entry.provenance.clone(),
                })
                .collect(),
        }
    }

    fn from_wire(
        wire: WireMap,
        expected_frozen_ms: u64,
        allowed_authorities: BTreeSet<String>,
    ) -> Result<Self> {
        if wire.schema_version != SCHEMA_VERSION || wire.frozen_ms != expected_frozen_ms {
            return Err(Error("UUID mapping batch policy mismatch"));
        }
        let mut result = Self::new(expected_frozen_ms, allowed_authorities)?;
        for record in wire.entries {
            let old = canonical_uuid(&record.old_uuid)?;
            let new_uuid = canonical_uuid(&record.new_uuid)?;
            result.check_source(&record.authority, old)?;
            if !is_v7(new_uuid)
                || record.timestamp_ms > result.frozen_ms
                || timestamp_ms(new_uuid)? != record.timestamp_ms
            {
                return Err(Error("invalid UUID mapping target"));
            }
            match &record.provenance {
                Provenance::SourceEntity => {}
                Provenance::HistoricalTombstone {
                    evidence_sha256, ..
                } => {
                    if !valid_digest(evidence_sha256) {
                        return Err(Error("invalid UUID tombstone attestation"));
                    }
                }
            }
            let expected_time = if is_v7(old) {
                if old != new_uuid {
                    return Err(Error("preserved UUID mapping changed"));
                }
                (timestamp_ms(old)?, TimestampReason::PreservedV7)
            } else {
                choose_timestamp(record.source_created_at_ms, result.frozen_ms)
            };
            if (record.timestamp_ms, record.timestamp_reason) != expected_time {
                return Err(Error("UUID mapping timestamp provenance mismatch"));
            }
            let key = (record.authority.clone(), old);
            let target = (record.authority.clone(), new_uuid);
            if result.entries.contains_key(&key) {
                return Err(Error("duplicate typed UUID mapping"));
            }
            if result.targets.contains_key(&target) {
                return Err(Error("UUID mapping target collision"));
            }
            result.targets.insert(target, old);
            result.entries.insert(
                key,
                Entry {
                    new_uuid,
                    source_created_at_ms: record.source_created_at_ms,
                    timestamp_ms: record.timestamp_ms,
                    timestamp_reason: record.timestamp_reason,
                    provenance: record.provenance,
                },
            );
        }
        Ok(result)
    }
}

fn choose_timestamp(created_at_ms: Option<i64>, frozen_ms: u64) -> (u64, TimestampReason) {
    match created_at_ms {
        None => (frozen_ms, TimestampReason::MissingCreatedAt),
        Some(value) if value < 0 => (frozen_ms, TimestampReason::NegativeCreatedAt),
        Some(value) if value as u64 > frozen_ms => {
            (frozen_ms, TimestampReason::CreatedAtAfterFreeze)
        }
        Some(value) => (value as u64, TimestampReason::TrustedCreatedAt),
    }
}

fn valid_authority(authority: &str) -> bool {
    fn segment(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 80
            && value.as_bytes()[0].is_ascii_lowercase()
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    }
    authority
        .split_once('.')
        .is_some_and(|(owner, entity)| segment(owner) && segment(entity))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_v7(id: Uuid) -> bool {
    id.get_version_num() == 7 && id.get_variant() == Variant::RFC4122
}

fn canonical_uuid(value: &str) -> Result<Uuid> {
    let parsed = Uuid::parse_str(value).map_err(|_| Error("invalid UUID mapping identifier"))?;
    if value.len() != 36 || parsed.to_string() != value {
        return Err(Error("invalid UUID mapping identifier"));
    }
    Ok(parsed)
}

fn timestamp_ms(id: Uuid) -> Result<u64> {
    let timestamp = id
        .get_timestamp()
        .ok_or(Error("UUID mapping timestamp unavailable"))?;
    let (seconds, nanos) = timestamp.to_unix();
    seconds
        .checked_mul(1000)
        .and_then(|millis| millis.checked_add(u64::from(nanos / 1_000_000)))
        .ok_or(Error("UUID mapping timestamp out of range"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FROZEN: u64 = 1_792_800_000_123;

    fn authorities() -> BTreeSet<String> {
        ["platform.user", "platform.organization", "stores.store"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn map() -> TypedMap {
        TypedMap::new(FROZEN, authorities()).unwrap()
    }

    // Explicit source v4 fixtures model the offline input identity epoch.
    fn old(payload: u128) -> Uuid {
        Uuid::from_u128(0x00112233_4455_4000_8000_000000000000 | payload)
    }

    fn v7(millis: u64) -> Uuid {
        Uuid::new_v7(Timestamp::from_unix(
            NoContext,
            millis / 1000,
            ((millis % 1000) * 1_000_000) as u32,
        ))
    }

    #[test]
    fn authority_separates_entities_and_reuses_cross_database_references() {
        let mut mapping = map();
        assert!(mapping.is_empty());
        let user = mapping
            .allocate("platform.user", old(1), Some(1001))
            .unwrap();
        let store = mapping
            .allocate("stores.store", old(1), Some(1001))
            .unwrap();
        assert_ne!(user, store);
        assert_eq!(mapping.len(), 2);
        assert!(is_v7(user) && is_v7(store));
        assert_eq!(timestamp_ms(user).unwrap(), 1001);
        assert_eq!(mapping.resolve("platform.user", old(1)).unwrap(), user);
        // A repeated source seen by another database cannot change the first evidence.
        assert_eq!(
            mapping.allocate("platform.user", old(1), None).unwrap(),
            user
        );
        assert_eq!(
            mapping.to_value().unwrap()["entries"][0]["sourceCreatedAtMs"],
            1001
        );
        assert!(mapping.resolve("platform.organization", old(1)).is_err());
        assert!(mapping.resolve("platform.user", old(2)).is_err());
    }

    #[test]
    fn trusted_and_fallback_times_are_exact_and_recorded() {
        let mut mapping = map();
        for (n, supplied, expected_ms, reason) in [
            (1, Some(0), 0, "trusted_created_at"),
            (2, Some(1_234_567), 1_234_567, "trusted_created_at"),
            (3, None, FROZEN, "missing_created_at"),
            (4, Some(-1), FROZEN, "negative_created_at"),
            (
                5,
                Some(FROZEN as i64 + 1),
                FROZEN,
                "created_at_after_freeze",
            ),
            (6, Some(FROZEN as i64), FROZEN, "trusted_created_at"),
            (7, Some(i64::MAX), FROZEN, "created_at_after_freeze"),
            (8, Some(i64::MIN), FROZEN, "negative_created_at"),
        ] {
            let new = mapping.allocate("platform.user", old(n), supplied).unwrap();
            assert_eq!(timestamp_ms(new).unwrap(), expected_ms);
            let value = mapping.to_value().unwrap();
            let entry = value["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["oldUuid"] == old(n).to_string())
                .unwrap();
            assert_eq!(entry["timestampReason"], reason);
            assert_eq!(entry["timestampMs"], expected_ms);
        }
    }

    #[test]
    fn existing_v7_is_stable_but_future_source_ids_are_rejected() {
        let mut mapping = map();
        let existing = v7(FROZEN - 7);
        assert_eq!(
            mapping
                .allocate("platform.user", existing, Some(-1))
                .unwrap(),
            existing
        );
        assert_eq!(
            mapping.to_value().unwrap()["entries"][0]["timestampReason"],
            "preserved_v7"
        );
        assert!(mapping
            .allocate("platform.user", v7(FROZEN + 1), None)
            .is_err());
        assert_eq!(mapping.len(), 1);
    }

    #[test]
    fn persistence_restarts_keep_every_mapping_and_canonical_bytes() {
        let mut mapping = map();
        for n in 1..100 {
            mapping
                .allocate("platform.user", old(n), Some(n as i64))
                .unwrap();
        }
        let bytes = mapping.to_bytes().unwrap();
        let mut loaded = TypedMap::from_bytes(&bytes, FROZEN, authorities()).unwrap();
        for n in 1..100 {
            assert_eq!(
                loaded.allocate("platform.user", old(n), None).unwrap(),
                mapping.resolve("platform.user", old(n)).unwrap()
            );
        }
        assert_eq!(loaded.to_bytes().unwrap(), bytes);
        let loaded =
            TypedMap::from_value(mapping.to_value().unwrap(), FROZEN, authorities()).unwrap();
        assert_eq!(loaded.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn resume_coverage_accepts_independent_allocations_without_changing_targets() {
        let existing = v7(FROZEN - 7);
        let mut expected = map();
        let mut resumed = map();
        for mapping in [&mut expected, &mut resumed] {
            mapping
                .allocate("platform.user", old(1), Some(1001))
                .unwrap();
            mapping
                .allocate("stores.store", existing, Some(-1))
                .unwrap();
            mapping
                .attest_tombstone(
                    "platform.organization",
                    old(2),
                    None,
                    &"a".repeat(64),
                    "deleted_source_entity",
                )
                .unwrap();
        }
        let bytes = resumed.to_bytes().unwrap();
        let resumed = TypedMap::from_bytes(&bytes, FROZEN, authorities()).unwrap();
        assert_ne!(
            resumed.resolve("platform.user", old(1)).unwrap(),
            expected.resolve("platform.user", old(1)).unwrap()
        );
        resumed.same_source_coverage(&expected).unwrap();
        expected.same_source_coverage(&resumed).unwrap();
        assert_eq!(resumed.to_bytes().unwrap(), bytes);
        assert_eq!(resumed.resolve("stores.store", existing).unwrap(), existing);
        map().same_source_coverage(&map()).unwrap();
    }

    #[test]
    fn resume_coverage_rejects_extra_source_entities_and_tombstones() {
        let mut expected = map();
        expected.allocate("platform.user", old(1), None).unwrap();
        for tombstone in [false, true] {
            let mut resumed =
                TypedMap::from_bytes(&expected.to_bytes().unwrap(), FROZEN, authorities()).unwrap();
            if tombstone {
                resumed
                    .attest_tombstone(
                        "platform.user",
                        old(2),
                        None,
                        &"a".repeat(64),
                        "deleted_source_entity",
                    )
                    .unwrap();
            } else {
                resumed.allocate("platform.user", old(2), None).unwrap();
            }
            assert!(resumed.same_source_coverage(&expected).is_err());
            assert!(expected.same_source_coverage(&resumed).is_err());
        }
    }

    #[test]
    fn resume_coverage_rejects_equal_counts_with_changed_keys() {
        let mut expected = map();
        expected.allocate("platform.user", old(1), None).unwrap();
        for (authority, identifier) in
            [("platform.user", old(2)), ("platform.organization", old(1))]
        {
            let mut resumed = map();
            resumed.allocate(authority, identifier, None).unwrap();
            assert_eq!(resumed.len(), expected.len());
            let error = resumed.same_source_coverage(&expected).err().unwrap();
            assert!(!error.to_string().contains(authority));
            assert!(!error.to_string().contains(&identifier.to_string()));
        }
    }

    #[test]
    fn resume_coverage_rejects_changed_time_evidence_and_policy() {
        let mut expected = map();
        expected
            .allocate("platform.user", old(1), Some(1001))
            .unwrap();
        let bytes = expected.to_bytes().unwrap();
        for field in ["source_created_at_ms", "timestamp_ms", "timestamp_reason"] {
            let mut resumed = TypedMap::from_bytes(&bytes, FROZEN, authorities()).unwrap();
            let entry = resumed
                .entries
                .get_mut(&("platform.user".to_owned(), old(1)))
                .unwrap();
            match field {
                "source_created_at_ms" => entry.source_created_at_ms = Some(1002),
                "timestamp_ms" => entry.timestamp_ms = 1002,
                "timestamp_reason" => entry.timestamp_reason = TimestampReason::MissingCreatedAt,
                _ => unreachable!(),
            }
            assert!(resumed.same_source_coverage(&expected).is_err(), "{field}");
        }
        // Distinct but individually valid fallback evidence has the same timestamp.
        let mut missing = map();
        let mut negative = map();
        missing.allocate("platform.user", old(1), None).unwrap();
        negative
            .allocate("platform.user", old(1), Some(-1))
            .unwrap();
        assert!(negative.same_source_coverage(&missing).is_err());

        let mut other_freeze = TypedMap::new(FROZEN + 1, authorities()).unwrap();
        other_freeze
            .allocate("platform.user", old(1), Some(1001))
            .unwrap();
        assert!(other_freeze.same_source_coverage(&expected).is_err());
        let mut widened = authorities();
        widened.insert("platform.role".to_owned());
        let mut other_authorities = TypedMap::new(FROZEN, widened).unwrap();
        other_authorities
            .allocate("platform.user", old(1), Some(1001))
            .unwrap();
        assert!(other_authorities.same_source_coverage(&expected).is_err());
    }

    #[test]
    fn resume_coverage_rejects_changed_tombstone_provenance() {
        let mut expected = map();
        expected
            .attest_tombstone(
                "platform.user",
                old(1),
                None,
                &"a".repeat(64),
                "deleted_source_entity",
            )
            .unwrap();
        for (evidence, reason) in [('b', "deleted_source_entity"), ('a', "excluded_credential")] {
            let mut resumed = map();
            resumed
                .attest_tombstone(
                    "platform.user",
                    old(1),
                    None,
                    &evidence.to_string().repeat(64),
                    reason,
                )
                .unwrap();
            assert!(resumed.same_source_coverage(&expected).is_err());
        }
        let mut source_entity = map();
        source_entity
            .allocate("platform.user", old(1), None)
            .unwrap();
        assert!(source_entity.same_source_coverage(&expected).is_err());
        assert!(expected.same_source_coverage(&source_entity).is_err());
    }

    #[test]
    fn resume_coverage_requires_preserved_v7_targets_to_stay_exact() {
        let existing = v7(FROZEN - 7);
        let mut expected = map();
        expected.allocate("platform.user", existing, None).unwrap();
        let bytes = expected.to_bytes().unwrap();
        let mut resumed = TypedMap::from_bytes(&bytes, FROZEN, authorities()).unwrap();
        resumed
            .entries
            .get_mut(&("platform.user".to_owned(), existing))
            .unwrap()
            .new_uuid = v7(FROZEN - 7);
        assert!(resumed.same_source_coverage(&expected).is_err());
        assert!(expected.same_source_coverage(&resumed).is_err());
        // Persisted inputs are rejected at load time as well.
        assert!(TypedMap::from_bytes(&resumed.to_bytes().unwrap(), FROZEN, authorities()).is_err());
    }

    #[test]
    fn historical_tombstones_require_explicit_attestation_and_resolution() {
        let mut mapping = map();
        assert!(mapping.resolve_historical("platform.user", old(1)).is_err());
        let evidence = "a".repeat(64);
        assert!(mapping
            .attest_tombstone(
                "platform.user",
                old(1),
                None,
                "private evidence",
                "deleted_source_entity"
            )
            .is_err());
        assert!(mapping
            .attest_tombstone("platform.user", old(1), None, &evidence, "guess")
            .is_err());
        let new = mapping
            .attest_tombstone(
                "platform.user",
                old(1),
                None,
                &evidence,
                "deleted_source_entity",
            )
            .unwrap();
        assert!(mapping.resolve("platform.user", old(1)).is_err());
        assert_eq!(
            mapping.resolve_historical("platform.user", old(1)).unwrap(),
            new
        );
        assert!(mapping.allocate("platform.user", old(1), None).is_err());
        assert!(mapping
            .attest_tombstone(
                "platform.user",
                old(1),
                None,
                &"b".repeat(64),
                "deleted_source_entity"
            )
            .is_err());
        let loaded =
            TypedMap::from_bytes(&mapping.to_bytes().unwrap(), FROZEN, authorities()).unwrap();
        assert_eq!(
            loaded.resolve_historical("platform.user", old(1)).unwrap(),
            new
        );
        assert!(loaded.resolve("platform.user", old(1)).is_err());
    }

    #[test]
    fn loader_rejects_duplicate_keys_target_collisions_and_unregistered_authorities() {
        let mut mapping = map();
        mapping.allocate("platform.user", old(1), None).unwrap();
        let original = mapping.to_value().unwrap();
        let mut duplicate = original.clone();
        let row = duplicate["entries"][0].clone();
        duplicate["entries"].as_array_mut().unwrap().push(row);
        assert!(TypedMap::from_value(duplicate, FROZEN, authorities()).is_err());
        let mut collision = original.clone();
        let mut row = collision["entries"][0].clone();
        row["oldUuid"] = json!(old(2).to_string());
        collision["entries"].as_array_mut().unwrap().push(row);
        assert!(TypedMap::from_value(collision, FROZEN, authorities()).is_err());
        let mut unknown = original;
        unknown["entries"][0]["authority"] = json!("other.user");
        assert!(TypedMap::from_value(unknown, FROZEN, authorities()).is_err());
    }

    #[test]
    fn loader_rejects_policy_timestamp_version_and_provenance_tampering() {
        let mut mapping = map();
        mapping
            .allocate("platform.user", old(1), Some(1001))
            .unwrap();
        let original = mapping.to_value().unwrap();
        for (pointer, invalid) in [
            ("/schemaVersion", json!(2)),
            ("/frozenMs", json!(FROZEN + 1)),
            ("/entries/0/newUuid", json!(old(3).to_string())),
            ("/entries/0/timestampMs", json!(1002)),
            ("/entries/0/timestampReason", json!("missing_created_at")),
            ("/entries/0/sourceCreatedAtMs", json!(-1)),
            (
                "/entries/0/oldUuid",
                json!("00000000-0000-0000-0000-000000000000"),
            ),
            (
                "/entries/0/newUuid",
                json!("00112233-4455-7000-c000-000000000003"),
            ),
        ] {
            let mut invalid_map = original.clone();
            *invalid_map.pointer_mut(pointer).unwrap() = invalid;
            assert!(
                TypedMap::from_value(invalid_map, FROZEN, authorities()).is_err(),
                "{pointer}"
            );
        }
        let mut unknown = original;
        unknown["entries"][0]["unexpected"] = json!(true);
        assert!(TypedMap::from_value(unknown, FROZEN, authorities()).is_err());
        let duplicate_field = br#"{"schemaVersion":1,"schemaVersion":1,"frozenMs":0,"entries":[]}"#;
        assert!(TypedMap::from_bytes(duplicate_field, 0, authorities()).is_err());
    }

    #[test]
    fn persisted_time_requires_an_exact_signed_integer_or_null() {
        let mut mapping = map();
        mapping
            .allocate("platform.user", old(1), Some(1001))
            .unwrap();
        let original = String::from_utf8(mapping.to_bytes().unwrap()).unwrap();
        let field = "\"sourceCreatedAtMs\":1001";
        assert!(original.contains(field));
        for invalid in [
            "9223372036854775808",
            "-9223372036854775809",
            "1001.0",
            "1.001e3",
            "\"1001\"",
            "true",
            "{}",
            "[]",
        ] {
            let bytes = original.replace(field, &format!("\"sourceCreatedAtMs\":{invalid}"));
            assert!(TypedMap::from_bytes(bytes.as_bytes(), FROZEN, authorities()).is_err());
        }

        for supplied in [None, Some(i64::MIN), Some(i64::MAX)] {
            let mut mapping = map();
            let expected = mapping.allocate("platform.user", old(1), supplied).unwrap();
            let loaded =
                TypedMap::from_bytes(&mapping.to_bytes().unwrap(), FROZEN, authorities()).unwrap();
            assert_eq!(loaded.resolve("platform.user", old(1)).unwrap(), expected);
        }
    }

    #[test]
    fn invalid_sources_and_policy_are_rejected_without_identifiers_in_errors() {
        assert!(TypedMap::new(MAX_UUID_MILLIS + 1, authorities()).is_err());
        assert!(TypedMap::new(FROZEN, BTreeSet::new()).is_err());
        assert!(TypedMap::new(FROZEN, ["invalid.authority.extra".to_owned()].into()).is_err());
        let mut mapping = map();
        for id in [
            Uuid::nil(),
            Uuid::from_u128(0x00112233_4455_4000_c000_000000000001),
        ] {
            let error = mapping.allocate("platform.user", id, None).err().unwrap();
            assert!(!error.to_string().contains(&id.to_string()));
            assert!(!format!("{error:?}").contains(&id.to_string()));
        }
        assert!(mapping.allocate("unknown.user", old(1), None).is_err());
        assert!(mapping.is_empty());
    }
}
