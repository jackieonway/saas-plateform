//! Identity and admission rules shared by the platform's internal adapters.
//!
//! This module has no database or HTTP dependency. The database independently
//! enforces these invariants so an import cannot reinterpret an empty set.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum MemberType {
    Employee,
    Customer,
}

impl MemberType {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Employee => "EMPLOYEE",
            Self::Customer => "CUSTOMER",
        }
    }
}

impl FromStr for MemberType {
    type Err = IdentityPolicyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "EMPLOYEE" => Ok(Self::Employee),
            "CUSTOMER" => Ok(Self::Customer),
            _ => Err(IdentityPolicyError::UnknownMemberType),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum MembershipStatus {
    Enabled,
    Disabled,
}

impl MembershipStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "ENABLED",
            Self::Disabled => "DISABLED",
        }
    }
}

impl FromStr for MembershipStatus {
    type Err = IdentityPolicyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ENABLED" => Ok(Self::Enabled),
            "DISABLED" => Ok(Self::Disabled),
            _ => Err(IdentityPolicyError::UnknownMembershipStatus),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum SubjectMode {
    User,
    Machine,
    LegacyMixed,
}

impl SubjectMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::Machine => "MACHINE",
            Self::LegacyMixed => "LEGACY_MIXED",
        }
    }
}

