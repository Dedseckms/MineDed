//! Small helpers shared by the core modules: hashing, JSON files, stage sink.

use std::path::{Path, PathBuf};

use lc_runtime::{CancelToken, ProgressSink};
use lc_types::task::TaskEvent;
use lc_types::{AppError, AppResult, ErrorCode, TaskId};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha1::{Digest, Sha1};
use url::Url;

/// Wraps an `io::Error` with the path it happened on.
pub(crate) fn io_err(path: &Path, e: std::io::Error) -> AppError {
    let code = match e.kind() {
        std::io::ErrorKind::NotFound => ErrorCode::NotFound,
        std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
        _ => ErrorCode::Io,
    };
    AppError::new(code, e.to_string()).detail("path", path.to_string_lossy())
}

pub(crate) fn json_err(what: &str, e: serde_json::Error) -> AppError {
    AppError::new(ErrorCode::Internal, format!("invalid {what}: {e}")).detail("what", what)
}

pub(crate) fn sha1_hex(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

/// SHA-1 of a file, `None` when it does not exist.
pub(crate) async fn file_sha1(path: &Path) -> AppResult<Option<String>> {
    let path = path.to_path_buf();
    join(
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut f = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(io_err(&path, e)),
            };
            let mut h = Sha1::new();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = f.read(&mut buf).map_err(|e| io_err(&path, e))?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(Some(hex::encode(h.finalize())))
        })
        .await,
    )
}

/// Flattens a `spawn_blocking` join result.
pub(crate) fn join<T>(r: Result<AppResult<T>, tokio::task::JoinError>) -> AppResult<T> {
    r.map_err(|e| AppError::internal(format!("blocking task failed: {e}")))?
}

pub(crate) async fn read_json<T: DeserializeOwned>(path: &Path) -> AppResult<T> {
    let bytes = tokio::fs::read(path).await.map_err(|e| io_err(path, e))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| json_err("json file", e).detail("path", path.to_string_lossy()))
}

/// Writes through a temporary file + rename so readers never see half a file.
pub(crate) async fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| io_err(parent, e))?;
    }
    let tmp = tmp_path(path);
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| io_err(&tmp, e))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| io_err(path, e))
}

pub(crate) async fn write_json<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| json_err("json value", e))?;
    write_atomic(path, &bytes).await
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

pub(crate) fn parse_url(s: &str) -> AppResult<Url> {
    Url::parse(s).map_err(|e| {
        AppError::new(ErrorCode::InvalidInput, format!("bad url: {e}")).detail("url", s)
    })
}

pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Forwards everything to `inner` but labels progress with a fixed stage key,
/// so the UI sees `stages.libraries` etc. regardless of what the downloader reports.
pub(crate) struct StageSink<'a> {
    pub inner: &'a dyn ProgressSink,
    pub stage: &'static str,
}

impl ProgressSink for StageSink<'_> {
    fn task_id(&self) -> TaskId {
        self.inner.task_id()
    }

    fn emit(&self, event: TaskEvent) {
        let event = match event {
            TaskEvent::Progress {
                id,
                done,
                total,
                speed_bps,
                ..
            } => TaskEvent::Progress {
                id,
                done,
                total,
                stage: self.stage.to_owned(),
                speed_bps,
            },
            other => other,
        };
        self.inner.emit(event);
    }

    fn cancel_token(&self) -> &CancelToken {
        self.inner.cancel_token()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_value() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[tokio::test]
    async fn atomic_write_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a/b/c.txt");
        write_atomic(&p, b"abc").await.unwrap();
        assert_eq!(
            file_sha1(&p).await.unwrap().as_deref(),
            Some("a9993e364706816aba3e25717850c26c9cd0d89d")
        );
        assert_eq!(file_sha1(&dir.path().join("none")).await.unwrap(), None);
    }
}
