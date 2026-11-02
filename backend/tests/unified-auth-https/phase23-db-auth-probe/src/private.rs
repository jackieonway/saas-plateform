//! Linux directory-fd anchored, single-component private file access.
use crate::LocalResult;
use std::{
    ffi::CString,
    fs::File,
    io::Read,
    os::fd::{AsRawFd, FromRawFd},
    path::{Path, PathBuf},
};

pub(super) struct OwnedTask {
    root: PathBuf,
    directory: File,
}

fn metadata(file: &File) -> LocalResult<libc::stat> {
    let mut value = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fstat writes the complete initialized stat structure on success.
    if unsafe { libc::fstat(file.as_raw_fd(), value.as_mut_ptr()) } != 0 {
        return Err("DB_AUTH_PROBE_FILE_REJECTED");
    }
    Ok(unsafe { value.assume_init() })
}

impl OwnedTask {
    pub(super) fn open(root: &Path, owner: &[u8]) -> LocalResult<Self> {
        let raw = CString::new(root.as_os_str().as_encoded_bytes())
            .map_err(|_| "DB_AUTH_PROBE_TASK_REJECTED")?;
        // SAFETY: raw is NUL terminated and the result is immediately owned.
        let descriptor = unsafe {
            libc::open(
                raw.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err("DB_AUTH_PROBE_TASK_REJECTED");
        }
        let directory = unsafe { File::from_raw_fd(descriptor) };
        let attributes = metadata(&directory)?;
        if attributes.st_mode & libc::S_IFMT != libc::S_IFDIR
            || attributes.st_mode & 0o7777 != 0o700
            || attributes.st_uid != unsafe { libc::geteuid() }
        {
            return Err("DB_AUTH_PROBE_TASK_REJECTED");
        }
        let task = Self {
            root: root.to_owned(),
            directory,
        };
        if task.read(&root.join("OWNER"), 256)? != owner {
            return Err("DB_AUTH_PROBE_TASK_REJECTED");
        }
        Ok(task)
    }

    fn name(&self, path: &Path) -> LocalResult<CString> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("DB_AUTH_PROBE_FILE_REJECTED")?;
        if !path.is_absolute()
            || path.parent() != Some(self.root.as_path())
            || name.is_empty()
            || name.len() > 128
            || matches!(name, "." | "..")
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err("DB_AUTH_PROBE_FILE_REJECTED");
        }
        CString::new(name).map_err(|_| "DB_AUTH_PROBE_FILE_REJECTED")
    }

    pub(super) fn read(&self, path: &Path, maximum: usize) -> LocalResult<Vec<u8>> {
        let name = self.name(path)?;
        // SAFETY: openat uses our pinned directory fd and one validated component.
        let descriptor = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if descriptor < 0 {
            return Err("DB_AUTH_PROBE_FILE_REJECTED");
        }
        let file = unsafe { File::from_raw_fd(descriptor) };
        let before = metadata(&file)?;
        if before.st_mode & libc::S_IFMT != libc::S_IFREG
            || before.st_mode & 0o7777 != 0o600
            || before.st_uid != unsafe { libc::geteuid() }
            || before.st_nlink != 1
            || before.st_size < 1
            || before.st_size as u64 > maximum as u64
        {
            return Err("DB_AUTH_PROBE_FILE_REJECTED");
        }
        let mut result = Vec::with_capacity(before.st_size as usize);
        (&file)
            .take(maximum as u64 + 1)
            .read_to_end(&mut result)
            .map_err(|_| "DB_AUTH_PROBE_FILE_REJECTED")?;
        let after = metadata(&file)?;
        if result.len() != before.st_size as usize
            || before.st_dev != after.st_dev
            || before.st_ino != after.st_ino
            || before.st_size != after.st_size
            || before.st_mtime != after.st_mtime
            || before.st_mtime_nsec != after.st_mtime_nsec
            || before.st_ctime != after.st_ctime
            || before.st_ctime_nsec != after.st_ctime_nsec
            || before.st_uid != after.st_uid
            || before.st_mode != after.st_mode
            || after.st_nlink != 1
        {
            return Err("DB_AUTH_PROBE_FILE_REJECTED");
        }
        Ok(result)
    }

    pub(super) fn create(&self, path: &Path) -> LocalResult<File> {
        let name = self.name(path)?;
        if !name.to_bytes().starts_with(b"phase23-") {
            return Err("DB_AUTH_PROBE_OUTPUT_REJECTED");
        }
        let descriptor = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if descriptor < 0 {
            return Err("DB_AUTH_PROBE_OUTPUT_REJECTED");
        }
        let file = unsafe { File::from_raw_fd(descriptor) };
        if unsafe { libc::fchmod(file.as_raw_fd(), 0o600) } != 0 {
            return Err("DB_AUTH_PROBE_OUTPUT_REJECTED");
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{symlink, PermissionsExt},
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "phase23-probe-files-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            fs::write(path.join("OWNER"), b"owned-test\n").unwrap();
            fs::set_permissions(path.join("OWNER"), fs::Permissions::from_mode(0o600)).unwrap();
            Self(path)
        }
        fn file(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, b"test-only-material").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.0.parent() == Some(std::env::temp_dir().as_path())
                && self
                    .0
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("phase23-probe-files-")
            {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }

    #[test]
    fn regular_owned_0600_files_and_exclusive_output_are_accepted() {
        let fixture = Fixture::new();
        let task = OwnedTask::open(&fixture.0, b"owned-test\n").unwrap();
        let file = fixture.file("phase23-password");
        assert!(task.read(&file, 128).is_ok());
        let output = fixture.0.join("phase23-output.json");
        let opened = task.create(&output).unwrap();
        drop(opened);
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(task.create(&output).is_err());
    }

    #[test]
    fn symlink_hardlink_mode_escape_and_bounds_are_rejected() {
        let fixture = Fixture::new();
        let task = OwnedTask::open(&fixture.0, b"owned-test\n").unwrap();
        let file = fixture.file("phase23-password");
        let link = fixture.0.join("phase23-link");
        symlink(&file, &link).unwrap();
        assert!(task.read(&link, 128).is_err());
        assert!(task.create(&link).is_err());
        assert!(task
            .read(&fixture.0.join("sub/../phase23-password"), 128)
            .is_err());
        assert!(task.read(&file, 2).is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(task.read(&file, 128).is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&file, fixture.0.join("phase23-hardlink")).unwrap();
        assert!(task.read(&file, 128).is_err());
        fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(OwnedTask::open(&fixture.0, b"owned-test\n").is_err());
    }

    #[test]
    fn wrong_marker_directory_symlink_and_fifo_reject_without_blocking() {
        let fixture = Fixture::new();
        assert!(OwnedTask::open(&fixture.0, b"wrong-marker\n").is_err());
        let other = Fixture::new();
        let linked = other.0.join("phase23-directory-link");
        symlink(&fixture.0, &linked).unwrap();
        assert!(OwnedTask::open(&linked, b"owned-test\n").is_err());
        let task = OwnedTask::open(&fixture.0, b"owned-test\n").unwrap();
        let fifo = fixture.0.join("phase23-fifo");
        let name = CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: this is a new FIFO under the owned, disposable test directory.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(task.read(&fifo, 128).is_err());
    }
}
