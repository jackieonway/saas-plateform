//! Bounded authenticated file streams for private UUID rekey exports.
//!
//! The purpose must identify the batch and logical file, not just its media
//! type. Callers may consume decrypted output only after `open_file` succeeds.
// The stream implementation is tested on every host; file operations are
// intentionally unavailable on non-Linux systems, so those helpers have no
// production caller there.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
use crate::{Error, Result};
use ring::{
    aead, digest, hkdf,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"SAASRKF1";
const KEY_DOMAIN: &[u8] = b"saas-uuid-rekey-file-key-v1\0";
const MAX_CHUNK: usize = 1024 * 1024;
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const FRAME_HEADER_BYTES: usize = 1 + 8 + 4 + NONCE_BYTES;
const FOOTER_BYTES: usize = 32 + 8 + 8;
const DATA: u8 = 0;
const FINAL: u8 = 1;

/// Digest of the authenticated plaintext; `frames` excludes the final footer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileDigest {
    pub(crate) sha256: String,
    pub(crate) plaintext_bytes: u64,
    pub(crate) frames: u64,
}

fn validate_contract(key: &[u8], purpose: &str) -> Result<()> {
    if key.len() != 32 {
        return Err(Error("file encryption key must contain 32 bytes"));
    }
    if purpose.is_empty() || purpose.len() > 256 || purpose.chars().any(char::is_control) {
        return Err(Error("invalid authenticated file purpose"));
    }
    Ok(())
}

struct KeyLength;
impl hkdf::KeyType for KeyLength {
    fn len(&self) -> usize {
        32
    }
}

fn frame_key(prk: &hkdf::Prk, purpose: &str, index: u64, flag: u8) -> Result<aead::LessSafeKey> {
    let index = index.to_be_bytes();
    let flag = [flag];
    let info = [
        KEY_DOMAIN,
        purpose.as_bytes(),
        index.as_slice(),
        flag.as_slice(),
    ];
    let mut bytes = [0_u8; 32];
    prk.expand(&info, KeyLength)
        .map_err(|_| Error("file key derivation failed"))?
        .fill(&mut bytes)
        .map_err(|_| Error("file key derivation failed"))?;
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &bytes)
            .map_err(|_| Error("file key derivation failed"))?,
    ))
}

fn aad(salt: &[u8; SALT_BYTES], purpose: &str, flag: u8, index: u64, len: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(MAGIC.len() + SALT_BYTES + 4 + purpose.len() + 13);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(salt);
    bytes.extend_from_slice(&(purpose.len() as u32).to_be_bytes());
    bytes.extend_from_slice(purpose.as_bytes());
    bytes.push(flag);
    bytes.extend_from_slice(&index.to_be_bytes());
    bytes.extend_from_slice(&len.to_be_bytes());
    bytes
}

fn write_frame<W: Write>(
    output: &mut W,
    prk: &hkdf::Prk,
    salt: &[u8; SALT_BYTES],
    purpose: &str,
    flag: u8,
    index: u64,
    bytes: &mut Vec<u8>,
) -> Result<()> {
    let len = u32::try_from(bytes.len()).map_err(|_| Error("file frame too large"))?;
    let mut nonce = [0_u8; NONCE_BYTES];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| Error("file random generation failed"))?;
    // Each ordinal has its own derived key, so random nonce collisions between
    // frames do not repeat a GCM key/nonce pair even for large exports.
    frame_key(prk, purpose, index, flag)?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(aad(salt, purpose, flag, index, len)),
            bytes,
        )
        .map_err(|_| Error("file frame encryption failed"))?;
    let mut header = [0_u8; FRAME_HEADER_BYTES];
    header[0] = flag;
    header[1..9].copy_from_slice(&index.to_be_bytes());
    header[9..13].copy_from_slice(&len.to_be_bytes());
    header[13..].copy_from_slice(&nonce);
    output
        .write_all(&header)
        .and_then(|_| output.write_all(bytes))
        .map_err(|_| Error("encrypted file write failed"))
}

fn digest_record(hash: &[u8], plaintext_bytes: u64, frames: u64) -> FileDigest {
    FileDigest {
        sha256: crate::bundle::hex(hash),
        plaintext_bytes,
        frames,
    }
}

