//! Bounded, data-only extraction for browser micro-application releases.
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

use crate::error::AppError;
use crate::releases::Resource;

pub const MAX_ARCHIVE_BYTES: usize = 20 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_FILES: usize = 49; // one entry and at most 48 manifest resources
const PATH_SEGMENT_ENCODE: &AsciiSet = &CONTROLS.add(b' ').add(b'%').add(b'#').add(b'?')
    .add(b'/').add(b'"').add(b'\'').add(b'<').add(b'>').add(b'`')
    .add(b'{').add(b'}').add(b'|').add(b'^').add(b'[').add(b']');

pub struct PreparedArchive {
    files: Vec<(String, Vec<u8>)>,
    pub entry: String,
    pub resources: Vec<Resource>,
}

fn bad(message: &'static str) -> AppError {
    AppError::bad_request(message)
}

fn safe_path(raw: &str) -> Result<String, AppError> {
    let raw = raw.strip_prefix("./").unwrap_or(raw).trim_end_matches('/');
    if raw.is_empty() || raw.starts_with('/') || raw.contains('\\') || raw.contains(':')
        || raw.contains('?') || raw.contains('#') || raw.chars().any(char::is_control)
        || raw.len() > 500
    {
        return Err(bad("压缩包包含不安全路径"));
    }
    for component in raw.split('/') {
        if component.is_empty() || component == "." || component == ".." || component.len() > 120
            || component.to_ascii_lowercase().starts_with(".env")
            || [".git", ".npmrc", ".yarnrc", ".yarnrc.yml"].contains(&component.to_ascii_lowercase().as_str())
        {
            return Err(bad("压缩包包含路径穿越或敏感文件"));
        }
        let stem = component.split('.').next().unwrap_or("").to_ascii_uppercase();
        if ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3"].contains(&stem.as_str()) {
            return Err(bad("压缩包包含系统保留文件名"));
        }
    }
    Ok(raw.to_owned())
}

fn add_file(files: &mut Vec<(String, Vec<u8>)>, seen: &mut HashSet<String>,
    total: &mut usize, name: &str, reader: &mut impl Read) -> Result<(), AppError> {
    let name = safe_path(name)?;
    if !seen.insert(name.to_ascii_lowercase()) { return Err(bad("压缩包包含重复文件路径")); }
    if files.len() >= MAX_FILES { return Err(bad("压缩包文件数超过 49 个")); }
    let mut data = Vec::new();
    reader.take((MAX_FILE_BYTES + 1) as u64).read_to_end(&mut data)
        .map_err(|_| bad("压缩包文件读取失败"))?;
    if data.is_empty() || data.len() > MAX_FILE_BYTES { return Err(bad("压缩包文件为空或超过 4 MiB")); }
    *total += data.len();
    if *total > MAX_TOTAL_BYTES { return Err(bad("解压后总大小超过 16 MiB")); }
    files.push((name, data));
    Ok(())
}

fn encode_path(path: &str) -> String {
    path.split('/').map(|part| utf8_percent_encode(part, PATH_SEGMENT_ENCODE).to_string())
        .collect::<Vec<_>>().join("/")
}

