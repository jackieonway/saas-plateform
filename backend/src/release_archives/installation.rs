//! Immutable local installation. Only an entirely written payload is published.
use std::collections::HashSet;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{safe_path, PreparedArchive, MAX_FILES, MAX_FILE_BYTES, MAX_TOTAL_BYTES};
use crate::error::AppError;

pub(super) const MANIFEST_NAME: &str = ".artifact-manifest.json";
const STAGING_NAME: &str = ".staging";
const OWNER_NAME: &str = "owner.json";
const LOCK_NAME: &str = ".maintenance.lock";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtifactFile {
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtifactManifest {
    pub(crate) schema_version: u32,
    pub(crate) entry_type: String,
    pub(crate) entry: String,
    pub(crate) files: Vec<ArtifactFile>,
    pub(crate) artifact_sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DigestManifest<'a> {
    schema_version: u32,
    entry_type: &'a str,
    entry: &'a str,
    files: &'a [ArtifactFile],
}

fn conflict(message: &'static str) -> AppError {
    AppError::conflict(message)
}
fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl ArtifactManifest {
    pub(super) fn from_files(
        entry_type: &str,
        entry: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<Self, AppError> {
        let mut manifest = Self {
            schema_version: 1,
            entry_type: entry_type.to_owned(),
            entry: entry.to_owned(),
            files: files
                .iter()
                .map(|(path, data)| ArtifactFile {
                    path: path.clone(),
                    size: data.len() as u64,
                    sha256: sha256(data),
                })
                .collect(),
            artifact_sha256: String::new(),
        };
        manifest.artifact_sha256 = manifest.digest()?;
        Ok(manifest)
    }

    fn digest(&self) -> Result<String, AppError> {
        // Struct field order and the sorted UTF-8 path list define schema v1's
        // canonical JSON. Archive compression, timestamps and wrapper do not.
        let canonical = DigestManifest {
            schema_version: self.schema_version,
            entry_type: &self.entry_type,
            entry: &self.entry,
            files: &self.files,
        };
        serde_json::to_vec(&canonical)
            .map(|bytes| sha256(&bytes))
            .map_err(|_| conflict("无法计算产物摘要"))
    }

    fn validate(&self) -> Result<(), AppError> {
        let entry = match self.entry_type.as_str() {
            "HTML" => "index.html",
            "ESM" => "entry.mjs",
            _ => return Err(conflict("已有产物清单入口类型无效")),
        };
        if self.schema_version != 1
            || self.entry != entry
            || self.files.is_empty()
            || self.files.len() > MAX_FILES
        {
            return Err(conflict("已有产物清单结构无效"));
        }
        let mut seen = HashSet::new();
        let mut previous: Option<&str> = None;
        let mut total = 0_u64;
        for file in &self.files {
            if safe_path(&file.path).ok().as_deref() != Some(file.path.as_str())
                || file.path.eq_ignore_ascii_case("release.json")
                || previous.is_some_and(|name| name >= file.path.as_str())
                || !seen.insert(file.path.to_ascii_lowercase())
                || file.size == 0
                || file.size > MAX_FILE_BYTES as u64
                || file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(conflict("已有产物清单文件无效"));
            }
            total += file.size;
            previous = Some(&file.path);
        }
        if total > MAX_TOTAL_BYTES as u64
            || !self.files.iter().any(|file| file.path == entry)
            || self.artifact_sha256 != self.digest()?
        {
            return Err(conflict("已有产物清单摘要无效"));
        }
        Ok(())
    }
}

pub(crate) struct InstalledArchive {
    #[cfg(test)]
    pub(crate) path: PathBuf,
    pub(crate) created: bool,
    pub(crate) artifact_sha256: String,
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions and all other reparse points are rejected as well.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn check_no_symlinks(path: &Path) -> Result<(), AppError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(conflict("发布目录必须使用规范绝对路径"));
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(conflict("发布目录不能包含链接或重解析点"))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(conflict("无法检查发布目录")),
        }
    }
    Ok(())
}

fn storage_root(root: &str) -> Result<PathBuf, AppError> {
    if root.is_empty() {
        return Err(conflict("服务器未配置 MICRO_APPS_DIR，无法接收压缩包"));
    }
    let root = Path::new(root);
    if !root.is_absolute() {
        return Err(conflict("MICRO_APPS_DIR 必须是已存在的绝对目录"));
    }
    check_no_symlinks(root)?;
    if !root.is_dir() {
        return Err(conflict("MICRO_APPS_DIR 必须是已存在的绝对目录"));
    }
    Ok(root.to_owned())
}