fn seal_stream<R: Read, W: Write>(
    input: &mut R,
    output: &mut W,
    key: &[u8],
    purpose: &str,
) -> Result<FileDigest> {
    validate_contract(key, purpose)?;
    let mut salt = [0_u8; SALT_BYTES];
    SystemRandom::new()
        .fill(&mut salt)
        .map_err(|_| Error("file random generation failed"))?;
    output
        .write_all(MAGIC)
        .and_then(|_| output.write_all(&salt))
        .map_err(|_| Error("encrypted file write failed"))?;
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(key);
    let mut hash = digest::Context::new(&digest::SHA256);
    let mut plaintext_bytes = 0_u64;
    let mut frames = 0_u64;
    let mut buffer = Vec::with_capacity(MAX_CHUNK + TAG_BYTES);
    loop {
        buffer.clear();
        input
            .by_ref()
            .take(MAX_CHUNK as u64)
            .read_to_end(&mut buffer)
            .map_err(|_| Error("plaintext file read failed"))?;
        if buffer.is_empty() {
            break;
        }
        hash.update(&buffer);
        plaintext_bytes = plaintext_bytes
            .checked_add(buffer.len() as u64)
            .ok_or(Error("plaintext byte count overflow"))?;
        write_frame(output, &prk, &salt, purpose, DATA, frames, &mut buffer)?;
        frames = frames
            .checked_add(1)
            .ok_or(Error("file frame count overflow"))?;
    }
    let hash = hash.finish();
    let result = digest_record(hash.as_ref(), plaintext_bytes, frames);
    buffer.clear();
    buffer.extend_from_slice(hash.as_ref());
    buffer.extend_from_slice(&plaintext_bytes.to_be_bytes());
    buffer.extend_from_slice(&frames.to_be_bytes());
    write_frame(output, &prk, &salt, purpose, FINAL, frames, &mut buffer)?;
    output
        .flush()
        .map_err(|_| Error("encrypted file flush failed"))?;
    Ok(result)
}

fn u64_from(bytes: &[u8]) -> Result<u64> {
    Ok(u64::from_be_bytes(
        bytes
            .try_into()
            .map_err(|_| Error("invalid encrypted file integer"))?,
    ))
}

pub(super) fn open_stream<R: Read, W: Write>(
    input: &mut R,
    output: &mut W,
    key: &[u8],
    purpose: &str,
) -> Result<FileDigest> {
    validate_contract(key, purpose)?;
    let mut magic = [0_u8; MAGIC.len()];
    input
        .read_exact(&mut magic)
        .map_err(|_| Error("encrypted file truncated"))?;
    if &magic != MAGIC {
        return Err(Error("unsupported encrypted file format"));
    }
    let mut salt = [0_u8; SALT_BYTES];
    input
        .read_exact(&mut salt)
        .map_err(|_| Error("encrypted file truncated"))?;
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(key);
    let mut hash = digest::Context::new(&digest::SHA256);
    let mut plaintext_bytes = 0_u64;
    let mut frames = 0_u64;
    let mut buffer = Vec::with_capacity(MAX_CHUNK + TAG_BYTES);
    loop {
        let mut header = [0_u8; FRAME_HEADER_BYTES];
        input
            .read_exact(&mut header)
            .map_err(|_| Error("encrypted file truncated"))?;
        let flag = header[0];
        let index = u64_from(&header[1..9])?;
        let len = u32::from_be_bytes(
            header[9..13]
                .try_into()
                .map_err(|_| Error("invalid encrypted file length"))?,
        ) as usize;
        if index != frames
            || match flag {
                DATA => len == 0 || len > MAX_CHUNK,
                FINAL => len != FOOTER_BYTES,
                _ => true,
            }
        {
            return Err(Error("invalid encrypted file frame"));
        }
        let mut nonce = [0_u8; NONCE_BYTES];
        nonce.copy_from_slice(&header[13..]);
        buffer.resize(len + TAG_BYTES, 0);
        input
            .read_exact(&mut buffer)
            .map_err(|_| Error("encrypted file truncated"))?;
        let bytes = frame_key(&prk, purpose, index, flag)?
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(aad(&salt, purpose, flag, index, len as u32)),
                &mut buffer,
            )
            .map_err(|_| Error("encrypted file authentication failed"))?;
        if flag == FINAL {
            let actual_hash = hash.finish();
            if bytes.len() != FOOTER_BYTES
                || bytes[..32] != *actual_hash.as_ref()
                || u64_from(&bytes[32..40])? != plaintext_bytes
                || u64_from(&bytes[40..48])? != frames
            {
                return Err(Error("encrypted file footer mismatch"));
            }
            let mut trailing = [0_u8; 1];
            if input
                .read(&mut trailing)
                .map_err(|_| Error("encrypted file read failed"))?
                != 0
            {
                return Err(Error("trailing encrypted file data"));
            }
            output
                .flush()
                .map_err(|_| Error("plaintext file flush failed"))?;
            return Ok(digest_record(actual_hash.as_ref(), plaintext_bytes, frames));
        }
        hash.update(bytes);
        plaintext_bytes = plaintext_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error("plaintext byte count overflow"))?;
        output
            .write_all(bytes)
            .map_err(|_| Error("plaintext file write failed"))?;
        frames = frames
            .checked_add(1)
            .ok_or(Error("file frame count overflow"))?;
    }
}