pub fn prepare(bytes: &[u8], format: &str, entry_type: &str) -> Result<PreparedArchive, AppError> {
    if bytes.is_empty() || bytes.len() > MAX_ARCHIVE_BYTES { return Err(bad("压缩包不能为空或超过 20 MiB")); }
    let entry = match entry_type { "HTML" => "index.html", "ESM" => "entry.mjs", _ => return Err(bad("入口类型仅支持 HTML / ESM")) };
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut total = 0;
    match format {
        "zip" => {
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| bad("ZIP 文件格式错误"))?;
            if archive.len() > 128 { return Err(bad("压缩包条目过多")); }
            for index in 0..archive.len() {
                let mut file = archive.by_index(index).map_err(|_| bad("ZIP 条目读取失败"))?;
                let name = file.name().to_owned();
                safe_path(&name)?;
                let mode = file.unix_mode().unwrap_or(0);
                let kind = mode & 0o170000;
                if kind != 0 && kind != 0o100000 && kind != 0o040000 { return Err(bad("不接受链接或特殊文件")); }
                if file.is_dir() { continue; }
                add_file(&mut files, &mut seen, &mut total, &name, &mut file)?;
            }
        }
        "tar.gz" => {
            let decoder = GzDecoder::new(bytes);
            let mut archive = tar::Archive::new(decoder);
            let entries = archive.entries().map_err(|_| bad("tar.gz 文件格式错误"))?;
            let mut count = 0;
            for item in entries {
                count += 1;
                if count > 128 { return Err(bad("压缩包条目过多")); }
                let mut file = item.map_err(|_| bad("tar.gz 条目读取失败"))?;
                let path = file.path().map_err(|_| bad("tar.gz 路径错误"))?;
                let name = path.to_str().ok_or_else(|| bad("tar.gz 路径必须是 UTF-8"))?.to_owned();
                if file.header().entry_type().is_dir() && (name == "." || name == "./") { continue; }
                safe_path(&name)?;
                if file.header().entry_type().is_dir() { continue; }
                if !file.header().entry_type().is_file() { return Err(bad("不接受链接或特殊文件")); }
                add_file(&mut files, &mut seen, &mut total, &name, &mut file)?;
            }
        }
        _ => return Err(bad("仅支持 ZIP 或 tar.gz 压缩包")),
    }
    // Common build tools may wrap dist in a single top-level directory.
    if !files.iter().any(|(name, _)| name == entry) {
        let prefix = files.first().and_then(|(name, _)| name.split_once('/').map(|(first, _)| first.to_owned()))
            .ok_or_else(|| bad("压缩包根目录缺少入口文件"))?;
        if !files.iter().all(|(name, _)| name.starts_with(&format!("{prefix}/"))) {
            return Err(bad("压缩包根目录缺少入口文件"));
        }
        for (name, _) in &mut files { *name = name[prefix.len() + 1..].to_owned(); }
    }
    if !files.iter().any(|(name, _)| name == entry) { return Err(bad("压缩包根目录缺少入口文件")); }
    if files.iter().any(|(name, _)| {
        let lower = name.to_ascii_lowercase();
        name == "release.json" || name != entry && (lower.ends_with(".html") || lower.ends_with(".htm"))
    }) {
        return Err(bad("release.json 为保留文件；一个版本仅支持一个 HTML 入口"));
    }
    let resources = files.iter().filter(|(name, _)| name != entry).map(|(name, _)| {
        let kind = match Path::new(name).extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
            "js" | "mjs" | "cjs" => "SCRIPT", "css" => "STYLE", _ => "ASSET",
        };
        Resource { url: encode_path(name), kind: kind.to_owned() }
    }).collect();
    Ok(PreparedArchive { files, entry: entry.to_owned(), resources })
}

fn check_no_symlinks(path: &Path) -> Result<(), AppError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        if let Component::Normal(_) | Component::Prefix(_) | Component::RootDir = component {
            current.push(component.as_os_str());
            if let Ok(meta) = fs::symlink_metadata(&current) {
                if meta.file_type().is_symlink() { return Err(bad("发布目录不能包含符号链接")); }
            }
        }
    }
    Ok(())
}

