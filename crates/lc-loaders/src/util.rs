//! Small helpers shared by the installers.

use std::cmp::Ordering;
use std::path::{Component, Path, PathBuf};

use lc_http::{Downloader, ResourceKind};
use lc_types::{AppError, AppResult, ErrorCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha1::{Digest, Sha1};
use url::Url;

pub(crate) fn io_err(path: &Path, e: std::io::Error) -> AppError {
    AppError::new(ErrorCode::Io, e.to_string()).detail("path", path.to_string_lossy())
}

pub(crate) fn loader_err(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::LoaderInstall, message)
}

pub(crate) fn parse_url(s: &str) -> AppResult<Url> {
    Url::parse(s).map_err(|e| {
        AppError::new(ErrorCode::InvalidInput, format!("bad url: {e}")).detail("url", s)
    })
}

pub(crate) fn join<T>(r: Result<AppResult<T>, tokio::task::JoinError>) -> AppResult<T> {
    r.map_err(|e| AppError::internal(format!("blocking task failed: {e}")))?
}

/// Reject existing symlinks and Windows reparse points, including junctions.
/// Concurrent replacement by another local process is outside this check.
pub(crate) fn ensure_no_links(path: &Path) -> AppResult<()> {
    for part in path.ancestors() {
        match std::fs::symlink_metadata(part) {
            Ok(meta) => {
                #[cfg(windows)]
                let reparse = {
                    use std::os::windows::fs::MetadataExt;
                    meta.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let reparse = false;
                if meta.file_type().is_symlink() || reparse {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        "linked path is not allowed",
                    )
                    .detail("path", part.to_string_lossy()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(part, e)),
        }
    }
    Ok(())
}

pub(crate) fn sha1_hex(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

/// SHA-1 of a file, `None` when it does not exist.
pub(crate) fn file_sha1_blocking(path: &Path) -> AppResult<Option<String>> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(path, e)),
    };
    let mut h = Sha1::new();
    std::io::copy(&mut f, &mut h).map_err(|e| io_err(path, e))?;
    Ok(Some(hex::encode(h.finalize())))
}

pub(crate) async fn file_sha1(path: &Path) -> AppResult<Option<String>> {
    let p = path.to_path_buf();
    join(tokio::task::spawn_blocking(move || file_sha1_blocking(&p)).await)
}

/// Joins a relative, `/`-separated path under `root`, rejecting escapes.
pub(crate) fn safe_join(root: &Path, rel: &str) -> AppResult<PathBuf> {
    let bad = || AppError::new(ErrorCode::InvalidInput, "unsafe relative path").detail("path", rel);
    let rel = rel.replace('\\', "/");
    if rel.is_empty() {
        return Err(bad());
    }
    let mut out = root.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
        // Windows aliases trailing dots/spaces and reserves device names.
        let stem = part
            .split('.')
            .next()
            .unwrap_or(part)
            .trim_end()
            .to_ascii_uppercase();
        if part.ends_with(['.', ' '])
            || part.contains([':', '\0', '<', '>', '"', '|', '?', '*'])
            || part.chars().any(char::is_control)
            || matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            )
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(bad());
        }
        let comp = Path::new(part);
        let mut comps = comp.components();
        match (comps.next(), comps.next()) {
            (Some(Component::Normal(_)), None) if !part.contains(':') => out.push(part),
            _ => return Err(bad()),
        }
    }
    if out == root {
        return Err(bad());
    }
    ensure_no_links(&out)?;
    Ok(out)
}

pub(crate) async fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    ensure_no_links(path)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| io_err(parent, e))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    ensure_no_links(&tmp)?;
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| io_err(&tmp, e))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| io_err(path, e))
}

pub(crate) async fn write_json<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| AppError::internal(format!("json encode: {e}")))?;
    write_atomic(path, &bytes).await
}

pub(crate) fn parse_json<T: DeserializeOwned>(bytes: &[u8], what: &str) -> AppResult<T> {
    serde_json::from_slice(bytes).map_err(|e| {
        AppError::new(ErrorCode::LoaderInstall, format!("bad {what}: {e}")).detail("what", what)
    })
}

pub(crate) async fn get_json<T: DeserializeOwned>(
    dl: &dyn Downloader,
    url: &str,
    what: &str,
) -> AppResult<T> {
    let bytes = dl.get_bytes(&parse_url(url)?, ResourceKind::Loader).await?;
    parse_json(&bytes, what)
}