/// Seal a private tmpfs plaintext file into a new mode-0600 encrypted file.
/// Existing output, symlinks and non-Linux file paths are rejected.
pub(crate) fn seal_file(
    input: &Path,
    output: &Path,
    key: &[u8],
    purpose: &str,
) -> Result<FileDigest> {
    #[cfg(target_os = "linux")]
    {
        validate_contract(key, purpose)?;
        file_io::require_plaintext_parent(input)?;
        let mut input = file_io::open_input(input)?;
        let mut pending = file_io::Pending::create(output)?;
        let result = seal_stream(&mut input, &mut pending.file, key, purpose)?;
        pending.commit()?;
        Ok(result)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (input, output, key, purpose);
        Err(Error("authenticated file operations require Linux tmpfs"))
    }
}

/// Authenticate a complete private file, including its final digest and footer,
/// without writing decrypted bytes to a plaintext file.
pub(crate) fn verify_file(input: &Path, key: &[u8], purpose: &str) -> Result<FileDigest> {
    #[cfg(target_os = "linux")]
    {
        validate_contract(key, purpose)?;
        let mut input = file_io::open_input(input)?;
        open_stream(&mut input, &mut std::io::sink(), key, purpose)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (input, key, purpose);
        Err(Error("authenticated file verification requires Linux"))
    }
}

/// Open into a new private tmpfs file. The path stays quarantined until success;
/// any failure removes only the output inode created by this invocation.
pub(crate) fn open_file(
    input: &Path,
    output: &Path,
    key: &[u8],
    purpose: &str,
) -> Result<FileDigest> {
    #[cfg(target_os = "linux")]
    {
        validate_contract(key, purpose)?;
        file_io::require_plaintext_parent(output)?;
        let mut input = file_io::open_input(input)?;
        let mut pending = file_io::Pending::create(output)?;
        let result = open_stream(&mut input, &mut pending.file, key, purpose)?;
        pending.commit()?;
        Ok(result)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (input, output, key, purpose);
        Err(Error("authenticated file operations require Linux tmpfs"))
    }
}

