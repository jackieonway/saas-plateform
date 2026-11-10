#[cfg(test)]
use crate::{error::AppError, util::uuid7};
#[cfg(test)]
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::collections::BTreeSet;
#[cfg(test)]
use uuid::Uuid;

pub(super) const VIEW: &str = "system:oauth:view";
pub(super) const REGISTER: &str = "system:oauth:register";
pub(super) const REVIEW: &str = "system:oauth:review";
pub(super) const PUBLISH: &str = "system:oauth:publish";
pub(super) const ROTATE: &str = "system:oauth:rotate";
pub(super) const DISABLE: &str = "system:oauth:disable";
pub(super) const SCOPES: &str = "system:oauth:scopes";
pub(super) const INVITE: &str = "system:oauth:invite";

#[cfg(test)]
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Manifest {
    pub display_name: String,
    pub purpose: String,
    pub privacy_url: String,
    pub domains: Vec<String>,
    pub redirect_uris: Vec<String>,
    pub post_logout_redirect_uris: Vec<String>,
    pub scopes: Vec<String>,
    pub contact_reference: String,
    pub domain_evidence_reference: String,
    pub logout_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backchannel_logout_uri: Option<String>,
}

#[cfg(test)]
pub(crate) fn id(raw: &str) -> Result<Uuid, AppError> {
    let parsed = uuid7::parse_uuid(raw)?;
    if parsed.to_string() != raw {
        return Err(AppError::bad_request("标识必须是规范 UUIDv7"));
    }
    Ok(parsed)
}

#[cfg(test)]
pub(crate) fn environment(raw: &str) -> Result<&str, AppError> {
    if matches!(raw, "test" | "production") {
        Ok(raw)
    } else {
        Err(AppError::bad_request(
            "接入环境必须明确为 test 或 production",
        ))
    }
}

#[cfg(test)]
pub(super) fn evidence(raw: &str) -> Result<Uuid, AppError> {
    id(raw)
}

#[cfg(test)]
pub(super) fn bounded_text(raw: &str, max: usize) -> Result<String, AppError> {
    let value = raw.trim();
    if value.is_empty() || value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(AppError::bad_request("必填资料为空、过长或包含控制字符"));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
fn exact_https(raw: &str) -> Result<reqwest::Url, AppError> {
    let parsed = reqwest::Url::parse(raw)
        .map_err(|_| AppError::bad_request("接入地址必须为精确 HTTPS URL"))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.as_str() != raw
        || raw.contains('*')
        || ["code", "state", "error", "iss"]
            .iter()
            .any(|key| parsed.query_pairs().any(|(name, _)| name == *key))
    {
        return Err(AppError::bad_request(
            "接入地址必须为无通配符、用户信息或协议响应参数的精确 HTTPS URL",
        ));
    }
    Ok(parsed)
}

#[cfg(test)]
pub(crate) fn validate_manifest(mut value: Manifest) -> Result<Manifest, AppError> {
    value.display_name = bounded_text(&value.display_name, 120)?;
    value.purpose = bounded_text(&value.purpose, 1000)?;
    if matches!(
        value.purpose.as_str(),
        "PLATFORM_CONSOLE" | "CUSTOMER_PORTAL"
    ) {
        return Err(AppError::bad_request(
            "第三方不能使用平台或客户门户的内部用途",
        ));
    }
    evidence(&value.contact_reference)?;
    evidence(&value.domain_evidence_reference)?;
    if !matches!(
        value.logout_mode.as_str(),
        "BACK_CHANNEL" | "SHORT_SESSION_ONLINE_CHECK"
    ) {
        return Err(AppError::bad_request(
            "必须声明受控登出接入或短会话在线检查合同",
        ));
    }
    value.domains.sort();
    if value.domains.is_empty()
        || value.domains.len() > 16
        || value.domains.windows(2).any(|pair| pair[0] == pair[1])
        || value.domains.iter().any(|domain| {
            domain.len() > 253
                || !domain.contains('.')
                || domain.bytes().any(|byte| {
                    !(byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'-'))
                })
                || domain.starts_with('.')
                || domain.ends_with('.')
                || domain
                    .split('.')
                    .any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-'))
        })
    {
        return Err(AppError::bad_request(
            "域名清单必须为明确且不重复的规范 DNS 主机名",
        ));
    }
    let hosts: BTreeSet<_> = value.domains.iter().map(String::as_str).collect();
    match value.logout_mode.as_str() {
        "SHORT_SESSION_ONLINE_CHECK" if value.backchannel_logout_uri.is_none() => {}
        "BACK_CHANNEL" => {
            let raw = value
                .backchannel_logout_uri
                .as_deref()
                .ok_or_else(|| AppError::bad_request("back-channel审核必须填写精确HTTPS地址"))?;
            let url = exact_https(raw)?;
            if url.query().is_some() || !hosts.contains(url.host_str().unwrap_or_default()) {
                return Err(AppError::bad_request(
                    "back-channel地址须无查询参数且属于当前审核域名",
                ));
            }
        }
        _ => {
            return Err(AppError::bad_request(
                "登出方式与back-channel审核地址不匹配",
            ))
        }
    }
    for (uris, required) in [
        (&mut value.redirect_uris, true),
        (&mut value.post_logout_redirect_uris, false),
    ] {
        uris.sort();
        if (required && uris.is_empty())
            || uris.len() > 16
            || uris.windows(2).any(|pair| pair[0] == pair[1])
        {
            return Err(AppError::bad_request("回调地址为空、重复或超过登记上限"));
        }
        for uri in uris {
            let parsed = exact_https(uri)?;
            if !hosts.contains(parsed.host_str().unwrap_or_default()) {
                return Err(AppError::bad_request(
                    "回调地址必须属于本次待核验的明确域名",
                ));
            }
        }
    }
    let privacy = exact_https(&value.privacy_url)?;
    if !hosts.contains(privacy.host_str().unwrap_or_default()) {
        return Err(AppError::bad_request("隐私说明必须属于本次域名清单"));
    }
    value.scopes.sort();
    if value.scopes.is_empty()
        || !value.scopes.iter().any(|scope| scope == "openid")
        || value
            .scopes
            .iter()
            .any(|scope| !matches!(scope.as_str(), "openid" | "profile"))
        || value.scopes.windows(2).any(|pair| pair[0] == pair[1])
    {
        return Err(AppError::bad_request(
            "第三方首版仅批准 openid/profile，不开放个人联系方式、离线或业务资源",
        ));
    }
    Ok(value)
}