impl FromStr for SubjectMode {
    type Err = IdentityPolicyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "USER" => Ok(Self::User),
            "MACHINE" => Ok(Self::Machine),
            "LEGACY_MIXED" => Ok(Self::LegacyMixed),
            _ => Err(IdentityPolicyError::UnknownSubjectMode),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum IdentityPolicyError {
    #[error("成员类型必须是 EMPLOYEE 或 CUSTOMER")]
    UnknownMemberType,
    #[error("成员状态必须是 ENABLED 或 DISABLED")]
    UnknownMembershipStatus,
    #[error("客户端主体模式必须是 USER、MACHINE 或 LEGACY_MIXED")]
    UnknownSubjectMode,
    #[error("人员身份准入集合不能为空")]
    EmptyMemberTypes,
    #[error("人员客户端不能包含 client_credentials，请使用独立机器客户端")]
    UserMachineGrant,
    #[error("机器客户端必须是 CONFIDENTIAL、仅允许 client_credentials 且成员集合为空")]
    InvalidMachinePolicy,
    #[error("新客户端禁止使用 LEGACY_MIXED 主体模式")]
    NewLegacyMixed,
    #[error("既有混合客户端必须同时保留人员和机器用途并显式配置成员准入")]
    InvalidLegacyMixedPolicy,
}

/// Return the canonical database representation, never interpreting empty as all.
pub(crate) fn normalize_member_types(
    mut values: Vec<MemberType>,
    allow_empty: bool,
) -> Result<Vec<MemberType>, IdentityPolicyError> {
    values.sort_unstable();
    values.dedup();
    if values.is_empty() && !allow_empty {
        return Err(IdentityPolicyError::EmptyMemberTypes);
    }
    Ok(values)
}

/// Existing mixed mode may only be preserved by an adapter that has verified
/// the stored row is already LEGACY_MIXED. A request cannot self-authorize it.
pub(crate) fn validate_client_policy(
    mode: SubjectMode,
    member_types: &[MemberType],
    grants: &[String],
    client_type: &str,
    allow_legacy: bool,
) -> Result<(), IdentityPolicyError> {
    if mode == SubjectMode::LegacyMixed && !allow_legacy {
        return Err(IdentityPolicyError::NewLegacyMixed);
    }
    let member_types = normalize_member_types(member_types.to_vec(), mode == SubjectMode::Machine)?;
    let has_machine = grants
        .iter()
        .any(|grant| grant.trim() == "client_credentials");
    let has_human = grants
        .iter()
        .any(|grant| !grant.trim().is_empty() && grant.trim() != "client_credentials");
    match mode {
        SubjectMode::User => {
            if member_types.is_empty() {
                return Err(IdentityPolicyError::EmptyMemberTypes);
            }
            if has_machine {
                return Err(IdentityPolicyError::UserMachineGrant);
            }
        }
        SubjectMode::Machine => {
            if client_type != "CONFIDENTIAL"
                || !member_types.is_empty()
                || !has_machine
                || has_human
            {
                return Err(IdentityPolicyError::InvalidMachinePolicy);
            }
        }
        SubjectMode::LegacyMixed => {
            if client_type != "CONFIDENTIAL"
                || member_types.is_empty()
                || !has_machine
                || !has_human
            {
                return Err(IdentityPolicyError::InvalidLegacyMixedPolicy);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grants(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn empty_member_set_never_means_unrestricted_user_admission() {
        assert_eq!(
            normalize_member_types(Vec::new(), false),
            Err(IdentityPolicyError::EmptyMemberTypes)
        );
        assert_eq!(
            validate_client_policy(SubjectMode::User, &[], &[], "CONFIDENTIAL", false),
            Err(IdentityPolicyError::EmptyMemberTypes)
        );
    }

    #[test]
    fn admission_set_has_one_canonical_order_without_duplicates() {
        assert_eq!(
            normalize_member_types(
                vec![
                    MemberType::Customer,
                    MemberType::Employee,
                    MemberType::Customer
                ],
                false,
            ),
            Ok(vec![MemberType::Employee, MemberType::Customer])
        );
    }

    #[test]
    fn machine_mode_cannot_carry_user_grants_or_a_member_identity() {
        let machine_grants = grants(&["client_credentials"]);
        assert!(validate_client_policy(
            SubjectMode::Machine,
            &[],
            &machine_grants,
            "CONFIDENTIAL",
            false
        )
        .is_ok());
        for (member_types, grant_list, client_type) in [
            (
                vec![MemberType::Employee],
                machine_grants.clone(),
                "CONFIDENTIAL",
            ),
            (
                vec![],
                grants(&["client_credentials", "password"]),
                "CONFIDENTIAL",
            ),
            (vec![], machine_grants, "PUBLIC"),
            (vec![], vec![], "CONFIDENTIAL"),
        ] {
            assert_eq!(
                validate_client_policy(
                    SubjectMode::Machine,
                    &member_types,
                    &grant_list,
                    client_type,
                    false
                ),
                Err(IdentityPolicyError::InvalidMachinePolicy)
            );
        }
    }

    #[test]
    fn legacy_mixed_requires_proven_existing_registration() {
        let mixed_grants = grants(&["authorization_code", "client_credentials"]);
        assert_eq!(
            validate_client_policy(
                SubjectMode::LegacyMixed,
                &[MemberType::Employee],
                &mixed_grants,
                "CONFIDENTIAL",
                false
            ),
            Err(IdentityPolicyError::NewLegacyMixed)
        );
        assert!(validate_client_policy(
            SubjectMode::LegacyMixed,
            &[MemberType::Employee],
            &mixed_grants,
            "CONFIDENTIAL",
            true
        )
        .is_ok());
        assert_eq!(
            validate_client_policy(
                SubjectMode::User,
                &[MemberType::Employee],
                &mixed_grants,
                "CONFIDENTIAL",
                false
            ),
            Err(IdentityPolicyError::UserMachineGrant)
        );
    }

    #[test]
    fn whitespace_cannot_disguise_a_machine_grant_as_user_only() {
        for grant in [
            " client_credentials ",
            "\tclient_credentials\r\n",
            "\u{85}client_credentials\u{a0}",
            "\u{1680}\u{2000}client_credentials\u{2028}\u{3000}",
        ] {
            let grant_list = grants(&[grant]);
            assert_eq!(
                validate_client_policy(
                    SubjectMode::User,
                    &[MemberType::Employee],
                    &grant_list,
                    "CONFIDENTIAL",
                    false,
                ),
                Err(IdentityPolicyError::UserMachineGrant)
            );
            assert!(validate_client_policy(
                SubjectMode::Machine,
                &[],
                &grant_list,
                "CONFIDENTIAL",
                false,
            )
            .is_ok());
        }
    }

    #[test]
    fn unknown_wire_values_are_rejected() {
        for member in [MemberType::Employee, MemberType::Customer] {
            assert_eq!(member.as_str().parse(), Ok(member));
        }
        for status in [MembershipStatus::Enabled, MembershipStatus::Disabled] {
            assert_eq!(status.as_str().parse(), Ok(status));
        }
        for mode in [
            SubjectMode::User,
            SubjectMode::Machine,
            SubjectMode::LegacyMixed,
        ] {
            assert_eq!(mode.as_str().parse(), Ok(mode));
        }
        assert!("employee".parse::<MemberType>().is_err());
        assert!("ACTIVE".parse::<MembershipStatus>().is_err());
        assert!("PUBLIC".parse::<SubjectMode>().is_err());
    }
}
