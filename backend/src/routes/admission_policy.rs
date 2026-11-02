//! Shared validation for application admission management.

use crate::error::AppError;
use crate::identity_policy::{normalize_member_types, MemberType};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PolicyUpdate {
    pub allowed_member_types: Vec<MemberType>,
    pub access_policy_version: i64,
    pub reason: String,
}

/// Omission keeps legacy defaults; an explicitly provided null is malformed.
pub(super) fn present_value<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

pub(super) fn member_strings(
    values: Vec<MemberType>,
    allow_empty: bool,
) -> Result<Vec<String>, AppError> {
    normalize_member_types(values, allow_empty)
        .map(|members| {
            members
                .into_iter()
                .map(|member| member.as_str().to_owned())
                .collect()
        })
        .map_err(|error| AppError::bad_request(error.to_string()))
}

pub(super) fn reason(value: Option<&str>) -> Result<&str, AppError> {
    let value = value.unwrap_or_default().trim();
    if value.is_empty() || value.chars().count() > 500 || value.chars().any(char::is_control) {
        return Err(AppError::bad_request(
            "策略变更原因须为 1 至 500 个字符且不能包含控制字符",
        ));
    }
    Ok(value)
}

/// An explicit policy edit always carries the version the administrator read.
/// Locking and checking the stored version happen together in the owner transaction.
pub(super) fn check_version(
    requested: Option<i64>,
    current: i64,
    required: bool,
) -> Result<(), AppError> {
    match requested {
        None if required => Err(AppError::bad_request(
            "策略变更必须提供 accessPolicyVersion",
        )),
        Some(version) if version < 1 => {
            Err(AppError::bad_request("accessPolicyVersion 必须为正整数"))
        }
        Some(version) if version != current => {
            Err(AppError::conflict("准入策略已变化，请重新读取后保存"))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_edits_require_a_current_positive_version() {
        assert!(matches!(
            check_version(None, 1, true),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            check_version(Some(0), 1, true),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            check_version(Some(1), 2, true),
            Err(AppError::Conflict(_))
        ));
        assert!(check_version(Some(2), 2, true).is_ok());
        assert!(check_version(None, 2, false).is_ok());
    }

    #[test]
    fn policies_canonicalize_identity_sets_without_empty_wildcards() {
        assert_eq!(
            member_strings(
                vec![
                    MemberType::Customer,
                    MemberType::Employee,
                    MemberType::Customer
                ],
                false
            )
            .unwrap(),
            ["EMPLOYEE", "CUSTOMER"]
        );
        assert!(member_strings(vec![], false).is_err());
        assert!(member_strings(vec![], true).unwrap().is_empty());
    }

    #[test]
    fn reasons_and_policy_wire_values_are_validated() {
        for invalid in [None, Some(" "), Some("changed\npolicy")] {
            assert!(reason(invalid).is_err());
        }
        assert_eq!(
            reason(Some("  关闭不再使用的客户入口  ")).unwrap(),
            "关闭不再使用的客户入口"
        );
        assert!(serde_json::from_value::<PolicyUpdate>(serde_json::json!({"allowedMemberTypes":["ALIEN"],"accessPolicyVersion":1,"reason":"test"})).is_err());
        assert!(serde_json::from_value::<PolicyUpdate>(serde_json::json!({"allowedMemberTypes":["EMPLOYEE"],"accessPolicyVersion":1,"reason":"test","unknown":true})).is_err());
    }
}
