//! Exact bindings for the centralized member-selection protocol.
//!
//! These decisions do not authenticate a caller or query authorization. Adapters
//! must obtain registration and source proofs from their respective authorities,
//! lock the original session, and commit credential consumption with the context.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use uuid::{Uuid, Variant};

/// Version accepted by the member-selection contract.
pub const CONTRACT_VERSION: &str = "context-selection/1";

/// Maximum time between confirmation and credential consumption.
pub const CREDENTIAL_TTL_SECONDS: i64 = 120;

/// Explicit wire version; unrecognized versions fail deserialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum ContractVersion {
    /// First centralized context-selection protocol.
    #[serde(rename = "context-selection/1")]
    V1,
}

/// A backend-only opaque credential, deliberately without Debug or Display.
/// Adapters generate 32 cryptographically random bytes and encode lowercase hex.
#[derive(Serialize)]
#[serde(transparent)]
pub struct Credential(String);

impl Credential {
    /// Validate the exact wire encoding; this does not authenticate the value.
    pub fn parse(raw: String) -> Result<Self, SelectionError> {
        if raw.len() == 64
            && raw
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(raw))
        } else {
            Err(SelectionError::Invalid)
        }
    }

    /// Compute the only credential representation that may be stored in the DB.
    pub fn digest(&self) -> String {
        hex::encode(Sha256::digest(self.0.as_bytes()))
    }

    /// Expose the credential only to an authenticated private service adapter
    /// that must serialize it or seal it before persistence. It has no `Debug`
    /// or `Display` implementation and must never be logged.
    pub fn as_secret(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Credential {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Extension delivered only in the authenticated authorization-code response.
/// Refresh responses must never carry or mint a new selection credential.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeExtension {
    /// Explicit independently versioned protocol.
    pub contract_version: ContractVersion,
    /// The BFF's server-side transaction identity.
    pub transaction_id: Id,
    /// Single-use platform credential; do not log this response.
    pub credential: Credential,
}

/// Immutable source and selection failures; none permits a fallback identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SelectionError {
    /// Wire identity, version or registration is malformed.
    #[error("invalid context-selection binding")]
    Invalid,
    /// A source, registration, session or selected member has changed.
    #[error("context-selection binding changed")]
    Changed,
    /// The original credential/source deadline has passed.
    #[error("context-selection deadline expired")]
    Expired,
    /// Credential consumption already committed.
    #[error("context-selection credential already consumed")]
    Consumed,
}

/// Canonical RFC UUIDv7, rejecting alternate textual forms at the wire boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Id(Uuid);

impl Id {
    /// Validate an internally obtained identity before binding it to a flow.
    pub fn new(value: Uuid) -> Result<Self, SelectionError> {
        if value.get_version_num() == 7 && value.get_variant() == Variant::RFC4122 {
            Ok(Self(value))
        } else {
            Err(SelectionError::Invalid)
        }
    }

    /// Access the validated identity for a typed database parameter.
    pub fn uuid(self) -> Uuid {
        self.0
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let uuid = Uuid::parse_str(&raw).map_err(serde::de::Error::custom)?;
        if uuid.to_string() != raw {
            return Err(serde::de::Error::custom("canonical UUIDv7 required"));
        }
        Self::new(uuid).map_err(serde::de::Error::custom)
    }
}

/// Database-compatible nonnegative version, serialized as a decimal string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version(i64);

impl Version {
    /// Validate a generation obtained from authoritative state.
    pub fn new(value: i64) -> Result<Self, SelectionError> {
        if value >= 0 {
            Ok(Self(value))
        } else {
            Err(SelectionError::Invalid)
        }
    }

    /// Access the generation for comparison or a database parameter.
    pub fn value(self) -> i64 {
        self.0
    }
}

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let value = raw.parse::<i64>().map_err(serde::de::Error::custom)?;
        if value.to_string() != raw {
            return Err(serde::de::Error::custom("canonical version required"));
        }
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Member purpose is chosen by the registered RP, never the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MemberType {
    /// Employee console and host micro-app context.
    Employee,
    /// Independent customer portal context.
    Customer,
}

/// A switch is bound to the exact session and version observed at initiation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Intent {
    /// No original business context is being replaced.
    Login {},
    /// Replace only this BFF session's business context.
    Switch {
        /// Opaque server-side identity of the original session.
        #[serde(rename = "sessionId")]
        session_id: Id,
        /// Must still match at commit and during result recovery.
        #[serde(rename = "expectedVersion")]
        expected_version: Version,
    },
}

/// Platform registration snapshot, obtained without browser-provided overrides.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Registration {
    /// Registered OIDC relying party.
    pub client_id: Id,
    /// Immutable OIDC client policy generation.
    pub client_policy_version: Version,
    /// Exactly one purpose for each managed employee/customer RP.
    pub member_type: MemberType,
    /// Exact callback, already matched against the RP registration.
    pub redirect_uri: String,
    /// Console has no business application; other managed RPs bind one.
    pub application: Option<ApplicationBinding>,
}

/// Application identity and immutable policy generation are inseparable.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationBinding {
    /// Registered target application.
    pub id: Id,
    /// Original policy version, not a newly repaired generation.
    pub policy_version: Version,
}