/// `group:artifact:version[:classifier][@ext]` → repository path.
pub(crate) fn maven_path(name: &str) -> Option<String> {
    let (coords, ext) = match name.split_once('@') {
        Some((c, e)) => (c, e),
        None => (name, "jar"),
    };
    let mut parts = coords.split(':');
    let group = parts.next().filter(|s| !s.is_empty())?;
    let artifact = parts.next().filter(|s| !s.is_empty())?;
    let version = parts.next().filter(|s| !s.is_empty())?;
    let classifier = parts.next();
    if parts.next().is_some() {
        return None;
    }
    let file = match classifier {
        Some(c) => format!("{artifact}-{version}-{c}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!(
        "{}/{artifact}/{version}/{file}",
        group.replace('.', "/")
    ))
}

/// Compares dotted versions numerically where possible (`47.10.0 > 47.9.1`).
pub(crate) fn cmp_versions(a: &str, b: &str) -> Ordering {
    let split = |s: &str| -> Vec<String> {
        s.split(['.', '-', '+'])
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let (pa, pb) = (split(a), split(b));
    for (x, y) in pa.iter().zip(pb.iter()) {
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(n), Ok(m)) => n.cmp(&m),
            _ => x.cmp(y),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    pa.len().cmp(&pb.len())
}

/// `<version>` entries of a `maven-metadata.xml`.
pub(crate) fn maven_metadata_versions(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<version>") {
        let after = &rest[start + "<version>".len()..];
        let Some(end) = after.find("</version>") else {
            break;
        };
        let v = after[..end].trim();
        if !v.is_empty() {
            out.push(v.to_owned());
        }
        rest = &after[end..];
    }
    out
}

/// `tokio::process::Command` that does not flash a console window on Windows.
pub(crate) fn command(program: &Path) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_paths() {
        assert_eq!(
            maven_path("net.fabricmc:fabric-loader:0.16.9").as_deref(),
            Some("net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar")
        );
        assert_eq!(
            maven_path("de.oceanlabs.mcp:mcp_config:1.20.1-20230612.114412@zip").as_deref(),
            Some("de/oceanlabs/mcp/mcp_config/1.20.1-20230612.114412/mcp_config-1.20.1-20230612.114412.zip")
        );
        assert_eq!(
            maven_path("net.minecraft:client:1.20.1-20230612.114412:srg").as_deref(),
            Some(
                "net/minecraft/client/1.20.1-20230612.114412/client-1.20.1-20230612.114412-srg.jar"
            )
        );
        assert_eq!(maven_path("bad"), None);
    }

    #[test]
    fn version_order() {
        assert_eq!(cmp_versions("47.10.0", "47.9.1"), Ordering::Greater);
        assert_eq!(cmp_versions("21.1.77", "21.1.77"), Ordering::Equal);
        assert_eq!(cmp_versions("21.1.7", "21.1.77"), Ordering::Less);
    }

    #[test]
    fn metadata_parsing() {
        let xml = "<metadata><versioning><versions><version>1.20.1-47.3.0</version>\
                   <version> 1.20.1-47.2.0 </version></versions></versioning></metadata>";
        assert_eq!(
            maven_metadata_versions(xml),
            vec!["1.20.1-47.3.0", "1.20.1-47.2.0"]
        );
    }

    #[test]
    fn safe_join_rejects_escape() {
        let root = Path::new("/r");
        assert!(safe_join(root, "a/b.txt").is_ok());
        assert!(safe_join(root, "../x").is_err());
        assert!(safe_join(root, "a/../../x").is_err());
        assert!(safe_join(root, "C:/x").is_err());
        assert!(safe_join(root, "").is_err());
        assert!(safe_join(root, "a/.. /x").is_err());
        assert!(safe_join(root, "a/NUL.jar").is_err());
        assert!(safe_join(root, "a/file:stream").is_err());
    }

    #[tokio::test]
    async fn refuses_redirected_libraries_and_temporary_writes() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let redirected = dir.path().join("redirected");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &redirected).unwrap();
        #[cfg(windows)]
        {
            let result = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&redirected)
                .arg(outside.path())
                .output()
                .unwrap();
            assert!(result.status.success(), "{:?}", result);
        }
        assert!(safe_join(dir.path(), "redirected/new/library.jar").is_err());
        assert!(write_atomic(&redirected.join("profile.json"), b"write")
            .await
            .is_err());
        assert!(!outside.path().join("profile.json").exists());
    }
}