fn validate_identity(app: &str, version: &str) -> Result<(), AppError> {
    if !(2..=32).contains(&app.len())
        || !app
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !app
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(conflict("应用编码不适合静态版本目录"));
    }
    crate::releases::validate_version(version)?;
    safe_path(app)?;
    safe_path(version)?;
    Ok(())
}

#[cfg(windows)]
fn reject_case_alias(path: &Path) -> Result<(), AppError> {
    let (Some(parent), Some(name)) = (
        path.parent(),
        path.file_name().and_then(|name| name.to_str()),
    ) else {
        return Ok(());
    };
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(conflict("无法检查发布路径大小写")),
    };
    for entry in entries {
        let entry = entry.map_err(|_| conflict("无法检查发布路径大小写"))?;
        if let Some(actual) = entry.file_name().to_str() {
            if actual.eq_ignore_ascii_case(name) && actual != name {
                return Err(conflict("发布路径存在大小写别名"));
            }
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn reject_case_alias(_: &Path) -> Result<(), AppError> {
    Ok(())
}

fn destination(root: &Path, app: &str, version: &str) -> Result<PathBuf, AppError> {
    validate_identity(app, version)?;
    reject_case_alias(&root.join(app))?;
    let target = root.join(app).join("versions").join(version);
    reject_case_alias(&target)?;
    check_no_symlinks(&target)?;
    Ok(target)
}

fn create_directory(path: &Path, private: bool) -> Result<(), AppError> {
    check_no_symlinks(path)?;
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let mut builder = builder;
    #[cfg(not(unix))]
    let _ = private;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(if private { 0o700 } else { 0o755 });
    }
    match builder.create(path) {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    path,
                    fs::Permissions::from_mode(if private { 0o700 } else { 0o755 }),
                )
                .map_err(|_| conflict("无法设置发布目录权限"))?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(conflict("无法创建发布目录")),
    }
    check_no_symlinks(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| conflict("无法读取发布目录"))?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(conflict("发布路径不是普通目录"));
    }
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(conflict("暂存目录必须仅供发布进程访问（0700）"));
        }
    }
    Ok(())
}

fn open_lock(root: &Path) -> Result<File, AppError> {
    let staging = root.join(STAGING_NAME);
    create_directory(&staging, true)?;
    let path = staging.join(LOCK_NAME);
    check_no_symlinks(&path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(path)
        .map_err(|_| conflict("无法打开暂存维护锁"))?;
    let metadata = file
        .metadata()
        .map_err(|_| conflict("无法检查暂存维护锁"))?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(conflict("暂存维护锁不是普通文件"));
    }
    Ok(file)
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StageOwner {
    schema_version: u32,
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    staging_id: Uuid,
    application: String,
    version: String,
}

struct Stage {
    directory: PathBuf,
    payload: PathBuf,
}
impl Drop for Stage {
    fn drop(&mut self) {
        // Only this random private container is owned. payload may already
        // have moved out; no cleanup path ever targets the installed version.
        if let Err(error) = fs::remove_dir_all(&self.directory) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(event = "release_staging_cleanup_failed");
            }
        }
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o644);
    }
    let mut file = options
        .open(path)
        .map_err(|_| conflict("无法写入暂存产物文件"))?;
    file.write_all(bytes)
        .map_err(|_| conflict("无法写入暂存产物文件"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o644))
            .map_err(|_| conflict("无法设置产物文件权限"))?;
    }
    file.sync_all()
        .map_err(|_| conflict("无法同步暂存产物文件"))?;
    Ok(())
}

fn create_stage(root: &Path, app: &str, version: &str) -> Result<Stage, AppError> {
    let id = crate::util::uuid7::new_uuid();
    let directory = root.join(STAGING_NAME).join(id.to_string());
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let mut builder = builder;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    // Unlike the shared parent directories, a stage must be newly owned.
    // A UUID collision must never adopt or clean somebody else's directory.
    builder
        .create(&directory)
        .map_err(|_| conflict("无法独占创建随机暂存目录"))?;
    let stage = Stage {
        payload: directory.join("payload"),
        directory,
    };
    let owner = StageOwner {
        schema_version: 1,
        staging_id: id,
        application: app.to_owned(),
        version: version.to_owned(),
    };
    let owner = serde_json::to_vec(&owner).map_err(|_| conflict("无法生成暂存所有权记录"))?;
    write_file(&stage.directory.join(OWNER_NAME), &owner)?;
    create_directory(&stage.payload, false)?;
    sync_directory(&stage.directory)?;
    sync_directory(&root.join(STAGING_NAME))?;
    Ok(stage)
}

