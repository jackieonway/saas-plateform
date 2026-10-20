//! Read-only static-version retention decisions. A preview never grants deletion.
use chrono::{DateTime, Duration, Utc};
use std::collections::{BTreeSet, HashMap, HashSet};
use uuid::Uuid;

/// Candidate policy for isolated previews; production approval is separate.
#[derive(Clone, Copy)]
pub struct Policy {
    minimum_age_days: u32,
    keep_latest: usize,
}
impl Policy {
    /// Policies may be stricter than 30 days / 5 versions, never shorter.
    pub fn new(minimum_age_days: u32, keep_latest: usize) -> Result<Self, InvalidPolicy> {
        if !(30..=3650).contains(&minimum_age_days) || !(5..=1000).contains(&keep_latest) {
            return Err(InvalidPolicy);
        }
        Ok(Self {
            minimum_age_days,
            keep_latest,
        })
    }
}
/// The requested preview policy is outside its bounded candidate contract.
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidPolicy;

/// Trusted snapshot input, independent of SQL row types and HTTP frameworks.
pub struct Version {
    /// Immutable release identity.
    pub id: Uuid,
    /// Owning application identity.
    pub application: Uuid,
    /// Registered environment.
    pub environment: String,
    /// Version name; identical versions may be referenced by multiple environments.
    pub version: String,
    /// Registration time, not an operator-supplied "last accessed" time.
    pub created_at: DateTime<Utc>,
    /// Canonical digest and URL identities shared with other records.
    pub artifact_keys: Vec<String>,
    /// An active pointer in any environment refers to this row.
    pub active: bool,
    /// A previous pointer in any environment refers to this row.
    pub previous: bool,
    /// A durable application operation has not completed/reconciled.
    pub in_flight: bool,
    /// The platform owns a ZIP manifest; external URL ownership is not assumed.
    pub platform_owned: bool,
}
/// One read-only impact decision. Empty reasons means manual review, never apply.
pub struct Decision {
    /// Release identity; no file path, credentials or arbitrary URL is returned.
    pub id: Uuid,
    /// Stable protection reason codes.
    pub reasons: BTreeSet<&'static str>,
}

/// Protect age OR recent-version count, plus every reference and pending job.
/// `now` and the inputs must come from a trusted consistent snapshot. Callers
/// must reread before any later operation; this result is not a deletion token.
pub fn preview(versions: &[Version], policy: Policy, now: DateTime<Utc>) -> Vec<Decision> {
    let mut order: Vec<_> = versions.iter().collect();
    order.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    let mut recent = HashSet::new();
    let mut seen: HashMap<(Uuid, &str), HashSet<&str>> = HashMap::new();
    for row in order {
        let names = seen.entry((row.application, &row.environment)).or_default();
        if names.contains(row.version.as_str()) || names.len() < policy.keep_latest {
            names.insert(&row.version);
            recent.insert((row.application, row.version.as_str()));
        }
    }
    let mut keys: HashMap<&str, Vec<&Version>> = HashMap::new();
    for row in versions {
        for key in &row.artifact_keys {
            keys.entry(key).or_default().push(row);
        }
    }
    versions
        .iter()
        .map(|row| {
            let mut reasons = BTreeSet::new();
            if row.created_at > now {
                reasons.insert("FUTURE_TIMESTAMP");
            }
            if now.signed_duration_since(row.created_at)
                < Duration::days(i64::from(policy.minimum_age_days))
            {
                reasons.insert("MINIMUM_AGE");
            }
            if recent.contains(&(row.application, row.version.as_str())) {
                reasons.insert("LATEST_VERSIONS");
            }
            if row.active {
                reasons.insert("ACTIVE");
            }
            if row.previous {
                reasons.insert("PREVIOUS");
            }
            if row.in_flight {
                reasons.insert("IN_FLIGHT_OPERATION");
            }
            if !row.platform_owned {
                reasons.insert("EXTERNAL_OWNERSHIP_REQUIRES_REVIEW");
            }
            if row.artifact_keys.is_empty() {
                reasons.insert("ARTIFACT_IDENTITY_UNKNOWN");
            }
            for other in versions.iter().filter(|other| {
                other.application == row.application
                    && other.version == row.version
                    && other.id != row.id
            }) {
                if other.environment != row.environment {
                    reasons.insert("CROSS_ENVIRONMENT_REFERENCE");
                }
                if other.active {
                    reasons.insert("ACTIVE_REFERENCE");
                }
                if other.previous {
                    reasons.insert("PREVIOUS_REFERENCE");
                }
                if other.in_flight {
                    reasons.insert("IN_FLIGHT_OPERATION");
                }
                if recent.contains(&(other.application, other.version.as_str()))
                    || now.signed_duration_since(other.created_at)
                        < Duration::days(i64::from(policy.minimum_age_days))
                {
                    reasons.insert("RETAINED_VERSION_REFERENCE");
                }
            }
            for other in row
                .artifact_keys
                .iter()
                .filter_map(|key| keys.get(key.as_str()))
                .flatten()
                .filter(|other| other.id != row.id)
            {
                reasons.insert("SHARED_ARTIFACT_REFERENCE");
                if other.active {
                    reasons.insert("ACTIVE_REFERENCE");
                }
                if other.previous {
                    reasons.insert("PREVIOUS_REFERENCE");
                }
                if other.in_flight {
                    reasons.insert("IN_FLIGHT_OPERATION");
                }
                if other.environment != row.environment {
                    reasons.insert("CROSS_ENVIRONMENT_REFERENCE");
                }
            }
            Decision {
                id: row.id,
                reasons,
            }
        })
        .collect()
}
