use serde::Serialize;
use uuid::Uuid;

use super::proof::Authority;
use super::{CONTRACT_VERSION, ENTRY_PATH, PURPOSE};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NamedEntity {
    id: Uuid,
    name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Application {
    id: Uuid,
    code: String,
    name: String,
    entry_path: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Context {
    contract_version: &'static str,
    purpose: &'static str,
    user_id: Uuid,
    membership_id: Uuid,
    member_type: &'static str,
    identity_version: String,
    account_version: String,
    tenant: NamedEntity,
    organization: NamedEntity,
    application: Application,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Account {
    id: Uuid,
    username: String,
    real_name: String,
    avatar: String,
}

#[derive(Serialize)]
pub(super) struct Profile {
    context: Context,
    account: Account,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Applications {
    contract_version: &'static str,
    context: Context,
    applications: Vec<Application>,
}

#[derive(sqlx::FromRow)]
pub(super) struct Presentation {
    tenant_name: String,
    organization_name: String,
    application_code: String,
    application_name: String,
    username: String,
    real_name: String,
    avatar: String,
}

impl Context {
    pub(super) fn new(authority: &Authority, values: &Presentation) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            purpose: PURPOSE,
            user_id: authority.user_id,
            membership_id: authority.membership_id,
            member_type: "CUSTOMER",
            identity_version: authority.identity_version.to_string(),
            account_version: authority.account_version.to_string(),
            tenant: NamedEntity {
                id: authority.tenant_id,
                name: values.tenant_name.clone(),
            },
            organization: NamedEntity {
                id: authority.organization_id,
                name: values.organization_name.clone(),
            },
            application: Application {
                id: authority.application_id,
                code: values.application_code.clone(),
                name: values.application_name.clone(),
                entry_path: ENTRY_PATH,
            },
        }
    }
}

impl Profile {
    pub(super) fn new(authority: &Authority, values: &Presentation) -> Self {
        Self {
            context: Context::new(authority, values),
            account: Account {
                id: authority.user_id,
                username: values.username.clone(),
                real_name: values.real_name.clone(),
                avatar: values.avatar.clone(),
            },
        }
    }
}

impl Applications {
    pub(super) fn new(authority: &Authority, values: &Presentation) -> Self {
        let context = Context::new(authority, values);
        Self {
            contract_version: CONTRACT_VERSION,
            applications: vec![context.application.clone()],
            context,
        }
    }
}
