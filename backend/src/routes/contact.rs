use serde::Deserialize;

use crate::error::AppError;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Contact {
    #[serde(default)]
    pub contact_name: String,
    #[serde(default)]
    pub contact_phone: String,
    #[serde(default)]
    pub contact_email: String,
}

pub(super) fn normalize_phone(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && !['-', '(', ')'].contains(c))
        .collect()
}

pub(super) fn phone_identifier(value: &str) -> String {
    let phone = normalize_phone(value);
    if phone.len() >= 6
        && phone.len() <= 30
        && phone
            .chars()
            .enumerate()
            .all(|(i, c)| c.is_ascii_digit() || (i == 0 && c == '+'))
    {
        phone
    } else {
        String::new()
    }
}

impl Contact {
    pub fn normalized(mut self) -> Self {
        self.contact_name = self.contact_name.trim().to_string();
        self.contact_phone = self.contact_phone.trim().to_string();
        self.contact_email = self.contact_email.trim().to_lowercase();
        self
    }

    pub fn is_empty(&self) -> bool {
        self.contact_name.is_empty()
            && self.contact_phone.is_empty()
            && self.contact_email.is_empty()
    }

    pub fn validate(&self, provisioning: bool) -> Result<(), AppError> {
        if self.contact_name.chars().count() > 100
            || self.contact_name.chars().any(char::is_control)
        {
            return Err(AppError::bad_request(
                "联系人姓名最多 100 个字符，不能包含控制字符",
            ));
        }
        if !self.contact_phone.is_empty()
            && (self.contact_phone.chars().count() > 30
                || phone_identifier(&self.contact_phone).is_empty())
        {
            return Err(AppError::bad_request("联系人电话格式不正确"));
        }
        if !self.contact_email.is_empty() {
            let parts: Vec<&str> = self.contact_email.split('@').collect();
            if self.contact_email.len() > 100
                || parts.len() != 2
                || parts[0].is_empty()
                || parts[1].is_empty()
                || !parts[1].contains('.')
                || parts[1].starts_with('.')
                || parts[1].ends_with('.')
                || self
                    .contact_email
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control())
            {
                return Err(AppError::bad_request("联系人 Email 格式不正确"));
            }
        }
        // Legacy clients may omit the entire contact. A supplied contact must
        // identify a person and provide at least one usable login identifier.
        if provisioning
            && !self.is_empty()
            && (self.contact_name.is_empty()
                || (self.contact_phone.is_empty() && self.contact_email.is_empty()))
        {
            return Err(AppError::bad_request(
                "请填写联系人姓名，并至少填写电话或 Email",
            ));
        }
        Ok(())
    }

    pub fn username(&self) -> String {
        if self.contact_phone.is_empty() {
            self.contact_email.clone()
        } else {
            normalize_phone(&self.contact_phone)
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContactUpdate {
    pub contact_name: Option<String>,
    pub contact_phone: Option<String>,
    pub contact_email: Option<String>,
}

impl ContactUpdate {
    pub fn normalized(self) -> Result<Self, AppError> {
        let updated = Self {
            contact_name: self.contact_name.map(|value| value.trim().to_string()),
            contact_phone: self.contact_phone.map(|value| value.trim().to_string()),
            contact_email: self.contact_email.map(|value| value.trim().to_lowercase()),
        };
        Contact {
            contact_name: updated.contact_name.clone().unwrap_or_default(),
            contact_phone: updated.contact_phone.clone().unwrap_or_default(),
            contact_email: updated.contact_email.clone().unwrap_or_default(),
        }
        .validate(false)?;
        Ok(updated)
    }
}
