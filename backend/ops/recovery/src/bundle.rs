use crate::{io, Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::{
    aead, digest,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
};
pub(crate) const MAX: u64 = 512 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub path: String,
    pub sha256: String,
    pub data: String,
    pub executable: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Database {
    pub kind: String,
    pub source_name: String,
    pub migrations: String,
    pub server_version: String,
    pub catalog: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bundle {
    pub format: u32,
    pub recovery_point: u64,
    pub databases: Vec<Database>,
    pub entries: Vec<Entry>,
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn hash(bytes: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, bytes).as_ref())
}
pub(crate) fn random() -> Result<[u8; 12]> {
    let mut n = [0; 12];
    SystemRandom::new()
        .fill(&mut n)
        .map_err(|_| Error("random generation failed"))?;
    Ok(n)
}
fn key(bytes: &[u8]) -> Result<aead::LessSafeKey> {
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, bytes)
            .map_err(|_| Error("key must contain 32 random bytes"))?,
    ))
}
const AAD: &[u8] = b"SAAS-RECOVERY-1";
pub(crate) fn seal(bundle: &Bundle, k: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(bundle).map_err(|_| Error("bundle serialization failed"))?;
    if bytes.len() as u64 > MAX {
        return Err(Error("pilot bundle capacity exceeded"));
    }
    let n = random()?;
    key(k)?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(n),
            aead::Aad::from(AAD),
            &mut bytes,
        )
        .map_err(|_| Error("encryption failed"))?;
    let mut result = AAD.to_vec();
    result.extend_from_slice(&n);
    result.extend(bytes);
    Ok(result)
}
pub(crate) fn open(bytes: &[u8], k: &[u8]) -> Result<Bundle> {
    if bytes.len() < AAD.len() + 12 + 16
        || bytes.len() as u64 > MAX + 64
        || &bytes[..AAD.len()] != AAD
    {
        return Err(Error("empty, invalid or oversized bundle"));
    }
    let mut n = [0; 12];
    n.copy_from_slice(&bytes[AAD.len()..AAD.len() + 12]);
    let mut body = bytes[AAD.len() + 12..].to_vec();
    let plain = key(k)?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(n),
            aead::Aad::from(AAD),
            &mut body,
        )
        .map_err(|_| Error("bundle authentication failed"))?;
    let b: Bundle = serde_json::from_slice(plain).map_err(|_| Error("bundle manifest invalid"))?;
    validate(&b)?;
    Ok(b)
}
pub(crate) fn safe_path(s: &str) -> bool {
    !s.is_empty()
        && s.split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && !s.contains('\\')
        && !s.contains(':')
        && !s.contains('\0')
        && Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && !s.starts_with('/')
}

