use super::*;
use std::io::Cursor;
use std::process::{Child, Command};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

struct Storage(PathBuf);
impl Storage {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "saas-atomic-install-{}",
            crate::util::uuid7::new_uuid()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn as_str(&self) -> &str {
        self.0.to_str().unwrap()
    }
}
impl Drop for Storage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn archive(bytes: &[u8]) -> PreparedArchive {
    let mut files = vec![
        ("assets/app.js".to_owned(), bytes.to_vec()),
        ("index.html".to_owned(), b"<html>complete</html>".to_vec()),
    ];
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let manifest = ArtifactManifest::from_files("HTML", "index.html", &files).unwrap();
    PreparedArchive {
        files,
        entry: "index.html".to_owned(),
        resources: vec![],
        artifact_sha256: manifest.artifact_sha256.clone(),
        manifest,
    }
}

#[test]
fn canonical_digest_and_resources_ignore_archive_order_format_and_wrapper() {
    let files: &[(&str, &[u8])] = &[
        ("dist/index.html", b"<html></html>"),
        ("dist/z.js", b"last"),
        ("dist/a b.js", b"first"),
    ];
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    let zip = super::super::prepare(&zip.finish().unwrap().into_inner(), "zip", "HTML").unwrap();
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    for (name, bytes) in files.iter().rev() {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, name.trim_start_matches("dist/"), *bytes)
            .unwrap();
    }
    let bytes = tar.into_inner().unwrap().finish().unwrap();
    let tar = super::super::prepare(&bytes, "tar.gz", "HTML").unwrap();
    assert_eq!(zip.artifact_sha256, tar.artifact_sha256);
    assert_eq!(zip.manifest, tar.manifest);
    assert_eq!(
        serde_json::to_value(zip.resources).unwrap(),
        serde_json::to_value(tar.resources).unwrap()
    );
    assert_eq!(zip.manifest.files[0].path, "a b.js");
    let mut esm = zip.manifest.clone();
    esm.entry_type = "ESM".to_owned();
    assert_ne!(esm.digest().unwrap(), zip.artifact_sha256);
}

#[test]
fn tar_extension_metadata_is_bounded_before_internal_longname_or_pax_allocation() {
    for kind in *b"Lx" {
        let mut header = tar::Header::new_gnu();
        header.set_path("././@LongLink").unwrap();
        header.set_entry_type(tar::EntryType::new(kind));
        let size = super::super::MAX_TAR_STREAM_BYTES + 8192;
        header.set_size(size as u64);
        header.set_mode(0o644);
        header.set_cksum();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(header.as_bytes()).unwrap();
        let block = [b'x'; 8192];
        let mut remaining = size;
        while remaining > 0 {
            let written = remaining.min(block.len());
            gzip.write_all(&block[..written]).unwrap();
            remaining -= written;
        }
        gzip.write_all(&[0; 1024]).unwrap();
        let compressed = gzip.finish().unwrap();
        assert!(
            compressed.len() < 64 * 1024,
            "fixture must exercise a compressed metadata bomb"
        );
        let error = super::super::prepare(&compressed, "tar.gz", "HTML")
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("解压流超过 17 MiB"),
            "kind={kind}: {error}"
        );
    }
}

#[test]
fn rejects_reserved_metadata_case_aliases_and_windows_path_aliases() {
    for name in [
        ".ARTIFACT-MANIFEST.JSON",
        "nested/.artifact-manifest.json",
        "file.",
        "file ",
        "COM9.txt",
        "LPT8",
        "con.txt",
    ] {
        assert!(safe_path(name).is_err(), "{name}");
    }
    for entries in [
        vec![("index.html", b"ok".as_slice()), ("RELEASE.JSON", b"bad")],
        vec![
            ("index.html", b"ok"),
            ("assets", b"a"),
            ("assets/app.js", b"b"),
        ],
        vec![("index.html", b"ok"), ("A.js", b"a"), ("a.js", b"b")],
    ] {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        assert!(super::super::prepare(&zip.finish().unwrap().into_inner(), "zip", "HTML").is_err());
    }
}

#[test]
fn partial_write_error_never_exposes_a_version_and_cleans_only_the_owned_stage() {
    let root = Storage::new();
    let prepared = archive(b"first");
    let target = root.0.join("sample/versions/1.0.0");
    let error = write_with_hook(&prepared, root.as_str(), "sample", "1.0.0", |event, _| {
        assert!(!target.exists());
        if matches!(event, WriteEvent::FileWritten) {
            return Err(conflict("injected disk write failure"));
        }
        Ok(())
    });
    assert!(error.is_err());
    assert!(!target.exists());
    assert_eq!(
        fs::read_dir(root.0.join(STAGING_NAME)).unwrap().count(),
        1,
        "only persistent lock may remain"
    );
}

