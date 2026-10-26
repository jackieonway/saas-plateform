//! Account-only HTTPS adapter. No PHC, database transaction or prepared password
//! crosses this boundary. Transport uncertainty never authorizes a local fallback.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::Config;
use crate::error::AppError;
use crate::platform_accounts::{
    AccountAdministrationPort, AccountCredential, AccountFuture, AccountMutationTx,
    AuthenticationPort, CreatedAccount, CredentialHash, LoginIdentifier, NewAccount,
    PasswordChange, PreparedPassword,
};

pub(crate) struct RemoteAccounts {
    config: Arc<Config>,
    slots: tokio::sync::Semaphore,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccountDto {
    id: Uuid,
    username: String,
    real_name: String,
    status: String,
    account_version: String,
}

impl AccountDto {
    fn credential(self) -> Result<AccountCredential, AppError> {
        let version = canonical_version(&self.account_version)?;
        if !crate::util::uuid7::is_uuid_v7(self.id)
            || !matches!(self.status.as_str(), "ENABLED" | "DISABLED")
            || self.username.len() > 1024
            || self.real_name.len() > 4096
        {
            return Err(AppError::Unavailable);
        }
        // This value is a local, non-serializable capability slot. Remote code
        // never reads it: the only proof sent is the explicit account/version.
        Ok(AccountCredential::new(
            self.id,
            self.username,
            self.real_name,
            self.status,
            version,
            CredentialHash::new(String::new()),
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommandContext {
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub tenant_id: Uuid,
    pub organization_id: Uuid,
    pub account_version: String,
}

/// Password-containing payloads are never Debug, logged, or persisted. The
/// corresponding intent contains only its keyed digest and safe follow-up data.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Command {
    pub operation_id: Uuid,
    pub target_user_id: Uuid,
    pub expected_account_version: String,
    pub expires_at: DateTime<Utc>,
    pub canonical_digest: String,
    pub kind: String,
    pub payload: Value,
    pub context: Option<CommandContext>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Receipt {
    pub operation_id: Uuid,
    pub target_user_id: Uuid,
    pub canonical_digest: String,
    pub state: String,
    pub before_version: String,
    pub after_version: String,
    pub revoked_sessions: u64,
    pub revoked_tokens: u64,
}

impl RemoteAccounts {
    pub(crate) fn configured(config: Arc<Config>) -> Option<Arc<Self>> {
        config.identity_remote_enabled.then(|| {
            Arc::new(Self {
                config,
                slots: tokio::sync::Semaphore::new(16),
            })
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, AppError> {
        let _permit = self
            .slots
            .try_acquire()
            .map_err(|_| AppError::Unavailable)?;
        let client = self
            .config
            .identity_http_client
            .as_ref()
            .ok_or(AppError::Unavailable)?;
        let origin = self
            .config
            .identity_service_url
            .as_deref()
            .ok_or(AppError::Unavailable)?;
        let id = self
            .config
            .identity_service_client_id
            .as_deref()
            .ok_or(AppError::Unavailable)?;
        let secret = self
            .config
            .identity_service_client_secret
            .as_deref()
            .ok_or(AppError::Unavailable)?;
        let mut request = client
            .request(method, format!("{origin}{path}"))
            .basic_auth(id, Some(secret))
            .header("accept", "application/json")
            .header("cache-control", "no-store");
        if let Some(body) = body {
            let bytes = serde_json::to_vec(body).map_err(|_| AppError::Unavailable)?;
            if bytes.len() > 32 * 1024 {
                return Err(AppError::bad_request("账号命令超出允许大小"));
            }
            request = request
                .header("content-type", "application/json")
                .body(bytes);
        }
        let mut response = request.send().await.map_err(|_| AppError::Unavailable)?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > 64 * 1024)
        {
            return Err(AppError::Unavailable);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| AppError::Unavailable)? {
            if bytes.len() + chunk.len() > 64 * 1024 {
                return Err(AppError::Unavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            // Never forward provider bodies (or errors containing submitted secrets).
            return Err(match status {
                StatusCode::UNAUTHORIZED => AppError::Unavailable,
                StatusCode::NOT_FOUND => AppError::not_found("账号操作结果尚不可确认"),
                StatusCode::CONFLICT => AppError::conflict("账号代际或操作摘要已变更"),
                StatusCode::BAD_REQUEST => AppError::bad_request("账号命令不符合要求"),
                _ => AppError::Unavailable,
            });
        }
        serde_json::from_slice(&bytes).map_err(|_| AppError::Unavailable)
    }

    pub(crate) fn command(
        &self,
        target: Uuid,
        expected: i64,
        kind: &str,
        payload: Value,
        context: Option<CommandContext>,
    ) -> Result<Command, AppError> {
        if !crate::util::uuid7::is_uuid_v7(target)
            || expected < 0
            || !payload.is_object()
            || !matches!(
                kind,
                "CREATE"
                    | "UPDATE_PROFILE"
                    | "SET_STATUS"
                    | "RESET_PASSWORD"
                    | "CHANGE_PASSWORD"
                    | "REVOKE_ALL"
                    | "RETIRE"
            )
        {
            return Err(AppError::bad_request("账号命令前提无效"));
        }
        let mut command = Command {
            operation_id: crate::util::uuid7::new_uuid(),
            target_user_id: target,
            expected_account_version: expected.to_string(),
            expires_at: Utc::now() + chrono::Duration::seconds(50),
            canonical_digest: String::new(),
            kind: kind.to_owned(),
            payload,
            context,
        };
        let mut value = serde_json::to_value(&command).map_err(|_| AppError::Unavailable)?;
        value
            .as_object_mut()
            .ok_or(AppError::Unavailable)?
            .remove("canonicalDigest");
        let secret = self
            .config
            .identity_service_client_secret
            .as_deref()
            .ok_or(AppError::Unavailable)?;
        command.canonical_digest = command_digest(secret.as_bytes(), &value)?;
        Ok(command)
    }

    pub(crate) async fn execute(&self, command: &Command) -> Result<Receipt, AppError> {
        let value = serde_json::to_value(command).map_err(|_| AppError::Unavailable)?;
        let receipt = self
            .request(
                Method::POST,
                "/internal/identity/accounts/commands",
                Some(&value),
            )
            .await?;
        let receipt = validate_receipt(
            receipt,
            command.operation_id,
            command.target_user_id,
            &command.canonical_digest,
        )?;
        if receipt.before_version != command.expected_account_version {
            return Err(AppError::Unavailable);
        }
        Ok(receipt)
    }

    /// Receipt queries never dispatch a command, even for a provider 404. A
    /// timeout/404 is unknown, not permission to replay an old password request.
    pub(crate) async fn receipt(
        &self,
        operation: Uuid,
        target: Uuid,
        digest: &str,
    ) -> Result<Receipt, AppError> {
        if !valid_digest(digest) {
            return Err(AppError::Unavailable);
        }
        let value = self
            .request(
                Method::GET,
                &format!("/internal/identity/accounts/operations/{operation}?digest={digest}"),
                None,
            )
            .await?;
        validate_receipt(value, operation, target, digest)
    }

    pub(crate) async fn snapshot(&self, id: Uuid) -> Result<AccountCredential, AppError> {
        let value = self
            .request(
                Method::GET,
                &format!("/internal/identity/accounts/{id}"),
                None,
            )
            .await?;
        let account: AccountDto =
            serde_json::from_value(value).map_err(|_| AppError::Unavailable)?;
        if account.id != id {
            return Err(AppError::Unavailable);
        }
        account.credential()
    }
}

fn canonical_version(value: &str) -> Result<i64, AppError> {
    let parsed: i64 = value.parse().map_err(|_| AppError::Unavailable)?;
    if parsed < 0 || parsed.to_string() != value {
        return Err(AppError::Unavailable);
    }
    Ok(parsed)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validate_receipt(
    value: Value,
    operation: Uuid,
    target: Uuid,
    digest: &str,
) -> Result<Receipt, AppError> {
    let receipt: Receipt = serde_json::from_value(value).map_err(|_| AppError::Unavailable)?;
    if receipt.operation_id != operation
        || receipt.target_user_id != target
        || !crate::util::uuid7::is_uuid_v7(operation)
        || !crate::util::uuid7::is_uuid_v7(target)
        || receipt.canonical_digest != digest
        || receipt.state != "COMPLETED"
        || !valid_digest(digest)
        || canonical_version(&receipt.after_version)? < canonical_version(&receipt.before_version)?
    {
        return Err(AppError::Unavailable);
    }
    Ok(receipt)
}

/// HMAC-SHA256, not a public hash of a low-entropy password. Canonical JSON is
/// recursively sorted and compact, with the digest field omitted by the caller.
fn command_digest(secret: &[u8], value: &Value) -> Result<String, AppError> {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let ordered: std::collections::BTreeMap<_, _> =
                    object.iter().map(|(k, v)| (k.clone(), sorted(v))).collect();
                Value::Object(ordered.into_iter().collect())
            }
            Value::Array(values) => Value::Array(values.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    let bytes = serde_json::to_vec(&sorted(value)).map_err(|_| AppError::Unavailable)?;
    Ok(hex::encode(hmac_sha256(secret, &bytes)))
}

fn hmac_sha256(secret: &[u8], message: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 64];
    if secret.len() > key.len() {
        key[..32].copy_from_slice(&Sha256::digest(secret));
    } else {
        key[..secret.len()].copy_from_slice(secret);
    }
    let mut inner_key = key;
    let mut outer_key = key;
    for byte in &mut inner_key {
        *byte ^= 0x36;
    }
    for byte in &mut outer_key {
        *byte ^= 0x5c;
    }
    let mut inner = Sha256::new();
    inner.update(inner_key);
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(outer_key);
    outer.update(inner.finalize());
    outer.finalize().into()
}

impl AuthenticationPort for RemoteAccounts {
    fn candidates<'a>(
        &'a self,
        identifier: LoginIdentifier<'a>,
    ) -> AccountFuture<'a, Vec<AccountCredential>> {
        Box::pin(async move {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Candidates {
                accounts: Vec<AccountDto>,
            }
            let value = self.request(Method::POST, "/internal/identity/accounts/candidates", Some(&json!({"identifier":identifier.identifier,"phone":identifier.phone,"email":identifier.email}))).await?;
            let result: Candidates =
                serde_json::from_value(value).map_err(|_| AppError::Unavailable)?;
            if result.accounts.len() > 65 {
                return Err(AppError::Unavailable);
            }
            result
                .accounts
                .into_iter()
                .map(AccountDto::credential)
                .collect()
        })
    }

    fn by_username<'a>(
        &'a self,
        username: &'a str,
    ) -> AccountFuture<'a, Option<AccountCredential>> {
        Box::pin(async move {
            let rows = self
                .candidates(LoginIdentifier {
                    identifier: username,
                    phone: "",
                    email: "",
                })
                .await?;
            let mut exact = rows.into_iter().filter(|row| row.username == username);
            let account = exact.next();
            if exact.next().is_some() {
                return Err(AppError::Unavailable);
            }
            Ok(account)
        })
    }

    fn by_user_id(&self, id: Uuid) -> AccountFuture<'_, AccountCredential> {
        Box::pin(self.snapshot(id))
    }

    fn verify<'a>(
        &'a self,
        credential: &'a AccountCredential,
        password: &'a str,
    ) -> AccountFuture<'a, bool> {
        Box::pin(async move {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Verification {
                verified: bool,
            }
            let value = self.request(Method::POST, "/internal/identity/accounts/verify", Some(&json!({"id":credential.id,"expectedAccountVersion":credential.account_version.to_string(),"password":password}))).await?;
            Ok(serde_json::from_value::<Verification>(value)
                .map_err(|_| AppError::Unavailable)?
                .verified)
        })
    }

    fn credential_matches<'a>(
        &'a self,
        _tx: &'a mut dyn AccountMutationTx,
        _credential: &'a AccountCredential,
    ) -> AccountFuture<'a, bool> {
        // A caller-owned database transaction must never turn into a network
        // transaction. Remote issuance uses a safe version projection in SQL.
        Box::pin(async { Err(AppError::Unavailable) })
    }
}