#[cfg(target_os = "linux")]
fn atomic_move_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};
    renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(std::io::Error::from)
}

#[cfg(windows)]
fn atomic_move_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if from[..from.len() - 1].contains(&0) || to[..to.len() - 1].contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "NUL in path",
        ));
    }
    // SAFETY: both pointers refer to live, NUL-terminated UTF-16 buffers. Flags
    // zero forbid replacement and cross-volume copy; the OS performs one move.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn atomic_move_no_replace(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace installation is unsupported on this platform",
    ))
}

#[cfg(target_os = "linux")]
fn sync_directory(path: &Path) -> Result<(), AppError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| conflict("目录同步失败，已安装产物保留，请执行状态对账"))
}
#[cfg(not(target_os = "linux"))]
fn sync_directory(_: &Path) -> Result<(), AppError> {
    Ok(())
}

fn sync_tree(path: &Path) -> Result<(), AppError> {
    for entry in fs::read_dir(path).map_err(|_| conflict("无法读取暂存目录"))? {
        let entry = entry.map_err(|_| conflict("无法读取暂存目录"))?;
        if entry
            .file_type()
            .map_err(|_| conflict("无法读取暂存目录"))?
            .is_dir()
        {
            sync_tree(&entry.path())?;
        }
    }
    sync_directory(path)
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, AppError> {
    check_no_symlinks(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| conflict("已有产物文件缺失"))?;
    if !metadata.is_file() || is_link(&metadata) || metadata.len() > limit {
        return Err(conflict("已有产物文件类型或大小无效"));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|_| conflict("无法读取已有产物文件"))?;
    if bytes.len() as u64 > limit {
        return Err(conflict("已有产物文件大小无效"));
    }
    Ok(bytes)
}

fn list_files(root: &Path) -> Result<Vec<String>, AppError> {
    fn visit(
        root: &Path,
        current: &Path,
        files: &mut Vec<String>,
        count: &mut usize,
    ) -> Result<(), AppError> {
        for entry in fs::read_dir(current).map_err(|_| conflict("无法读取已有产物目录"))?
        {
            *count += 1;
            // A legal 500-byte path may imply up to 250 directories. Bound
            // traversal without rejecting trees allowed by the upload limits.
            if *count > MAX_FILES * 251 + 1 {
                return Err(conflict("已有产物目录条目过多"));
            }
            let entry = entry.map_err(|_| conflict("无法读取已有产物目录"))?;
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| conflict("无法读取已有产物目录"))?;
            if is_link(&metadata) {
                return Err(conflict("已有产物包含链接或重解析点"));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| conflict("已有产物路径无效"))?
                .to_str()
                .ok_or_else(|| conflict("已有产物路径不是 UTF-8"))?
                .replace('\\', "/");
            if relative.len() > 500 {
                return Err(conflict("已有产物路径过长"));
            }
            if metadata.is_dir() {
                visit(root, &path, files, count)?;
            } else if metadata.is_file() {
                if relative != MANIFEST_NAME {
                    files.push(relative);
                }
            } else {
                return Err(conflict("已有产物包含特殊文件"));
            }
        }
        Ok(())
    }
    let metadata = fs::symlink_metadata(root).map_err(|_| conflict("已有产物目录不可读"))?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(conflict("已有版本路径不是普通目录"));
    }
    let mut files = Vec::new();
    visit(root, root, &mut files, &mut 0)?;
    files.sort();
    Ok(files)
}

fn verify_contents(target: &Path, manifest: &ArtifactManifest) -> Result<(), AppError> {
    manifest.validate()?;
    let files = list_files(target)?;
    if files
        != manifest
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>()
    {
        return Err(conflict("已有产物实际文件清单不同"));
    }
    for file in &manifest.files {
        let bytes = read_bounded(
            &target.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
            MAX_FILE_BYTES as u64,
        )?;
        if bytes.len() as u64 != file.size || sha256(&bytes) != file.sha256 {
            return Err(conflict("已有产物实际文件摘要不同"));
        }
    }
    Ok(())
}