#[test]
fn failure_after_atomic_install_keeps_complete_final_for_reconciliation() {
    let root = Storage::new();
    let prepared = archive(b"complete");
    assert!(
        write_with_hook(&prepared, root.as_str(), "sample", "1.0.0", |event, _| {
            if matches!(event, WriteEvent::Installed) {
                Err(conflict("injected post-install sync failure"))
            } else {
                Ok(())
            }
        })
        .is_err()
    );
    let installed =
        verify_installed(root.as_str(), "sample", "1.0.0", &prepared.artifact_sha256).unwrap();
    assert_eq!(installed, prepared.manifest);
    assert!(
        !write(&prepared, root.as_str(), "sample", "1.0.0")
            .unwrap()
            .created
    );
}

#[test]
fn concurrent_same_digest_install_is_idempotent_and_different_digests_never_mix() {
    for identical in [true, false] {
        let root = Storage::new();
        let prepared = Arc::new([archive(b"first-content"), archive(b"second-content")]);
        let barrier = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|number| {
                let (prepared, barrier, root) = (prepared.clone(), barrier.clone(), root.0.clone());
                thread::spawn(move || {
                    let archive = &prepared[if identical { 0 } else { number % 2 }];
                    write_with_hook(
                        archive,
                        root.to_str().unwrap(),
                        "sample",
                        "1.0.0",
                        |event, _| {
                            if matches!(event, WriteEvent::BeforeInstall) {
                                barrier.wait();
                            }
                            Ok(())
                        },
                    )
                })
            })
            .collect();
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(
            results
                .iter()
                .filter(|result| result.as_ref().is_ok_and(|installed| installed.created))
                .count(),
            1
        );
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            if identical { 8 } else { 4 }
        );
        let installed = inspect_installed(root.as_str(), "sample", "1.0.0")
            .unwrap()
            .unwrap();
        assert!(prepared.iter().any(|archive| archive.manifest == installed));
        assert_eq!(fs::read_dir(root.0.join(STAGING_NAME)).unwrap().count(), 1);
    }
}

#[test]
fn legacy_adoption_requires_exact_actual_bytes_and_never_trusts_release_json() {
    let root = Storage::new();
    let prepared = archive(b"legacy");
    let path = root.0.join("sample/versions/1.0.0");
    fs::create_dir_all(path.join("assets")).unwrap();
    for (name, bytes) in &prepared.files {
        fs::write(path.join(name), bytes).unwrap();
    }
    fs::write(path.join("release.json"), b"{\"trusted\":true}").unwrap();
    assert!(write(&prepared, root.as_str(), "sample", "1.0.0").is_err());
    fs::remove_file(path.join("release.json")).unwrap();
    assert!(
        !write(&prepared, root.as_str(), "sample", "1.0.0")
            .unwrap()
            .created
    );
    assert_eq!(
        inspect_installed(root.as_str(), "sample", "1.0.0")
            .unwrap()
            .unwrap(),
        prepared.manifest
    );
    fs::write(path.join("assets/app.js"), b"corrupted").unwrap();
    assert!(inspect_installed(root.as_str(), "sample", "1.0.0").is_err());
    assert!(write(&prepared, root.as_str(), "sample", "1.0.0").is_err());
}

#[test]
fn primitive_no_replace_preserves_existing_complete_empty_and_file_targets() {
    let root = Storage::new();
    let source = root.0.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("fresh"), b"new").unwrap();
    let complete = root.0.join("complete");
    fs::create_dir(&complete).unwrap();
    fs::write(complete.join("old"), b"old").unwrap();
    let empty = root.0.join("empty");
    fs::create_dir(&empty).unwrap();
    let file = root.0.join("file");
    fs::write(&file, b"original-file").unwrap();
    for target in [&complete, &empty, &file] {
        assert!(atomic_move_no_replace(&source, target).is_err());
        assert_eq!(fs::read(source.join("fresh")).unwrap(), b"new");
    }
    assert_eq!(fs::read(complete.join("old")).unwrap(), b"old");
    assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);
    assert_eq!(fs::read(&file).unwrap(), b"original-file");
    assert!(atomic_move_no_replace(&source, &root.0.join("installed")).is_ok());
    assert!(!source.exists());
}