fn portable_relative(path: &Path) -> Result<String> {
    let parts = path
        .components()
        .map(|part| match part {
            Component::Normal(name) => name.to_str().ok_or(Error("artifact path must be UTF-8")),
            _ => Err(Error("artifact path cannot be restored safely")),
        })
        .collect::<Result<Vec<_>>>()?;
    let path = parts.join("/");
    if !safe_path(&path) {
        return Err(Error("artifact path cannot be restored safely"));
    }
    Ok(path)
}
pub(crate) fn entry(path: String, bytes: Vec<u8>) -> Entry {
    Entry {
        path,
        sha256: hash(&bytes),
        data: STANDARD.encode(bytes),
        executable: false,
    }
}
pub(crate) fn decode(e: &Entry) -> Result<Vec<u8>> {
    let bytes = STANDARD
        .decode(&e.data)
        .map_err(|_| Error("entry encoding invalid"))?;
    if hash(&bytes) != e.sha256 {
        return Err(Error("entry hash mismatch"));
    }
    Ok(bytes)
}
pub(crate) fn validate(b: &Bundle) -> Result<()> {
    let kinds: BTreeSet<_> = b.databases.iter().map(|d| d.kind.as_str()).collect();
    if b.format != 1
        || b.recovery_point == 0
        || b.databases.len() != 3
        || kinds != BTreeSet::from(["platform", "stores", "work_orders"])
        || b.entries.len() > 20000
    {
        return Err(Error("incomplete bundle manifest"));
    }
    let names: BTreeSet<_> = b.databases.iter().map(|d| d.source_name.as_str()).collect();
    if names.len() != 3 {
        return Err(Error("source databases must be distinct"));
    }
    let mut paths = BTreeSet::new();
    for e in &b.entries {
        if !safe_path(&e.path) || !paths.insert(e.path.as_str()) {
            return Err(Error("unsafe or duplicate entry path"));
        }
        decode(e)?;
    }
    // Every entry is a regular file. Reject file/directory collisions while
    // authenticating the bundle, before any consumer can materialize it.
    for path in &paths {
        let mut ancestor = *path;
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if paths.contains(parent) {
                return Err(Error("entry path conflicts with a parent file"));
            }
            ancestor = parent;
        }
    }
    for d in &b.databases {
        if !crate::config::identifier(&d.source_name)
            || d.migrations.trim().is_empty()
            || (d.kind != "platform" && d.catalog.trim().is_empty())
            || !paths.contains(format!("db/{}.dump", d.kind).as_str())
        {
            return Err(Error("missing database artifact"));
        }
    }
    for required in [
        "artifacts/routes/nginx.conf",
        "artifacts/images/lock.json",
        "artifacts/host/index.html",
        "artifacts/stores/v1/index.html",
        "artifacts/stores/v2/index.html",
        "artifacts/work-orders/v1/index.html",
        "artifacts/work-orders/v2/index.html",
    ] {
        if !paths.contains(required) {
            return Err(Error("missing required release artifact"));
        }
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Image {
        app: String,
        version: String,
        image_id: String,
        archive_sha256: String,
        archive_bytes: u64,
    }
    let lock = b
        .entries
        .iter()
        .find(|e| e.path == "artifacts/images/lock.json")
        .ok_or(Error("image lock missing"))?;
    let images: Vec<Image> =
        serde_json::from_slice(&decode(lock)?).map_err(|_| Error("image lock invalid"))?;
    let mut identities = BTreeSet::new();
    for image in &images {
        if !["stores", "work-orders"].contains(&image.app.as_str())
            || !["v1", "v2"].contains(&image.version.as_str())
            || !identities.insert((&image.app, &image.version))
            || image.image_id.len() != 71
            || !image.image_id.starts_with("sha256:")
            || !image.image_id[7..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error("image identity invalid"));
        }
        let archive = b
            .entries
            .iter()
            .find(|e| e.path == format!("artifacts/images/{}-{}.tar", image.app, image.version))
            .ok_or(Error("image archive missing"))?;
        let bytes = decode(archive)?;
        if hash(&bytes) != image.archive_sha256 || bytes.len() as u64 != image.archive_bytes {
            return Err(Error("image archive differs from lock"));
        }
    }
    if images.len() != 4 {
        return Err(Error("four frontend image versions required"));
    }
    Ok(())
}
pub(crate) fn collect(root: &Path, dir: &Path, entries: &mut Vec<Entry>) -> Result<()> {
    let metadata = io(fs::symlink_metadata(dir))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error("artifact directory symlink refused"));
    }
    for item in io(fs::read_dir(dir))? {
        let p = io(item)?.path();
        let m = io(fs::symlink_metadata(&p))?;
        if m.file_type().is_symlink() {
            return Err(Error("artifact symlink refused"));
        }
        if m.is_dir() {
            collect(root, &p, entries)?;
        } else if m.is_file() {
            if m.len() > MAX || entries.len() >= 20000 {
                return Err(Error("artifact capacity exceeded"));
            }
            let relative = p
                .strip_prefix(root)
                .map_err(|_| Error("artifact root escaped"))?;
            if entries.iter().map(|e| e.data.len() as u64).sum::<u64>() + m.len() * 4 / 3 > MAX {
                return Err(Error("aggregate artifact capacity exceeded"));
            }
            let relative = portable_relative(relative)?;
            let item = entry(format!("artifacts/{relative}"), io(fs::read(p))?);
            #[cfg(unix)]
            let item = {
                use std::os::unix::fs::PermissionsExt;
                let mut item = item;
                item.executable = m.permissions().mode() & 0o111 != 0;
                item
            };
            entries.push(item);
        } else {
            return Err(Error("artifact must be regular file"));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    fn complete_bundle() -> Bundle {
        let mut b = Bundle {
            format: 1,
            recovery_point: 1,
            databases: vec![],
            entries: vec![],
        };
        for kind in ["platform", "stores", "work_orders"] {
            b.databases.push(Database {
                kind: kind.into(),
                source_name: format!("source_{kind}"),
                migrations: "1:checksum:true".into(),
                server_version: "17".into(),
                catalog: "catalog".into(),
            });
            b.entries
                .push(entry(format!("db/{kind}.dump"), b"dump".to_vec()));
        }
        for path in ["routes/nginx.conf", "host/index.html"] {
            b.entries
                .push(entry(format!("artifacts/{path}"), b"content".to_vec()));
        }
        let mut images = Vec::new();
        for app in ["stores", "work-orders"] {
            for version in ["v1", "v2"] {
                b.entries.push(entry(
                    format!("artifacts/{app}/{version}/index.html"),
                    b"<html>fixture</html>".to_vec(),
                ));
                let archive = format!("image-{app}-{version}").into_bytes();
                images.push(serde_json::json!({
                    "app":app,"version":version,"imageId":format!("sha256:{}", "a".repeat(64)),
                    "archiveSha256":hash(&archive),"archiveBytes":archive.len(),
                }));
                b.entries.push(entry(
                    format!("artifacts/images/{app}-{version}.tar"),
                    archive,
                ));
            }
        }
        b.entries.push(entry(
            "artifacts/images/lock.json".into(),
            serde_json::to_vec(&images).unwrap(),
        ));
        b
    }

    #[test]
    fn authenticated_bundle_refuses_unrestorable_paths_before_restore() {
        let mut b = complete_bundle();
        assert!(validate(&b).is_ok());
        b.entries.push(entry(
            "artifacts/host/assets/app.js".into(),
            b"script".to_vec(),
        ));
        // The parent file may come after its child in the serialized manifest.
        b.entries.push(entry(
            "artifacts/host/assets".into(),
            b"parent file".to_vec(),
        ));
        assert_eq!(
            validate(&b).unwrap_err().0,
            "entry path conflicts with a parent file"
        );
        let encrypted = seal(&b, &[1; 32]).unwrap();
        assert_eq!(
            open(&encrypted, &[1; 32]).err().unwrap().0,
            "entry path conflicts with a parent file"
        );

        let mut b = complete_bundle();
        b.entries.push(entry(
            "artifacts/host/bad\0path".into(),
            b"content".to_vec(),
        ));
        assert_eq!(
            validate(&b).unwrap_err().0,
            "unsafe or duplicate entry path"
        );
    }
    #[test]
    fn rejects_unsafe_paths() {
        for s in [
            "",
            "/etc/passwd",
            "../x",
            "a/../x",
            "x\\y",
            "c:x",
            "bad\0path",
        ] {
            assert!(!safe_path(s));
        }
        assert!(safe_path("stores/v1/index.html"));
    }

    #[test]
    fn collected_file_names_are_preserved_without_lossy_aliases() {
        let path = Path::new("stores").join("v1").join("index.html");
        assert_eq!(portable_relative(&path).unwrap(), "stores/v1/index.html");
        assert!(portable_relative(Path::new("a/../index.html")).is_err());
        #[cfg(unix)]
        {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt;
            let invalid = OsString::from_vec(vec![b'x', 0xff]);
            assert_eq!(
                portable_relative(Path::new(&invalid)).unwrap_err().0,
                "artifact path must be UTF-8"
            );
            // Backslash is a literal Unix filename character, not a separator.
            assert!(portable_relative(Path::new("stores\\index.html")).is_err());
        }
    }
    #[test]
    fn malformed_and_wrong_keys_fail() {
        for b in [vec![], vec![0; 100]] {
            assert!(open(&b, &[1; 32]).is_err());
        }
        let b = Bundle {
            format: 1,
            recovery_point: 1,
            databases: vec![],
            entries: vec![],
        };
        let mut bytes = seal(&b, &[1; 32]).unwrap();
        assert!(open(&bytes, &[2; 32]).is_err());
        *bytes.last_mut().unwrap() ^= 1;
        assert!(open(&bytes, &[1; 32]).is_err());
    }
    #[test]
    fn missing_artifacts_fail() {
        assert!(validate(&Bundle {
            format: 1,
            recovery_point: 1,
            databases: vec![],
            entries: vec![]
        })
        .is_err());
    }
}
