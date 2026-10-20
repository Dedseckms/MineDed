//! Small filesystem helpers: atomic writes and owner-only files.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use lc_types::{AppError, AppResult, ErrorCode};

pub(crate) fn io_err(op: &str, path: &Path, e: std::io::Error) -> AppError {
    AppError::new(ErrorCode::Io, format!("{op}: {e}")).detail("path", path.display().to_string())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Opens a new file for writing; on Unix it is created with mode 0600.
///
/// On Windows files under the user profile inherit an owner-only ACL, so no
/// extra steps are taken there.
fn create_private(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

/// Writes `data` to `path` atomically: `<path>.tmp` + fsync + rename.
///
/// Either the old or the new content is visible, never a partial file.
/// `private` = owner-only permissions (Unix 0600).
pub(crate) fn write_atomic(path: &Path, data: &[u8], private: bool) -> AppResult<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| io_err("create dir", dir, e))?;
    }
    let tmp = tmp_path(path);
    let result = (|| -> std::io::Result<()> {
        let mut f = if private {
            create_private(&tmp)?
        } else {
            File::create(&tmp)?
        };
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        // Best effort: do not leave a half-written temp file behind.
        let _ = fs::remove_file(&tmp);
        return Err(io_err("atomic write", path, e));
    }
    Ok(())
}

/// Reads a file, `Ok(None)` if it does not exist.
pub(crate) fn read_optional(path: &Path) -> AppResult<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err("read", path, e)),
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::{Path, PathBuf};

    /// Self-cleaning temp dir (no extra dev-dependency).
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let p = std::env::temp_dir().join(format!("lc-accounts-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::TempDir;
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_tmp() {
        let dir = TempDir::new();
        let p = dir.path().join("a.json");
        write_atomic(&p, b"one", false).unwrap();
        write_atomic(&p, b"two", false).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        assert!(!tmp_path(&p).exists());
    }

    #[test]
    fn failed_write_keeps_old_content() {
        let dir = TempDir::new();
        let p = dir.path().join("a.json");
        write_atomic(&p, b"old", false).unwrap();
        // A directory in place of the temp file makes the write fail midway.
        fs::create_dir_all(tmp_path(&p)).unwrap();
        let err = write_atomic(&p, b"new", false).unwrap_err();
        assert_eq!(err.code, ErrorCode::Io);
        assert_eq!(fs::read(&p).unwrap(), b"old");
    }

    #[cfg(unix)]
    #[test]
    fn private_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let p = dir.path().join("k");
        write_atomic(&p, b"x", true).unwrap();
        let mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