#[cfg(target_os = "linux")]
mod file_io {
    use super::*;
    use std::fs::{self, File, Metadata, OpenOptions};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Component, PathBuf};

    fn no_links(path: &Path) -> Result<()> {
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        {
            return Err(Error("absolute normalized file path required"));
        }
        for ancestor in path.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(Error("symlink file path rejected"))
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error("file path inspection failed")),
            }
        }
        Ok(())
    }

    fn private(meta: &Metadata) -> Result<()> {
        let current = fs::metadata("/proc/self")
            .map_err(|_| Error("file ownership check failed"))?
            .uid();
        if meta.uid() != current || meta.mode() & 0o077 != 0 {
            return Err(Error("file must be privately owned"));
        }
        Ok(())
    }

    fn private_output(meta: &Metadata) -> Result<()> {
        private(meta)?;
        if !meta.is_file() || meta.mode() & 0o7777 != 0o600 {
            return Err(Error("output file must be owned mode 0600"));
        }
        Ok(())
    }

    pub(super) fn require_plaintext_parent(path: &Path) -> Result<()> {
        no_links(path)?;
        let parent = path.parent().ok_or(Error("file parent missing"))?;
        crate::native::require_tmpfs_directory(parent)?;
        let parent_meta =
            fs::metadata(parent).map_err(|_| Error("plaintext parent inspection failed"))?;
        private(&parent_meta)?;
        // A bind-mounted ordinary-disk file inside tmpfs must not satisfy the
        // plaintext policy just because its parent lives in memory.
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.dev() != parent_meta.dev() => {
                Err(Error("plaintext file must be on the same tmpfs"))
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(Error("plaintext file inspection failed")),
        }
    }

    pub(super) fn open_input(path: &Path) -> Result<File> {
        no_links(path)?;
        let original =
            fs::symlink_metadata(path).map_err(|_| Error("input file inspection failed"))?;
        if !original.is_file() {
            return Err(Error("regular private input file required"));
        }
        private(&original)?;
        let file = File::open(path).map_err(|_| Error("input file open failed"))?;
        let opened = file
            .metadata()
            .map_err(|_| Error("input file inspection failed"))?;
        if !opened.is_file() || opened.dev() != original.dev() || opened.ino() != original.ino() {
            return Err(Error("input file identity changed"));
        }
        Ok(file)
    }

    pub(super) struct Pending {
        pub(super) file: File,
        path: PathBuf,
        committed: bool,
    }

    impl Pending {
        pub(super) fn create(path: &Path) -> Result<Self> {
            no_links(path)?;
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .map_err(|_| Error("new private output file required"))?;
            let pending = Self {
                file,
                path: path.to_path_buf(),
                committed: false,
            };
            // create_new's mode is filtered by the process umask. Normalize
            // this fresh descriptor before writing, with Drop already armed.
            pending
                .file
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| Error("output file permission setup failed"))?;
            private_output(
                &pending
                    .file
                    .metadata()
                    .map_err(|_| Error("output file identity changed"))?,
            )?;
            Ok(pending)
        }

        fn validate_output(&self) -> Result<()> {
            no_links(&self.path)?;
            let actual = fs::symlink_metadata(&self.path)
                .map_err(|_| Error("output file identity changed"))?;
            let created = self
                .file
                .metadata()
                .map_err(|_| Error("output file identity changed"))?;
            if !actual.is_file() || actual.dev() != created.dev() || actual.ino() != created.ino() {
                return Err(Error("output file identity changed"));
            }
            private_output(&actual)?;
            private_output(&created)
        }

        pub(super) fn commit(&mut self) -> Result<()> {
            self.validate_output()?;
            self.file
                .sync_all()
                .map_err(|_| Error("output file sync failed"))?;
            let parent = self.path.parent().ok_or(Error("file parent missing"))?;
            File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(|_| Error("output directory sync failed"))?;
            self.validate_output()?;
            self.committed = true;
            Ok(())
        }
    }

    impl Drop for Pending {
        fn drop(&mut self) {
            if self.committed {
                return;
            }
            // A caller replacement must not be removed on authentication error.
            if let (Ok(actual), Ok(created)) =
                (fs::symlink_metadata(&self.path), self.file.metadata())
            {
                if actual.is_file()
                    && actual.dev() == created.dev()
                    && actual.ino() == created.ino()
                {
                    let _ = fs::remove_file(&self.path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    struct PrivateFixture(std::path::PathBuf);

    #[cfg(target_os = "linux")]
    impl PrivateFixture {
        fn new() -> Self {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            let mut nonce = [0_u8; 12];
            SystemRandom::new().fill(&mut nonce).unwrap();
            let path = std::path::PathBuf::from("/dev/shm").join(format!(
                "uuid-codec-adversarial-{}",
                crate::bundle::hex(&nonce)
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            // This fixture also runs under an intentionally restrictive umask.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for PrivateFixture {
        fn drop(&mut self) {
            if self.0.parent() == Some(Path::new("/dev/shm"))
                && self.0.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .starts_with("uuid-codec-adversarial-")
                })
            {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
    use std::io::Cursor;

    const KEY: [u8; 32] = [0x61; 32];
    const PURPOSE: &str = "fixture-batch/platform/users.ndjson";

    fn seal(bytes: &[u8]) -> (Vec<u8>, FileDigest) {
        let mut encrypted = Vec::new();
        let summary = seal_stream(&mut Cursor::new(bytes), &mut encrypted, &KEY, PURPOSE).unwrap();
        (encrypted, summary)
    }

    fn open(bytes: &[u8], key: &[u8], purpose: &str) -> Result<(Vec<u8>, FileDigest)> {
        let mut plaintext = Vec::new();
        let summary = open_stream(&mut Cursor::new(bytes), &mut plaintext, key, purpose)?;
        Ok((plaintext, summary))
    }

    #[test]
    fn bounded_frames_round_trip_empty_partial_and_multiple_chunks() {
        for size in [
            0,
            1,
            MAX_CHUNK - 1,
            MAX_CHUNK,
            MAX_CHUNK + 1,
            MAX_CHUNK * 3 + 7,
        ] {
            let bytes: Vec<_> = (0..size).map(|index| (index % 251) as u8).collect();
            let (encrypted, sealed) = seal(&bytes);
            let (plaintext, opened) = open(&encrypted, &KEY, PURPOSE).unwrap();
            assert_eq!(plaintext, bytes);
            assert_eq!(opened, sealed);
            assert_eq!(opened.plaintext_bytes, size as u64);
            assert_eq!(opened.frames, size.div_ceil(MAX_CHUNK) as u64);
            assert_eq!(opened.sha256, crate::bundle::hash(&bytes));
        }
    }

    #[test]
    fn streaming_export_can_exceed_the_old_bundle_capacity_without_a_total_buffer() {
        let size = 513 * 1024 * 1024;
        let result = seal_stream(
            &mut std::io::repeat(0).take(size),
            &mut std::io::sink(),
            &KEY,
            PURPOSE,
        )
        .unwrap();
        assert_eq!(result.plaintext_bytes, size);
        assert_eq!(result.frames, 513);
    }

    #[test]
    fn key_purpose_header_ciphertext_and_footer_are_authenticated() {
        let (original, _) = seal(b"private fixture data");
        assert!(open(&original, &[0x62; 32], PURPOSE).is_err());
        assert!(open(&original, &KEY, "fixture-batch/platform/roles.ndjson").is_err());
        assert!(open(&original, &[0; 31], PURPOSE).is_err());
        for position in [
            0,
            MAGIC.len(),
            MAGIC.len() + SALT_BYTES + 13,
            original.len() - 1,
        ] {
            let mut corrupt = original.clone();
            corrupt[position] ^= 1;
            assert!(open(&corrupt, &KEY, PURPOSE).is_err());
        }
        for length in [0, 7, MAGIC.len() + SALT_BYTES, original.len() - 1] {
            assert!(open(&original[..length], &KEY, PURPOSE).is_err());
        }
        let mut trailing = original.clone();
        trailing.push(0);
        assert!(open(&trailing, &KEY, PURPOSE).is_err());
        assert_ne!(seal(b"private fixture data").0, original);
    }

    #[test]
    fn an_authenticated_footer_must_match_actual_hash_byte_count_and_frame_count() {
        let (original, _) = seal(b"x");
        let start = MAGIC.len() + SALT_BYTES;
        let footer_start = start + FRAME_HEADER_BYTES + 1 + TAG_BYTES;
        let mut salt = [0_u8; SALT_BYTES];
        salt.copy_from_slice(&original[MAGIC.len()..start]);
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(&KEY);
        let hash = digest::digest(&digest::SHA256, b"x");
        for wrong_field in [0, 32, 40] {
            let mut footer = hash.as_ref().to_vec();
            footer.extend_from_slice(&1_u64.to_be_bytes());
            footer.extend_from_slice(&1_u64.to_be_bytes());
            footer[wrong_field] ^= 1;
            let mut forged = original[..footer_start].to_vec();
            write_frame(&mut forged, &prk, &salt, PURPOSE, FINAL, 1, &mut footer).unwrap();
            assert!(open(&forged, &KEY, PURPOSE).is_err());
        }
    }

    #[test]
    fn frames_cannot_be_reordered_duplicated_removed_or_spliced() {
        let (original, _) = seal(&vec![0x42; MAX_CHUNK * 2]);
        let (other, _) = seal(&vec![0x43; MAX_CHUNK * 2]);
        let start = MAGIC.len() + SALT_BYTES;
        let frame_len = FRAME_HEADER_BYTES + MAX_CHUNK + TAG_BYTES;
        let second = start + frame_len;
        let footer = second + frame_len;
        let mut reordered = original.clone();
        reordered[start..second].copy_from_slice(&original[second..footer]);
        reordered[second..footer].copy_from_slice(&original[start..second]);
        assert!(open(&reordered, &KEY, PURPOSE).is_err());
        let mut duplicate = original[..second].to_vec();
        duplicate.extend_from_slice(&original[start..]);
        assert!(open(&duplicate, &KEY, PURPOSE).is_err());
        let mut removed = original[..second].to_vec();
        removed.extend_from_slice(&original[footer..]);
        assert!(open(&removed, &KEY, PURPOSE).is_err());
        let mut spliced = original.clone();
        spliced[start..second].copy_from_slice(&other[start..second]);
        assert!(open(&spliced, &KEY, PURPOSE).is_err());
        let mut oversized = original;
        oversized[start + 9..start + 13].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(open(&oversized, &KEY, PURPOSE).is_err());
    }

    #[test]
    fn invalid_contract_and_read_write_failures_never_report_success() {
        assert!(seal_stream(&mut Cursor::new(b"x"), &mut Vec::new(), &KEY, "").is_err());
        assert!(seal_stream(
            &mut Cursor::new(b"x"),
            &mut Vec::new(),
            &KEY,
            &"x".repeat(257)
        )
        .is_err());
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("fixture-only failure"))
            }
        }
        assert!(seal_stream(&mut Broken, &mut Vec::new(), &KEY, PURPOSE).is_err());
        let (encrypted, _) = seal(b"private fixture data");
        let mut limited = [0_u8; 1];
        assert!(open_stream(
            &mut Cursor::new(encrypted),
            &mut limited.as_mut_slice(),
            &KEY,
            PURPOSE
        )
        .is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_file_wrappers_keep_plaintext_private_and_remove_only_failed_outputs() {
        use std::fs;
        use std::os::unix::fs::{symlink, DirBuilderExt, OpenOptionsExt, PermissionsExt};
        let mut nonce = [0_u8; 12];
        SystemRandom::new().fill(&mut nonce).unwrap();
        let root = std::path::PathBuf::from("/dev/shm")
            .join(format!("uuid-codec-test-{}", crate::bundle::hex(&nonce)));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let plain = root.join("source.ndjson");
        let encrypted = root.join("source.sealed");
        let opened = root.join("opened.ndjson");
        let mut source = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&plain)
            .unwrap();
        source.write_all(b"private fixture data").unwrap();
        source.sync_all().unwrap();
        drop(source);
        let sealed = seal_file(&plain, &encrypted, &KEY, PURPOSE).unwrap();
        assert_eq!(verify_file(&encrypted, &KEY, PURPOSE).unwrap(), sealed);
        assert!(verify_file(&encrypted, &KEY, "wrong-purpose").is_err());
        assert_eq!(
            open_file(&encrypted, &opened, &KEY, PURPOSE).unwrap(),
            sealed
        );
        assert_eq!(fs::read(&opened).unwrap(), b"private fixture data");
        assert_eq!(
            fs::metadata(&opened).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(open_file(&encrypted, &opened, &KEY, PURPOSE).is_err());
        assert_eq!(fs::read(&opened).unwrap(), b"private fixture data");
        let failed = root.join("failed.ndjson");
        assert!(open_file(&encrypted, &failed, &KEY, "wrong-purpose").is_err());
        assert!(!failed.exists());
        let corrupted = root.join("bad-footer.sealed");
        let mut ciphertext = fs::read(&encrypted).unwrap();
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 1;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&corrupted)
            .unwrap();
        file.write_all(&ciphertext).unwrap();
        drop(file);
        assert!(verify_file(&corrupted, &KEY, PURPOSE).is_err());
        assert!(open_file(&corrupted, &failed, &KEY, PURPOSE).is_err());
        assert!(
            !failed.exists(),
            "late footer failure must remove partial plaintext"
        );
        let link = root.join("input-link");
        symlink(&encrypted, &link).unwrap();
        assert!(open_file(&link, &failed, &KEY, PURPOSE).is_err());
        assert!(!failed.exists());
        symlink(&plain, &failed).unwrap();
        assert!(open_file(&encrypted, &failed, &KEY, PURPOSE).is_err());
        assert!(fs::symlink_metadata(&failed)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&plain).unwrap(), b"private fixture data");
        let changed = root.join("changed.ndjson");
        let moved = root.join("moved.ndjson");
        let mut pending = file_io::Pending::create(&changed).unwrap();
        pending.file.write_all(b"pending plaintext").unwrap();
        fs::rename(&changed, &moved).unwrap();
        fs::write(&changed, b"replacement owned by caller").unwrap();
        assert!(pending.commit().is_err());
        drop(pending);
        assert_eq!(fs::read(&changed).unwrap(), b"replacement owned by caller");
        assert!(open_file(
            &encrypted,
            Path::new("/tmp/uuid-codec-must-not-be-created"),
            &KEY,
            PURPOSE
        )
        .is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_commit_rechecks_exact_output_permissions_and_current_owner() {
        use std::fs;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let fixture = PrivateFixture::new();
        let current = fs::metadata("/proc/self").unwrap().uid();
        for mode in [0o400, 0o700, 0o640, 0o604, 0o4600] {
            let path = fixture.0.join(format!("mode-{mode:o}"));
            let mut pending = file_io::Pending::create(&path).unwrap();
            pending.file.write_all(b"private fixture").unwrap();
            pending
                .file
                .set_permissions(fs::Permissions::from_mode(mode))
                .unwrap();
            assert_eq!(pending.file.metadata().unwrap().mode() & 0o7777, mode);
            assert!(pending.commit().is_err());
            drop(pending);
            assert!(!path.exists());
        }
        let valid = fixture.0.join("valid");
        let mut pending = file_io::Pending::create(&valid).unwrap();
        pending.file.write_all(b"private fixture").unwrap();
        pending.commit().unwrap();
        drop(pending);
        let meta = fs::metadata(valid).unwrap();
        assert_eq!(meta.uid(), current);
        assert_eq!(meta.mode() & 0o7777, 0o600);
        if current == 0 {
            let path = fixture.0.join("foreign-owner");
            let mut pending = file_io::Pending::create(&path).unwrap();
            pending.file.write_all(b"private fixture").unwrap();
            let status = std::process::Command::new("chown")
                .arg("65534:65534")
                .arg(&path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
            assert_eq!(fs::metadata(&path).unwrap().mode() & 0o7777, 0o600);
            assert_ne!(fs::metadata(&path).unwrap().uid(), current);
            assert!(pending.commit().is_err());
            drop(pending);
            assert!(!path.exists());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_output_creation_normalizes_restrictive_umask_in_an_isolated_process() {
        use std::fs;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        const CHILD: &str = "SAAS_UUID_CODEC_UMASK_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new("sh")
                .args(["-c", "umask 0777; exec \"$@\"", "uuid-codec-umask"])
                .arg(std::env::current_exe().unwrap())
                .args(["--exact", "uuid_rekey::codec::tests::linux_output_creation_normalizes_restrictive_umask_in_an_isolated_process", "--nocapture"])
                .env(CHILD, "1")
                .status().unwrap();
            assert!(status.success());
            return;
        }
        let fixture = PrivateFixture::new();
        let plain = fixture.0.join("plain");
        let sealed = fixture.0.join("sealed");
        let opened = fixture.0.join("opened");
        let mut input = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&plain)
            .unwrap();
        input
            .set_permissions(fs::Permissions::from_mode(0o600))
            .unwrap();
        input.write_all(b"private umask fixture").unwrap();
        drop(input);
        let digest = seal_file(&plain, &sealed, &KEY, PURPOSE).unwrap();
        assert_eq!(open_file(&sealed, &opened, &KEY, PURPOSE).unwrap(), digest);
        let current = fs::metadata("/proc/self").unwrap().uid();
        for path in [&sealed, &opened] {
            let meta = fs::metadata(path).unwrap();
            assert_eq!(meta.uid(), current);
            assert_eq!(meta.mode() & 0o7777, 0o600);
        }
        assert_eq!(fs::read(opened).unwrap(), b"private umask fixture");
    }
}