/// Identity-owned account authentication proof tied to one OIDC interaction.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    /// Exact configured public issuer.
    pub issuer: String,
    /// Stable account identity; changing organization never changes this value.
    pub user_id: Id,
    /// Original account generation.
    pub account_version: Version,
    /// Original global SSO session.
    pub global_session_id: Id,
    /// Authorization interaction from which the exchanged code must originate.
    pub interaction_id: Id,
    /// Original authentication instant.
    pub auth_time: DateTime<Utc>,
}

/// Exact flow identity saved server-side and repeated by authenticated adapters.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    /// BFF-created correlation identity, unrelated to OAuth state secrets.
    pub transaction_id: Id,
    /// Identity authority proof; adapters must verify it online.
    pub source: Source,
    /// Current registered RP and application policy snapshot.
    pub registration: Registration,
    /// Original session ownership when switching.
    pub intent: Intent,
}

impl Binding {
    /// Reject malformed or incomplete trusted-adapter input before persistence.
    /// `issuer` is the configured authority, never a caller-supplied issuer.
    pub fn validate(&self, issuer: &str, now: DateTime<Utc>) -> Result<(), SelectionError> {
        let callback = reqwest::Url::parse(&self.registration.redirect_uri)
            .map_err(|_| SelectionError::Invalid)?;
        let authority = reqwest::Url::parse(issuer).map_err(|_| SelectionError::Invalid)?;
        if self.source.issuer != issuer
            || authority.scheme() != "https"
            || authority.host_str().is_none()
            || authority.origin().ascii_serialization() != issuer
            || callback.scheme() != "https"
            || callback.host_str().is_none()
            || !callback.username().is_empty()
            || callback.password().is_some()
            || callback.fragment().is_some()
            || self.registration.redirect_uri.len() > 2048
            || self.registration.client_policy_version.value() < 1
            || self
                .registration
                .application
                .as_ref()
                .is_some_and(|app| app.policy_version.value() < 1)
            || self.source.auth_time > now
            || matches!(self.intent, Intent::Switch { expected_version, .. } if expected_version.value() < 1)
        {
            return Err(SelectionError::Invalid);
        }
        Ok(())
    }
}

/// Selected authoritative member; organizations never imply membership.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Member {
    /// Account owning the member.
    pub user_id: Id,
    /// Tenant inferred by platform from the member's organization.
    pub tenant_id: Id,
    /// Exact member organization.
    pub organization_id: Id,
    /// Independent member identity.
    pub membership_id: Id,
    /// Employee/customer use, checked against the registered RP.
    pub member_type: MemberType,
    /// Original member generation.
    pub identity_version: Version,
}

/// Persisted immutable selection. Construct only from freshly verified authority.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    /// Exact authenticated flow binding.
    pub binding: Binding,
    /// Confirmed member and original generation.
    pub member: Member,
    /// Confirmation time, taken from the database clock by the adapter.
    pub created_at: DateTime<Utc>,
    /// Earliest original source/interaction deadline, capped at 120 seconds.
    pub expires_at: DateTime<Utc>,
}

impl Selection {
    /// Bind one member and cap lifetime without extending upstream deadlines.
    pub fn confirm(
        binding: Binding,
        member: Member,
        issuer: &str,
        now: DateTime<Utc>,
        source_expires_at: DateTime<Utc>,
        interaction_expires_at: DateTime<Utc>,
    ) -> Result<Self, SelectionError> {
        binding.validate(issuer, now)?;
        if member.user_id != binding.source.user_id
            || member.member_type != binding.registration.member_type
        {
            return Err(SelectionError::Changed);
        }
        let expires_at = now
            .checked_add_signed(Duration::seconds(CREDENTIAL_TTL_SECONDS))
            .ok_or(SelectionError::Invalid)?
            .min(source_expires_at)
            .min(interaction_expires_at);
        if expires_at <= now {
            return Err(SelectionError::Expired);
        }
        Ok(Self {
            binding,
            member,
            created_at: now,
            expires_at,
        })
    }

    /// Validate against fresh proofs before atomic consumption. Caller must lock
    /// the original session and mark consumption in the same transaction.
    pub fn check_consumption(
        &self,
        binding: &Binding,
        member: &Member,
        now: DateTime<Utc>,
        consumed: bool,
    ) -> Result<(), SelectionError> {
        if &self.binding != binding || &self.member != member {
            return Err(SelectionError::Changed);
        }
        if consumed {
            return Err(SelectionError::Consumed);
        }
        if now < self.created_at || now >= self.expires_at {
            return Err(SelectionError::Expired);
        }
        Ok(())
    }

    /// Recover only a result of the exact already committed operation while its
    /// original deadline remains valid. Adapters additionally revalidate the
    /// stored resulting context and authenticate the original BFF principal.
    /// This decision never authorizes creating another context or token pair.
    pub fn check_recovery(
        &self,
        binding: &Binding,
        member: &Member,
        now: DateTime<Utc>,
        consumed: bool,
    ) -> Result<(), SelectionError> {
        self.check_consumption(binding, member, now, false)?;
        if !consumed {
            return Err(SelectionError::Changed);
        }
        Ok(())
    }
}