fn matches_existing(destination: &Path, prepared: &PreparedArchive) -> Result<bool, AppError> {
    let mut existing = Vec::new();
    fn visit(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), AppError> {
        for item in fs::read_dir(dir).map_err(|_| bad("无法读取已有版本目录"))? {
            let item = item.map_err(|_| bad("无法读取已有版本目录"))?;
            let path = item.path();
            let kind = item.file_type().map_err(|_| bad("无法读取已有版本目录"))?;
            if kind.is_symlink() { return Err(bad("已有版本目录包含符号链接")); }
            if kind.is_dir() { visit(root, &path, files)?; }
            else if kind.is_file() {
                let name = path.strip_prefix(root).map_err(|_| bad("已有版本路径错误"))?
                    .to_string_lossy().replace('\\', "/");
                if name != "release.json" { files.push(name); }
            } else { return Err(bad("已有版本包含特殊文件")); }
        }
        Ok(())
    }
    visit(destination, destination, &mut existing)?;
    if existing.len() != prepared.files.len() { return Ok(false); }
    for (name, data) in &prepared.files {
        if !existing.contains(name) || fs::read(destination.join(name.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .map_err(|_| bad("无法读取已有版本文件"))? != *data { return Ok(false); }
    }
    Ok(true)
}

pub fn write(prepared: &PreparedArchive, root: &str, app: &str, version: &str) -> Result<(PathBuf, bool), AppError> {
    if root.is_empty() { return Err(AppError::conflict("服务器未配置 MICRO_APPS_DIR，无法接收压缩包")); }
    let root = Path::new(root);
    if !root.is_absolute() || !root.is_dir() { return Err(AppError::conflict("MICRO_APPS_DIR 必须是已存在的绝对目录")); }
    check_no_symlinks(root)?;
    let parent = root.join(app).join("versions");
    check_no_symlinks(&parent)?;
    fs::create_dir_all(&parent).map_err(|_| AppError::conflict("发布目录不可写"))?;
    check_no_symlinks(&parent)?;
    let destination = parent.join(version);
    match fs::create_dir(&destination) {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            check_no_symlinks(&destination)?;
            if matches_existing(&destination, prepared)? { return Ok((destination, false)); }
            return Err(AppError::conflict("版本目录已存在且文件不同，请使用新版本号"));
        }
        Err(_) => return Err(AppError::conflict("无法创建版本目录")),
    }
    let result = (|| -> Result<(), AppError> {
        for (name, data) in &prepared.files {
            let target = destination.join(name.replace('/', std::path::MAIN_SEPARATOR_STR));
            if let Some(parent) = target.parent() { fs::create_dir_all(parent).map_err(|_| bad("无法创建资源目录"))?; }
            let mut file = OpenOptions::new().write(true).create_new(true).open(&target)
                .map_err(|_| bad("无法写入资源文件"))?;
            file.write_all(data).map_err(|_| bad("无法写入资源文件"))?;
        }
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_dir_all(&destination); }
    result?;
    Ok((destination, true))
}

pub fn remove_uploaded(path: &Path) { let _ = fs::remove_dir_all(path); }

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_files(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn tar_gz_files(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut writer = tar::Builder::new(encoder);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            writer.append_data(&mut header, *name, *data).unwrap();
        }
        writer.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn rejects_unsafe_archive_paths() {
        for path in ["../index.html", "/index.html", "dist/../../x", "dist/.env", "dist/a\\b", "CON.txt"] {
            assert!(safe_path(path).is_err(), "{path}");
        }
    }

    #[test]
    fn zip_and_tar_gz_prepare_the_same_version_and_reject_different_bytes() {
        let files: &[(&str, &[u8])] = &[("dist/index.html", b"<html></html>"), ("dist/assets/app.js", b"console.log(1)")];
        let zip = prepare(&zip_files(files), "zip", "HTML").unwrap();
        let tar = prepare(&tar_gz_files(files), "tar.gz", "HTML").unwrap();
        assert_eq!(zip.entry, "index.html");
        assert_eq!(zip.resources[0].url, "assets/app.js");
        let root = std::env::temp_dir().join(format!("micro-archive-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root_str = root.to_str().unwrap();
        let (first, created) = write(&zip, root_str, "sample", "1.0.0").unwrap();
        assert!(created);
        assert_eq!(fs::read(first.join("index.html")).unwrap(), b"<html></html>");
        assert!(!write(&tar, root_str, "sample", "1.0.0").unwrap().1);
        let changed = prepare(&zip_files(&[("index.html", b"changed")]), "zip", "HTML").unwrap();
        assert!(write(&changed, root_str, "sample", "1.0.0").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_traversal_and_missing_entry() {
        assert!(prepare(&zip_files(&[("../index.html", b"bad")]), "zip", "HTML").is_err());
        assert!(prepare(&zip_files(&[("app.js", b"ok")]), "zip", "HTML").is_err());
        let oversized = vec![b'x'; MAX_FILE_BYTES + 1];
        assert!(prepare(&zip_files(&[("index.html", &oversized)]), "zip", "HTML").is_err());
    }
}