fn inspect_target(target: &Path) -> Result<Option<ArtifactManifest>, AppError> {
    check_no_symlinks(target)?;
    match fs::symlink_metadata(target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(conflict("无法读取已有版本目录")),
        Ok(metadata) if !metadata.is_dir() || is_link(&metadata) => {
            return Err(conflict("已有版本路径不是普通目录"))
        }
        Ok(_) => {}
    }
    let bytes = read_bounded(&target.join(MANIFEST_NAME), MAX_MANIFEST_BYTES)?;
    let manifest: ArtifactManifest =
        serde_json::from_slice(&bytes).map_err(|_| conflict("已有产物清单损坏"))?;
    verify_contents(target, &manifest)?;
    Ok(Some(manifest))
}

/// Inspect and hash every installed file; a missing path is distinct from corruption.
pub(crate) fn inspect_installed(
    root: &str,
    app: &str,
    version: &str,
) -> Result<Option<ArtifactManifest>, AppError> {
    let root = storage_root(root)?;
    inspect_target(&destination(&root, app, version)?)
}

/// Require the exact immutable artifact before reconciling a database record.
pub(crate) fn verify_installed(
    root: &str,
    app: &str,
    version: &str,
    expected_digest: &str,
) -> Result<ArtifactManifest, AppError> {
    let manifest =
        inspect_installed(root, app, version)?.ok_or_else(|| conflict("版本产物目录不存在"))?;
    if manifest.artifact_sha256 != expected_digest {
        return Err(conflict("版本产物摘要与登记不同"));
    }
    Ok(manifest)
}

