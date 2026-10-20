use super::{
    config::{self, ConfiguredGroup},
    worker::Job,
};
use crate::{error::AppError, state::AppState};
use base64::Engine;
use deployment_contract::{
    sha256,
    zip::{self, Manifest, Payload, PayloadFile, Receipt, Work},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{io::Read, sync::Arc, time::Duration};

/// Read and hash the platform's canonical files; remote nodes never fetch a supplied URL.
pub(super) async fn payload(
    state: &AppState,
    j: &Job,
    m: &Manifest,
) -> Result<Arc<Payload>, AppError> {
    let root = state.config.micro_apps_dir.clone();
    let code = j.application_code.clone();
    let version = j.version.clone();
    let digest = j.artifact_digest.clone();
    let expected = m.clone();
    tokio::task::spawn_blocking(move || {
        let installed = crate::release_archives::verify_installed(&root, &code, &version, &digest)?;
        if serde_json::to_value(&installed).ok() != serde_json::to_value(&expected).ok() {
            return Err(AppError::conflict("源产物清单变化"));
        }
        let directory = std::path::Path::new(&root)
            .join(&code)
            .join("versions")
            .join(&version);
        let mut files = Vec::new();
        for f in &expected.files {
            let path = directory.join(&f.path);
            for p in path.ancestors() {
                if std::fs::symlink_metadata(p)
                    .map_err(|_| AppError::Unavailable)?
                    .is_symlink()
                {
                    return Err(AppError::Unavailable);
                }
            }
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|_| AppError::Unavailable)?
                .take(f.size + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| AppError::Unavailable)?;
            if bytes.len() as u64 != f.size || sha256(&bytes) != f.sha256 {
                return Err(AppError::conflict("源产物内容变化"));
            }
            files.push(PayloadFile {
                path: f.path.clone(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            });
        }
        Ok(Arc::new(Payload {
            manifest: expected,
            files,
        }))
    })
    .await
    .map_err(|_| AppError::Unavailable)?
}
struct Peer {
    client: reqwest::Client,
    origin: String,
    token: String,
}
impl Peer {
    fn new(g: &ConfiguredGroup, node: &str) -> Result<Self, &'static str> {
        let endpoint = g
            .policy
            .nodes
            .iter()
            .find(|n| n.id == node)
            .ok_or("POLICY_REJECTED")?;
        let credential = g
            .nodes
            .iter()
            .find(|n| n.node_id == node)
            .ok_or("POLICY_REJECTED")?;
        let ca = config::private_file(&credential.ca_ref, 65536)
            .map_err(|_| "NODE_CREDENTIAL_UNAVAILABLE")?;
        let ca = reqwest::Certificate::from_pem(&ca).map_err(|_| "NODE_CREDENTIAL_UNAVAILABLE")?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .add_root_certificate(ca)
            .build()
            .map_err(|_| "NODE_CREDENTIAL_UNAVAILABLE")?;
        Ok(Self {
            client,
            origin: endpoint.origin.clone(),
            token: config::token(credential).map_err(|_| "NODE_CREDENTIAL_UNAVAILABLE")?,
        })
    }
    async fn post(&self, path: &str, body: Vec<u8>) -> Result<Option<Receipt>, &'static str> {
        if body.len() > zip::MAX_WIRE_BYTES {
            return Err("PAYLOAD_TOO_LARGE");
        }
        let mut response = self
            .client
            .post(format!("{}{path}", self.origin))
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| "NODE_TLS_OR_UNAVAILABLE")?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "NODE_RESPONSE_FAILED")? {
            if bytes.len() + chunk.len() > 65536 {
                return Err("NODE_RESPONSE_TOO_LARGE");
            }
            bytes.extend_from_slice(&chunk)
        }
        if status == reqwest::StatusCode::NOT_FOUND
            && serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .is_some_and(|v| v["code"] == "ARTIFACT_MISSING")
        {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(match status.as_u16() {
                401 | 403 => "NODE_AUTH_FAILED",
                409 => "NODE_CONTENT_CONFLICT",
                423 => "NODE_BUSY",
                _ => "NODE_REQUEST_REJECTED",
            });
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "UNTRUSTED_RECEIPT")
    }
    async fn public_files(&self, w: &Work, m: &Manifest) -> Result<(), &'static str> {
        for f in &m.files {
            let path = crate::release_archives::encode_path(&f.path);
            let url = format!(
                "{}/sub-apps/{}/versions/{}/{path}",
                self.origin, w.application_code, w.version
            );
            let mut response = self
                .client
                .get(url)
                .send()
                .await
                .map_err(|_| "NODE_PUBLIC_UNAVAILABLE")?;
            if response.status() != reqwest::StatusCode::OK {
                return Err("NODE_PUBLIC_UNAVAILABLE");
            }
            let mut length = 0u64;
            let mut hash = Sha256::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "NODE_PUBLIC_UNAVAILABLE")?
            {
                length = length
                    .checked_add(chunk.len() as u64)
                    .ok_or("NODE_PUBLIC_BYTES_MISMATCH")?;
                if length > f.size {
                    return Err("NODE_PUBLIC_BYTES_MISMATCH");
                }
                hash.update(&chunk);
            }
            if length != f.size || format!("{:x}", hash.finalize()) != f.sha256 {
                return Err("NODE_PUBLIC_BYTES_MISMATCH");
            }
        }
        Ok(())
    }
}
pub(super) async fn node(
    g: &ConfiguredGroup,
    w: &Work,
    m: &Manifest,
    payload: Option<Arc<Payload>>,
) -> Result<Receipt, &'static str> {
    tokio::time::timeout(Duration::from_secs(45), node_inner(g, w, m, payload))
        .await
        .map_err(|_| "NODE_DEADLINE")?
}
async fn node_inner(
    g: &ConfiguredGroup,
    w: &Work,
    m: &Manifest,
    payload: Option<Arc<Payload>>,
) -> Result<Receipt, &'static str> {
    let peer = Peer::new(g, &w.node_id)?;
    let verify = serde_json::to_vec(&zip::Verify {
        work: w.clone(),
        manifest: m.clone(),
    })
    .map_err(|_| "INVALID_MANIFEST")?;
    let (receipt, operation) = match peer.post("/agent/verify", verify).await? {
        Some(r) => (r, "VERIFY"),
        None => {
            let payload = payload.ok_or("NODE_ARTIFACT_MISSING")?;
            let work = w.clone();
            let encoded = tokio::task::spawn_blocking(move || {
                #[derive(Serialize)]
                struct Body<'a> {
                    work: &'a Work,
                    payload: &'a Payload,
                }
                serde_json::to_vec(&Body {
                    work: &work,
                    payload: &payload,
                })
            })
            .await
            .map_err(|_| "SOURCE_UNAVAILABLE")?
            .map_err(|_| "SOURCE_UNAVAILABLE")?;
            (
                peer.post("/agent/install", encoded)
                    .await?
                    .ok_or("NODE_ARTIFACT_MISSING")?,
                "INSTALL",
            )
        }
    };
    if !zip::receipt_matches(&receipt, w, m, operation) {
        return Err("UNTRUSTED_RECEIPT");
    }
    // File arrival is not inferred from a single LB response: check every direct HTTPS peer.
    peer.public_files(w, m).await?;
    Ok(receipt)
}
