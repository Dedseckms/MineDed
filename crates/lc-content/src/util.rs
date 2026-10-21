//! Helpers shared by content modules.

use std::path::{Component, Path, PathBuf};

use lc_types::content::ContentKind;
use lc_types::version::LoaderKind;
use lc_types::{AppError, AppResult, ErrorCode};
use sha1::{Digest, Sha1};
use sha2::Sha512;
use url::Url;

pub(crate) fn io_err(path: &Path, e: std::io::Error) -> AppError {
    let code = if e.kind() == std::io::ErrorKind::NotFound {
        ErrorCode::NotFound
    } else {
        ErrorCode::Io
    };
    AppError::new(code, e.to_string()).detail("path", path.to_string_lossy())
}

pub(crate) fn parse_url(s: &str) -> AppResult<Url> {
    Url::parse(s).map_err(|e| {
        AppError::new(ErrorCode::InvalidInput, format!("bad url: {e}")).detail("url", s)
    })
}

pub(crate) fn join<T>(r: Result<AppResult<T>, tokio::task::JoinError>) -> AppResult<T> {
    r.map_err(|e| AppError::internal(format!("blocking task failed: {e}")))?
}

pub(crate) fn sha1_hex(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

/// (sha1, sha512) of a file.
pub(crate) fn file_hashes_blocking(path: &Path) -> AppResult<(String, String)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| io_err(path, e))?;
    let (mut a, mut b) = (Sha1::new(), Sha512::new());
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| io_err(path, e))?;
        if n == 0 {
            break;
        }
        a.update(&buf[..n]);
        b.update(&buf[..n]);
    }
    Ok((hex::encode(a.finalize()), hex::encode(b.finalize())))
}

pub(crate) async fn file_hashes(path: &Path) -> AppResult<(String, String)> {
    let p = path.to_path_buf();
    join(tokio::task::spawn_blocking(move || file_hashes_blocking(&p)).await)
}

/// A bare file name (no separators, no `..`, no drive).
pub(crate) fn check_file_name(name: &str) -> AppResult<&str> {
    let bad = || AppError::new(ErrorCode::InvalidInput, "bad file name").detail("file", name);
    if name.is_empty() || name.len() > 255 || name.contains(['/', '\\', ':', '\0']) {
        return Err(bad());
    }
    let mut c = Path::new(name).components();
    match (c.next(), c.next()) {
        (Some(Component::Normal(_)), None) => Ok(name),
        _ => Err(bad()),
    }
}

/// Joins a relative `/`-separated path under `root`, rejecting escapes.
pub(crate) fn safe_join(root: &Path, rel: &str) -> AppResult<PathBuf> {
    let bad = || AppError::new(ErrorCode::InvalidInput, "unsafe relative path").detail("path", rel);
    let rel = rel.replace('\\', "/");
    let mut out = root.to_path_buf();
    let mut any = false;
    for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
        check_file_name(part).map_err(|_| bad())?;
        if part == ".." {
            return Err(bad());
        }
        out.push(part);
        any = true;
    }
    if any {
        Ok(out)
    } else {
        Err(bad())
    }
}

/// Folder of the game dir for a content kind.
pub(crate) fn kind_dir(kind: ContentKind) -> AppResult<&'static str> {
    match kind {
        ContentKind::Mod => Ok("mods"),
        ContentKind::ResourcePack => Ok("resourcepacks"),
        ContentKind::Shader => Ok("shaderpacks"),
        ContentKind::DataPack => Ok("datapacks"),
        ContentKind::Modpack => Err(AppError::new(
            ErrorCode::InvalidInput,
            "modpacks are imported as new instances",
        )),
    }
}

pub(crate) const CONTENT_DIRS: [(&str, ContentKind); 4] = [
    ("mods", ContentKind::Mod),
    ("resourcepacks", ContentKind::ResourcePack),
    ("shaderpacks", ContentKind::Shader),
    ("datapacks", ContentKind::DataPack),
];

pub(crate) fn loader_from_str(s: &str) -> Option<LoaderKind> {
    match s.to_ascii_lowercase().as_str() {
        "fabric" => Some(LoaderKind::Fabric),
        "quilt" => Some(LoaderKind::Quilt),
        "forge" => Some(LoaderKind::Forge),
        "neoforge" => Some(LoaderKind::NeoForge),
        _ => None,
    }
}

/// Whether a file built for `file_loaders` runs on `target` (Quilt runs Fabric mods).
pub(crate) fn loader_compatible(file_loaders: &[LoaderKind], target: LoaderKind) -> bool {
    file_loaders.contains(&target)
        || (target == LoaderKind::Quilt && file_loaders.contains(&LoaderKind::Fabric))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert!(check_file_name("sodium-0.6.jar").is_ok());
        assert!(check_file_name("../x.jar").is_err());
        assert!(check_file_name("a/b.jar").is_err());
        assert!(check_file_name("..").is_err());
        assert!(check_file_name("C:x").is_err());
        assert!(safe_join(Path::new("/g"), "config/a.toml").is_ok());
        assert!(safe_join(Path::new("/g"), "config/../../a").is_err());
    }

    #[test]
    fn quilt_runs_fabric() {
        assert!(loader_compatible(&[LoaderKind::Fabric], LoaderKind::Quilt));
        assert!(!loader_compatible(&[LoaderKind::Quilt], LoaderKind::Fabric));
    }
}
