//! Persisted public branding and platform administration settings.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};

use crate::error::AppError;

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlatformBranding {
    pub(crate) platform_name: String,
    pub(crate) platform_description: String,
    pub(crate) logo_url: String,
    pub(crate) login_title: String,
    pub(crate) login_description: String,
    pub(crate) footer_text: String,
    pub(crate) primary_color: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlatformSettings {
    #[serde(flatten)]
    pub(crate) branding: PlatformBranding,
    pub(crate) platform_tenant_name: String,
    pub(crate) platform_tenant_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SettingsUpdate {
    platform_name: String,
    platform_description: String,
    logo_url: String,
    login_title: String,
    login_description: String,
    footer_text: String,
    platform_tenant_name: String,
    #[serde(default)]
    primary_color: Option<String>,
}

#[derive(sqlx::FromRow)]
struct SettingsRow {
    platform_name: String,
    platform_description: String,
    logo_url: String,
    login_title: String,
    login_description: String,
    footer_text: String,
    primary_color: String,
    platform_tenant_name: String,
    platform_tenant_code: String,
}

impl From<SettingsRow> for PlatformSettings {
    fn from(row: SettingsRow) -> Self {
        Self {
            branding: PlatformBranding {
                platform_name: row.platform_name,
                platform_description: row.platform_description,
                logo_url: row.logo_url,
                login_title: row.login_title,
                login_description: row.login_description,
                footer_text: row.footer_text,
                primary_color: row.primary_color,
            },
            platform_tenant_name: row.platform_tenant_name,
            platform_tenant_code: row.platform_tenant_code,
        }
    }
}

/// Read only the branding fields safe to display before authentication.
pub(crate) async fn load_branding(pool: &PgPool) -> Result<PlatformBranding, AppError> {
    Ok(sqlx::query_as(
        "SELECT platform_name, platform_description, logo_url, login_title, login_description, \
         footer_text, primary_color FROM platform_settings WHERE singleton = true",
    )
    .fetch_one(pool)
    .await?)
}

pub(crate) async fn load_settings(pool: &PgPool) -> Result<PlatformSettings, AppError> {
    let row: SettingsRow = sqlx::query_as(
        "SELECT s.platform_name, s.platform_description, s.logo_url, s.login_title, \
         s.login_description, s.footer_text, s.primary_color, t.name AS platform_tenant_name, \
         t.code AS platform_tenant_code FROM platform_settings s \
         JOIN tenants t ON t.is_platform WHERE s.singleton = true",
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::not_found("平台设置或平台租户不存在"))?;
    Ok(row.into())
}

fn normalize_text(
    value: &str,
    label: &str,
    maximum: usize,
    required: bool,
) -> Result<String, AppError> {
    // Reject controls before trimming, so surrounding newlines cannot bypass validation.
    if value.chars().any(char::is_control) {
        return Err(AppError::bad_request(format!("{label}不能包含控制字符")));
    }
    let value = value.trim();
    if required && value.is_empty() {
        return Err(AppError::bad_request(format!("{label}不能为空")));
    }
    if value.chars().count() > maximum {
        return Err(AppError::bad_request(format!(
            "{label}最多 {maximum} 个字符"
        )));
    }
    Ok(value.to_owned())
}

fn bad_logo() -> AppError {
    AppError::bad_request("平台标识地址必须为安全的站内绝对路径或 HTTP/HTTPS 地址")
}

fn check_logo_url(value: &str) -> Result<(), AppError> {
    if value.contains('\\') || value.chars().any(char::is_control) {
        return Err(bad_logo());
    }
    if value.starts_with('/') {
        if value.starts_with("//") {
            return Err(bad_logo());
        }
        let path = value.split(['?', '#']).next().unwrap_or("");
        if path
            .split('/')
            .any(|segment| segment == "." || segment == "..")
        {
            return Err(bad_logo());
        }
        let url = reqwest::Url::parse(&format!("https://platform.invalid{value}"))
            .map_err(|_| bad_logo())?;
        if url.host_str() != Some("platform.invalid") {
            return Err(bad_logo());
        }
    } else {
        let lower = value.to_ascii_lowercase();
        if !lower.starts_with("http://") && !lower.starts_with("https://") {
            return Err(bad_logo());
        }
        let url = reqwest::Url::parse(value).map_err(|_| bad_logo())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(bad_logo());
        }
    }
    Ok(())
}

fn normalize_logo(value: &str) -> Result<String, AppError> {
    let value = normalize_text(value, "平台标识地址", 2048, false)?;
    if value.is_empty() {
        return Ok(value);
    }
    check_logo_url(&value)?;
    // Check nested encodings as well as raw input; encoded separators, backslashes
    // and controls must not become a different URL when a proxy decodes a path.
    let mut decoded = value.clone();
    for _ in 0..8 {
        let next = percent_encoding::percent_decode_str(&decoded)
            .decode_utf8()
            .map_err(|_| bad_logo())?
            .into_owned();
        if next == decoded {
            return Ok(value);
        }
        check_logo_url(&next)?;
        decoded = next;
    }
    Err(bad_logo())
}

fn normalize_theme_color(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.len() != 7
        || !value.starts_with('#')
        || !value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    {
        return Err(AppError::bad_request("主题主色必须为 #RRGGBB 格式的颜色"));
    }
    Ok(value.to_ascii_uppercase())
}

impl SettingsUpdate {
    fn normalize(self) -> Result<Self, AppError> {
        Ok(Self {
            platform_name: normalize_text(&self.platform_name, "平台名称", 100, true)?,
            platform_description: normalize_text(
                &self.platform_description,
                "平台描述",
                300,
                false,
            )?,
            logo_url: normalize_logo(&self.logo_url)?,
            login_title: normalize_text(&self.login_title, "登录页标题", 100, true)?,
            login_description: normalize_text(&self.login_description, "登录页说明", 300, false)?,
            footer_text: normalize_text(&self.footer_text, "页脚文案", 500, false)?,
            platform_tenant_name: normalize_text(
                &self.platform_tenant_name,
                "平台租户名称",
                100,
                true,
            )?,
            primary_color: self
                .primary_color
                .as_deref()
                .map(normalize_theme_color)
                .transpose()?,
        })
    }
}

pub(crate) async fn update_settings(
    tx: &mut Transaction<'_, Postgres>,
    body: SettingsUpdate,
) -> Result<(PlatformSettings, PlatformSettings), AppError> {
    let body = body.normalize()?;
    // Keep the real tenant name and branding in one transaction. Lock the tenant
    // first to serialize settings saves with ordinary tenant edits.
    sqlx::query("SELECT id FROM tenants WHERE is_platform FOR UPDATE")
        .execute(&mut **tx)
        .await?;
    let before: SettingsRow = sqlx::query_as(
        "SELECT s.platform_name, s.platform_description, s.logo_url, s.login_title, \
         s.login_description, s.footer_text, s.primary_color, t.name AS platform_tenant_name, \
         t.code AS platform_tenant_code FROM platform_settings s \
         JOIN tenants t ON t.is_platform WHERE s.singleton = true FOR UPDATE OF s",
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::not_found("平台设置或平台租户不存在"))?;
    let (platform_tenant_name, platform_tenant_code): (String, String) = sqlx::query_as(
        "UPDATE tenants SET name = $1, updated_at = now() WHERE is_platform RETURNING name, code",
    )
    .bind(body.platform_tenant_name)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::not_found("平台租户不存在"))?;
    let branding = sqlx::query_as(
        "UPDATE platform_settings SET platform_name = $1, platform_description = $2, \
         logo_url = $3, login_title = $4, login_description = $5, footer_text = $6, \
         primary_color = COALESCE($7, primary_color), updated_at = now() WHERE singleton = true \
         RETURNING platform_name, platform_description, logo_url, login_title, login_description, footer_text, primary_color",
    )
    .bind(body.platform_name)
    .bind(body.platform_description)
    .bind(body.logo_url)
    .bind(body.login_title)
    .bind(body.login_description)
    .bind(body.footer_text)
    .bind(body.primary_color)
    .fetch_one(&mut **tx)
    .await?;
    Ok((
        before.into(),
        PlatformSettings {
            branding,
            platform_tenant_name,
            platform_tenant_code,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::{normalize_logo, normalize_text, normalize_theme_color, SettingsUpdate};
    use serde_json::json;

    fn update() -> serde_json::Value {
        json!({
            "platformName": "测试平台", "platformDescription": "", "logoUrl": "",
            "loginTitle": "欢迎回来", "loginDescription": "", "footerText": "",
            "platformTenantName": "管理租户"
        })
    }

    #[test]
    fn text_limits_count_characters_and_reject_blank_required_or_controls() {
        assert_eq!(normalize_text("  中文  ", "名称", 2, true).unwrap(), "中文");
        assert!(normalize_text("中".repeat(100).as_str(), "名称", 100, true).is_ok());
        assert!(normalize_text("中".repeat(101).as_str(), "名称", 100, true).is_err());
        assert!(normalize_text("  ", "名称", 100, true).is_err());
        assert!(normalize_text("", "说明", 300, false).is_ok());
        assert!(normalize_text("\n名称", "名称", 100, true).is_err());
        assert!(normalize_text("名\0称", "名称", 100, true).is_err());
    }

    #[test]
    fn theme_color_normalizes_safe_hex_and_rejects_css_input() {
        assert_eq!(normalize_theme_color("  #abCdEf  ").unwrap(), "#ABCDEF");
        for value in ["#008577", "#FFFFFF", "#000000"] {
            assert!(normalize_theme_color(value).is_ok());
        }
        for value in [
            "",
            "red",
            "#fff",
            "#GGGGGG",
            "#1234567",
            "rgb(1,2,3)",
            "#123456;--primary:red",
            "#中0000",
        ] {
            assert!(normalize_theme_color(value).is_err(), "accepted {value}");
        }
        let update: SettingsUpdate = serde_json::from_value(update()).unwrap();
        assert!(
            update.normalize().unwrap().primary_color.is_none(),
            "legacy updates must preserve the saved theme"
        );
    }

    #[test]
    fn logo_accepts_safe_paths_and_http_urls() {
        for value in [
            "",
            "/assets/logo.svg",
            "/标识/图标.png?v=2",
            "https://example.com/logo.png",
            "http://127.0.0.1/logo.png",
        ] {
            assert!(normalize_logo(value).is_ok(), "rejected {value}");
        }
    }

    #[test]
    fn logo_rejects_active_schemes_authorities_and_encoded_path_escapes() {
        for value in [
            "javascript:alert(1)",
            "https:example.com/logo.png",
            "data:image/svg+xml,x",
            "ftp://example.com/logo.png",
            "//example.com/logo.png",
            "assets/logo.svg",
            "/\\example.com/logo.png",
            "https://user:secret@example.com/logo.png",
            "/assets/../logo.png",
            "/%2fexample.com/logo.png",
            "/%252fexample.com/logo.png",
            "/assets/%2e%2e/logo.png",
            "/logo%0a.png",
            "/logo%5c.png",
            "https://example.com/lo%250ago.png",
        ] {
            assert!(normalize_logo(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn update_rejects_tenant_code_override_and_incomplete_payloads() {
        let mut value = update();
        value["platformTenantCode"] = json!("other-tenant");
        assert!(serde_json::from_value::<SettingsUpdate>(value).is_err());
        let mut value = update();
        value.as_object_mut().unwrap().remove("footerText");
        assert!(serde_json::from_value::<SettingsUpdate>(value).is_err());
    }

    #[test]
    fn update_applies_all_field_bounds_before_persistence() {
        for (field, maximum) in [
            ("platformName", 100),
            ("platformDescription", 300),
            ("loginTitle", 100),
            ("loginDescription", 300),
            ("footerText", 500),
            ("platformTenantName", 100),
        ] {
            let mut value = update();
            value[field] = json!("中".repeat(maximum));
            assert!(
                serde_json::from_value::<SettingsUpdate>(value.clone())
                    .unwrap()
                    .normalize()
                    .is_ok(),
                "rejected boundary {field}"
            );
            value[field] = json!("中".repeat(maximum + 1));
            assert!(
                serde_json::from_value::<SettingsUpdate>(value)
                    .unwrap()
                    .normalize()
                    .is_err(),
                "accepted oversized {field}"
            );
        }
    }
}