impl AccountAdministrationPort for RemoteAccounts {
    fn validate_password(&self, password: &str) -> Result<(), AppError> {
        crate::util::password::validate_for_config(password, &self.config)
    }
    fn prepare_password<'a>(&'a self, _password: &'a str) -> AccountFuture<'a, PreparedPassword> {
        Box::pin(async { Err(AppError::Unavailable) })
    }
    fn create<'a>(
        &'a self,
        _tx: &'a mut dyn AccountMutationTx,
        _account: NewAccount<'a>,
        _password: &'a PreparedPassword,
    ) -> AccountFuture<'a, CreatedAccount> {
        Box::pin(async { Err(AppError::Unavailable) })
    }
    fn replace_password<'a>(
        &'a self,
        _tx: &'a mut dyn AccountMutationTx,
        _id: Uuid,
        _password: &'a PreparedPassword,
        _expected: Option<&'a AccountCredential>,
    ) -> AccountFuture<'a, PasswordChange> {
        Box::pin(async { Err(AppError::Unavailable) })
    }
    fn set_status<'a>(
        &'a self,
        _tx: &'a mut dyn AccountMutationTx,
        _id: Uuid,
        _status: &'a str,
    ) -> AccountFuture<'a, bool> {
        Box::pin(async { Err(AppError::Unavailable) })
    }
    fn revoke_all<'a>(
        &'a self,
        _tx: &'a mut dyn AccountMutationTx,
        _id: Uuid,
    ) -> AccountFuture<'a, u64> {
        Box::pin(async { Err(AppError::Unavailable) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_keyed_canonical_and_covers_nested_password_and_operation() {
        assert_eq!(
            hex::encode(hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        let first = json!({"z":1,"payload":{"password":"secret","name":"abc"}});
        let reordered = json!({"payload":{"name":"abc","password":"secret"},"z":1});
        assert_eq!(
            command_digest(b"key", &first).unwrap(),
            command_digest(b"key", &reordered).unwrap()
        );
        assert_ne!(
            command_digest(b"key", &first).unwrap(),
            command_digest(b"other", &first).unwrap()
        );
        assert_ne!(
            command_digest(b"key", &first).unwrap(),
            command_digest(
                b"key",
                &json!({"z":2,"payload":{"password":"secret","name":"abc"}})
            )
            .unwrap()
        );
    }

    #[test]
    fn receipt_requires_exact_operation_target_digest_and_canonical_versions() {
        let operation = crate::util::uuid7::new_uuid();
        let target = crate::util::uuid7::new_uuid();
        let digest = "a".repeat(64);
        let value = json!({"operationId":operation,"targetUserId":target,"canonicalDigest":digest,"state":"COMPLETED","beforeVersion":"1","afterVersion":"2","revokedSessions":0,"revokedTokens":0});
        assert!(validate_receipt(value.clone(), operation, target, &digest).is_ok());
        assert!(validate_receipt(
            value.clone(),
            crate::util::uuid7::new_uuid(),
            target,
            &digest
        )
        .is_err());
        let mut reversed = value.clone();
        reversed["afterVersion"] = json!("0");
        assert!(validate_receipt(reversed, operation, target, &digest).is_err());
        let mut extra = value;
        extra["password"] = json!("leak");
        assert!(validate_receipt(extra, operation, target, &digest).is_err());
        for version in ["01", "-1", "1e2", "+1", " 1"] {
            assert!(canonical_version(version).is_err());
        }
    }
}