fn accept_existing(
    target: &Path,
    prepared: &PreparedArchive,
    stage: &Stage,
) -> Result<(), AppError> {
    // The first spelling check happens before staging. Another request can
    // install a case-variant name while this one writes its private payload.
    reject_case_alias(target)?;
    check_no_symlinks(target)?;
    match fs::symlink_metadata(target.join(MANIFEST_NAME)) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Legacy adoption is exhaustive; release.json is not trusted or
            // silently excluded. Publish only a complete hidden manifest.
            verify_contents(target, &prepared.manifest)?;
            match fs::hard_link(
                stage.payload.join(MANIFEST_NAME),
                target.join(MANIFEST_NAME),
            ) {
                Ok(()) => sync_directory(target)?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(conflict("无法原子采用已有产物清单")),
            }
        }
        Err(_) => return Err(conflict("无法读取已有产物清单")),
    }
    let manifest = inspect_target(target)?.ok_or_else(|| conflict("已有产物目录不存在"))?;
    if manifest != prepared.manifest {
        return Err(conflict("版本目录已存在且摘要不同，请使用新版本号"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum WriteEvent {
    FileWritten,
    BeforeInstall,
    Installed,
}

fn write_with_hook(
    prepared: &PreparedArchive,
    root: &str,
    app: &str,
    version: &str,
    mut hook: impl FnMut(WriteEvent, &Path) -> Result<(), AppError>,
) -> Result<InstalledArchive, AppError> {
    let root = storage_root(root)?;
    let target = destination(&root, app, version)?;
    let lock = open_lock(&root)?;
    lock.lock_shared()
        .map_err(|_| conflict("无法取得暂存写入锁"))?;
    create_directory(&root.join(app), false)?;
    create_directory(&root.join(app).join("versions"), false)?;
    let stage = create_stage(&root, app, version)?;
    for (name, data) in &prepared.files {
        let path = stage
            .payload
            .join(name.replace('/', std::path::MAIN_SEPARATOR_STR));
        let mut directories = Vec::new();
        let mut parent = path.parent();
        while let Some(current) = parent {
            if current == stage.payload {
                break;
            }
            directories.push(current);
            parent = current.parent();
        }
        for directory in directories.into_iter().rev() {
            create_directory(directory, false)?;
        }
        write_file(&path, data)?;
        hook(WriteEvent::FileWritten, &stage.payload)?;
    }
    let manifest =
        serde_json::to_vec(&prepared.manifest).map_err(|_| conflict("无法生成产物清单"))?;
    write_file(&stage.payload.join(MANIFEST_NAME), &manifest)?;
    if prepared.artifact_sha256 != prepared.manifest.artifact_sha256 {
        return Err(conflict("待安装产物摘要不一致"));
    }
    verify_contents(&stage.payload, &prepared.manifest)?;
    sync_tree(&stage.payload)?;
    hook(WriteEvent::BeforeInstall, &stage.payload)?;
    check_no_symlinks(&target)?;
    let created = match atomic_move_no_replace(&stage.payload, &target) {
        Ok(()) => {
            // Any later failure keeps final intact. A caller must reconcile;
            // it cannot infer an uncommitted artifact from a failed response.
            reject_case_alias(&target)?;
            hook(WriteEvent::Installed, &target)?;
            sync_directory(
                target
                    .parent()
                    .ok_or_else(|| conflict("版本目录无父目录"))?,
            )?;
            true
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists
                || fs::symlink_metadata(&target).is_ok() =>
        {
            accept_existing(&target, prepared, &stage)?;
            false
        }
        Err(_) => {
            return Err(conflict(
                "文件系统不支持同文件系统无覆盖原子安装，未发布产物",
            ))
        }
    };
    drop(stage);
    drop(lock);
    Ok(InstalledArchive {
        #[cfg(test)]
        path: target,
        created,
        artifact_sha256: prepared.artifact_sha256.clone(),
    })
}

/// Publish a complete same-filesystem payload with OS no-replace semantics.
/// Installed versions are never removed on a later filesystem/database error.
pub(crate) fn write(
    prepared: &PreparedArchive,
    root: &str,
    app: &str,
    version: &str,
) -> Result<InstalledArchive, AppError> {
    if !cfg!(any(target_os = "linux", windows)) {
        return Err(conflict("当前平台不支持无覆盖原子安装"));
    }
    write_with_hook(prepared, root, app, version, |_, _| Ok(()))
}

fn known_stage(path: &Path, id: Uuid) -> Result<(), AppError> {
    check_no_symlinks(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| conflict("暂存目录不可读"))?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(conflict("不是普通暂存目录"));
    }
    let owner: StageOwner = serde_json::from_slice(&read_bounded(&path.join(OWNER_NAME), 4096)?)
        .map_err(|_| conflict("所有权记录无效"))?;
    if owner.schema_version != 1 || owner.staging_id != id {
        return Err(conflict("所有权记录不匹配"));
    }
    validate_identity(&owner.application, &owner.version)?;
    for entry in fs::read_dir(path).map_err(|_| conflict("暂存目录不可读"))? {
        let entry = entry.map_err(|_| conflict("暂存目录不可读"))?;
        if entry.file_name() == OWNER_NAME {
            continue;
        }
        if entry.file_name() != "payload" {
            return Err(conflict("暂存目录含未知内容"));
        }
        // Bounded traversal also rejects links and special files. Payload can
        // be incomplete after a crash; its bytes need not match a manifest.
        list_files(&entry.path())?;
    }
    Ok(())
}

/// Offline orphan maintenance. Scan is the default; explicit apply removes
/// only UUID stages with a matching ownership record while no writer is active.
pub(crate) fn maintain_staging(root: &str, apply: bool) -> Result<Value, AppError> {
    let root = storage_root(root)?;
    let lock = open_lock(&root)?;
    lock.try_lock()
        .map_err(|_| conflict("发布写入或其他维护仍在进行，暂存维护已拒绝"))?;
    let staging = root.join(STAGING_NAME);
    let mut eligible = Vec::new();
    let mut removed = Vec::new();
    let mut retained = Vec::new();
    for entry in fs::read_dir(&staging).map_err(|_| conflict("无法扫描暂存目录"))? {
        let entry = entry.map_err(|_| conflict("无法扫描暂存目录"))?;
        if entry.file_name() == LOCK_NAME {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let recognized = crate::util::uuid7::parse_uuid(&name)
            .ok()
            .filter(|id| id.get_version_num() == 7 && id.to_string() == name);
        if let Some(id) = recognized {
            if known_stage(&entry.path(), id).is_ok() {
                eligible.push(name.clone());
                if apply {
                    // Recheck immediately before removal; the persistent lock
                    // excludes all cooperative writers and maintenance runs.
                    known_stage(&entry.path(), id)?;
                    fs::remove_dir_all(entry.path())
                        .map_err(|_| conflict("暂存清理失败，未触碰已安装版本"))?;
                    removed.push(name);
                }
                continue;
            }
        }
        retained.push(json!({ "name": name, "reason": "UNRECOGNIZED_OR_UNSAFE" }));
    }
    eligible.sort();
    removed.sort();
    retained.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    sync_directory(&staging)?;
    Ok(
        json!({ "schemaVersion": 1, "mode": if apply { "APPLY" } else { "SCAN" }, "eligible": eligible, "removed": removed, "retained": retained, "installedVersionsUntouched": true }),
    )
}

#[cfg(test)]
mod tests;
