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

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum IdentityPolicyError {
    #[error("成员类型必须是 EMPLOYEE 或 CUSTOMER")]
    UnknownMemberType,
    #[error("成员状态必须是 ENABLED 或 DISABLED")]
    UnknownMembershipStatus,
    #[error("人员身份准入集合不能为空")]
    EmptyMemberTypes,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_member_set_never_means_unrestricted_user_admission() {
        assert_eq!(
            normalize_member_types(Vec::new(), false),
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
                false
            ),
            Ok(vec![MemberType::Employee, MemberType::Customer])
        );
    }
    #[test]
    fn unknown_wire_values_are_rejected() {
        for member in [MemberType::Employee, MemberType::Customer] {
            assert_eq!(member.as_str().parse(), Ok(member));
        }
        for status in [MembershipStatus::Enabled, MembershipStatus::Disabled] {
            assert_eq!(status.as_str().parse(), Ok(status));
        }
        assert!("employee".parse::<MemberType>().is_err());
        assert!("ACTIVE".parse::<MembershipStatus>().is_err());
    }
}