#[cfg(unix)]
#[test]
fn symlinks_are_neither_overwritten_followed_nor_cleaned_as_owned_staging() {
    use std::os::unix::fs::symlink;
    let root = Storage::new();
    let outside = Storage::new();
    fs::write(outside.0.join("keep"), b"outside").unwrap();
    let prepared = archive(b"safe");
    let source = root.0.join("source");
    fs::create_dir(&source).unwrap();
    let link = root.0.join("link");
    symlink(&outside.0, &link).unwrap();
    assert!(atomic_move_no_replace(&source, &link).is_err());
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    create_directory(&root.0.join(STAGING_NAME), true).unwrap();
    let stage_link = root
        .0
        .join(STAGING_NAME)
        .join(crate::util::uuid7::new_uuid().to_string());
    symlink(&outside.0, &stage_link).unwrap();
    let report = maintain_staging(root.as_str(), true).unwrap();
    assert_eq!(report["retained"].as_array().unwrap().len(), 1);
    assert!(stage_link
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    fs::create_dir_all(root.0.join("sample/versions")).unwrap();
    symlink(&outside.0, root.0.join("sample/versions/1.0.0")).unwrap();
    assert!(write(&prepared, root.as_str(), "sample", "1.0.0").is_err());
    assert!(inspect_installed(root.as_str(), "sample", "1.0.0").is_err());
    assert_eq!(fs::read(outside.0.join("keep")).unwrap(), b"outside");
}

#[cfg(target_os = "linux")]
#[test]
fn cross_filesystem_move_fails_closed_without_copying() {
    let root = Storage::new();
    let destination = Path::new("/dev/shm").join(format!(
        "saas-no-replace-{}",
        crate::util::uuid7::new_uuid()
    ));
    let source = root.0.join("payload");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("bytes"), b"immutable").unwrap();
    let error = atomic_move_no_replace(&source, &destination).unwrap_err();
    assert_eq!(
        error.raw_os_error(),
        Some(18),
        "Linux must return EXDEV; verification needs /dev/shm on a distinct filesystem"
    );
    assert!(!destination.exists());
    assert_eq!(fs::read(source.join("bytes")).unwrap(), b"immutable");
}

#[test]
fn maintenance_requires_no_active_writer_and_retains_unknown_stages_and_final() {
    let root = Storage::new();
    let prepared = archive(b"installed");
    let installed = write(&prepared, root.as_str(), "sample", "1.0.0").unwrap();
    let lock = open_lock(&root.0).unwrap();
    lock.lock_shared().unwrap();
    assert!(maintain_staging(root.as_str(), false).is_err());
    drop(lock);
    let stage = create_stage(&root.0, "sample", "interrupted").unwrap();
    let stage_id = stage
        .directory
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(crate::util::uuid7::is_uuid_v7(
        Uuid::parse_str(&stage_id).unwrap()
    ));
    std::mem::forget(stage);
    let unknown = root
        .0
        .join(STAGING_NAME)
        .join(crate::util::uuid7::new_uuid().to_string());
    fs::create_dir(&unknown).unwrap();
    fs::write(unknown.join("keep"), b"unowned").unwrap();
    // Intentional legacy v4 fixture: maintenance cannot adopt a pre-cutover
    // stage merely because its directory and historical owner record agree.
    let legacy_id = Uuid::new_v4();
    let legacy = root.0.join(STAGING_NAME).join(legacy_id.to_string());
    fs::create_dir_all(legacy.join("payload")).unwrap();
    fs::write(
        legacy.join(OWNER_NAME),
        serde_json::to_vec(&StageOwner {
            schema_version: 1,
            staging_id: legacy_id,
            application: "sample".into(),
            version: "legacy".into(),
        })
        .unwrap(),
    )
    .unwrap();
    let scan = maintain_staging(root.as_str(), false).unwrap();
    assert_eq!(scan["eligible"], json!([stage_id]));
    assert!(root.0.join(STAGING_NAME).join(&stage_id).exists());
    assert_eq!(scan["removed"], json!([]));
    let applied = maintain_staging(root.as_str(), true).unwrap();
    assert_eq!(applied["removed"], json!([stage_id]));
    assert!(unknown.join("keep").is_file());
    assert!(legacy.join(OWNER_NAME).is_file());
    assert!(installed.path.join("index.html").is_file());
    assert!(root.0.join(STAGING_NAME).join(LOCK_NAME).is_file());
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn process_interruption_leaves_only_owned_private_stage() {
    const CHILD_ROOT: &str = "SAAS_ATOMIC_INSTALL_CRASH_TEST_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let _ = write_with_hook(
            &archive(b"partial"),
            root.to_str().unwrap(),
            "sample",
            "1.0.0",
            |event, _| {
                if matches!(event, WriteEvent::FileWritten) {
                    fs::write(root.join("writer-ready"), b"ready").unwrap();
                    loop {
                        thread::park();
                    }
                }
                Ok(())
            },
        );
        panic!("fixture must be killed after its first write");
    }
    let root = Storage::new();
    let mut child = OwnedChild(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "release_archives::installation::tests::process_interruption_leaves_only_owned_private_stage", "--test-threads=1"])
        .env(CHILD_ROOT, &root.0).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.0.join("writer-ready").exists() {
        assert!(
            Instant::now() < deadline,
            "owned writer did not reach the first synced file"
        );
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "owned writer exited before its interruption point"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        maintain_staging(root.as_str(), false).is_err(),
        "live writer owns its shared file lock"
    );
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    assert!(!root.0.join("sample/versions/1.0.0").exists());
    let scan = maintain_staging(root.as_str(), false).unwrap();
    assert_eq!(scan["eligible"].as_array().unwrap().len(), 1);
    assert_eq!(
        maintain_staging(root.as_str(), true).unwrap()["removed"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        inspect_installed(root.as_str(), "sample", "1.0.0").unwrap(),
        None
    );
}