#[cfg(test)]
pub(super) fn transition(from: &str, to: &str, env: &str) -> Result<&'static str, AppError> {
    let permission = match (from, to, env) {
        ("SUBMITTED", "RETURNED", _) => REVIEW,
        ("SUBMITTED", "TEST_APPROVED", "test") => REVIEW,
        ("TEST_APPROVED", "TEST_ACCEPTED", "test") => REVIEW,
        ("SUBMITTED", "PRODUCTION_APPROVED", "production") => REVIEW,
        ("PRODUCTION_APPROVED", "ENABLED", "production") => PUBLISH,
        ("TEST_APPROVED" | "TEST_ACCEPTED" | "PRODUCTION_APPROVED" | "ENABLED", "SUSPENDED", _) => {
            DISABLE
        }
        _ => {
            return Err(AppError::conflict(
                "接入申请状态不允许此操作；资料变化须重新提交审核",
            ))
        }
    };
    Ok(permission)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> Manifest {
        Manifest {
            display_name: "Owned partner login".into(),
            purpose: "Controlled account login".into(),
            privacy_url: "https://partner.example.invalid/privacy".into(),
            domains: vec!["partner.example.invalid".into()],
            redirect_uris: vec!["https://partner.example.invalid/callback".into()],
            post_logout_redirect_uris: vec![],
            scopes: vec!["openid".into(), "profile".into()],
            contact_reference: uuid7::new_uuid().to_string(),
            domain_evidence_reference: uuid7::new_uuid().to_string(),
            logout_mode: "SHORT_SESSION_ONLINE_CHECK".into(),
            backchannel_logout_uri: None,
        }
    }
    #[test]
    fn manifests_reject_unreviewed_scopes_and_callback_ownership_changes() {
        assert!(validate_manifest(manifest()).is_ok());
        for scope in ["email", "phone", "offline_access", "stores.read"] {
            let mut value = manifest();
            value.scopes.push(scope.into());
            assert!(validate_manifest(value).is_err());
        }
        for uri in [
            "http://partner.example.invalid/callback",
            "https://foreign.example.invalid/callback",
            "https://partner.example.invalid/callback?code=old",
            "https://*.example.invalid/callback",
        ] {
            let mut value = manifest();
            value.redirect_uris = vec![uri.into()];
            assert!(validate_manifest(value).is_err());
        }
        assert!(transition("SUBMITTED", "ENABLED", "production").is_err());
        assert!(transition("SUBMITTED", "TEST_APPROVED", "production").is_err());
        assert_eq!(
            transition("PRODUCTION_APPROVED", "ENABLED", "production").unwrap(),
            PUBLISH
        );
    }
}