#[test]
fn identities_are_validated_without_creating_external_paths() {
    let root = Storage::new();
    let prepared = archive(b"safe");
    for (app, version) in [
        ("../outside", "1"),
        ("sample", "../../outside"),
        ("Sample", "1"),
        ("sample", "CON"),
        ("sample", "1."),
    ] {
        assert!(write(&prepared, root.as_str(), app, version).is_err());
        assert!(inspect_installed(root.as_str(), app, version).is_err());
    }
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn windows_existing_version_case_alias_is_rejected() {
    let root = Storage::new();
    let prepared = archive(b"safe");
    write(&prepared, root.as_str(), "sample", "Release-A").unwrap();
    assert!(write(&prepared, root.as_str(), "sample", "release-a").is_err());
    assert!(inspect_installed(root.as_str(), "sample", "release-a").is_err());
}

#[cfg(windows)]
#[test]
fn concurrent_windows_version_case_aliases_cannot_both_install_or_register() {
    let root = Storage::new();
    let prepared = Arc::new(archive(b"same-artifact"));
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = ["Release-A", "release-a"]
        .into_iter()
        .map(|version| {
            let (root, prepared, barrier) = (root.0.clone(), prepared.clone(), barrier.clone());
            thread::spawn(move || {
                write_with_hook(
                    &prepared,
                    root.to_str().unwrap(),
                    "sample",
                    version,
                    |event, _| {
                        if matches!(event, WriteEvent::BeforeInstall) {
                            barrier.wait();
                        }
                        Ok(())
                    },
                )
            })
        })
        .collect();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| result.as_ref().is_ok_and(|installed| installed.created))
            .count(),
        1
    );
    assert_eq!(
        fs::read_dir(root.0.join("sample/versions"))
            .unwrap()
            .count(),
        1
    );
}

#[cfg(windows)]
#[test]
fn windows_junctions_are_not_followed_overwritten_or_removed_by_maintenance() {
    use std::os::windows::process::CommandExt;
    let root = Storage::new();
    let outside = Storage::new();
    fs::write(outside.0.join("keep"), b"outside").unwrap();
    let helper = root.0.join("junction.ps1");
    fs::write(&helper, "param([string]$LinkPath, [string]$TargetPath)\nNew-Item -ItemType Junction -Path $LinkPath -Target $TargetPath -ErrorAction Stop | Out-Null\n").unwrap();
    let junction = |path: &Path| {
        let status = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&helper)
            .arg("-LinkPath")
            .arg(path)
            .arg("-TargetPath")
            .arg(&outside.0)
            .creation_flags(0x0800_0000)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "native Windows junction fixture must be created"
        );
    };
    let link = root.0.join("junction");
    junction(&link);
    assert!(is_link(&fs::symlink_metadata(&link).unwrap()));
    let source = root.0.join("source");
    fs::create_dir(&source).unwrap();
    assert!(atomic_move_no_replace(&source, &link).is_err());
    assert!(is_link(&fs::symlink_metadata(&link).unwrap()));
    create_directory(&root.0.join(STAGING_NAME), true).unwrap();
    let stage = root
        .0
        .join(STAGING_NAME)
        .join(crate::util::uuid7::new_uuid().to_string());
    junction(&stage);
    assert_eq!(
        maintain_staging(root.as_str(), true).unwrap()["retained"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(is_link(&fs::symlink_metadata(&stage).unwrap()));
    fs::create_dir_all(root.0.join("sample/versions")).unwrap();
    junction(&root.0.join("sample/versions/1.0.0"));
    assert!(write(&archive(b"safe"), root.as_str(), "sample", "1.0.0").is_err());
    assert!(inspect_installed(root.as_str(), "sample", "1.0.0").is_err());
    assert_eq!(fs::read(outside.0.join("keep")).unwrap(), b"outside");
}
