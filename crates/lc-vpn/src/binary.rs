//! The mihomo executable: a bundled sidecar next to the launcher, or a pinned
//! release downloaded on first use into `Paths::mihomo()/bin` and verified by
//! SHA-256 (GitHub release digests).

use std::io::Read;
use std::path::{Path, PathBuf};

use lc_http::{DownloadReq, Downloader, ResourceKind};
use lc_runtime::{Paths, ProgressSink};
use lc_types::{AppError, AppResult, ErrorCode};
use sha2::{Digest, Sha256};
use url::Url;

/// Pinned mihomo release. Bump together with every digest below.
pub const MIHOMO_VERSION: &str = "v1.19.33";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Packing {
    Zip,
    Gzip,
}

/// One release asset for a platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Asset {
    pub name: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub packing: Packing,
}

/// `compatible` builds run on any x86-64 CPU (no AVX/v3 requirement).
const ASSETS: &[(&str, &str, Asset)] = &[
    (
        "windows",
        "x86_64",
        Asset {
            name: "mihomo-windows-amd64-compatible-v1.19.33.zip",
            sha256: "7aaf636a4f2172dcd8ec34ac4f52797b059195e5afd6409508c963e545f8864d",
            size: 22_543_546,
            packing: Packing::Zip,
        },
    ),
    (
        "windows",
        "aarch64",
        Asset {
            name: "mihomo-windows-arm64-v1.19.33.zip",
            sha256: "a2a26869a14634c49f3fbcecc593b498ed8591fbdd7616b24475b3d60b148da3",
            size: 20_212_418,
            packing: Packing::Zip,
        },
    ),
    (
        "linux",
        "x86_64",
        Asset {
            name: "mihomo-linux-amd64-compatible-v1.19.33.gz",
            sha256: "386508bfb1c72c43b6a0eda73708ecdce82546581b0ebcb16366c53757c622b6",
            size: 22_893_972,
            packing: Packing::Gzip,
        },
    ),
    (
        "linux",
        "aarch64",
        Asset {
            name: "mihomo-linux-arm64-v1.19.33.gz",
            sha256: "ba408b317080ee265c46cb923a50018c1b82cbd708551cdd46a1e3e307396187",
            size: 20_824_326,
            packing: Packing::Gzip,
        },
    ),
];

/// Upper bound for the unpacked executable (guards against zip bombs).
const MAX_EXE: u64 = 256 * 1024 * 1024;

pub(crate) fn asset_for(os: &str, arch: &str) -> Option<Asset> {
    ASSETS
        .iter()
        .find(|(o, a, _)| *o == os && *a == arch)
        .map(|(_, _, asset)| *asset)
}

pub(crate) fn exe_name() -> &'static str {
    if cfg!(windows) {
        "mihomo.exe"
    } else {
        "mihomo"
    }
}

fn release_url(asset: &Asset) -> AppResult<Url> {
    Url::parse(&format!(
        "https://github.com/MetaCubeX/mihomo/releases/download/{MIHOMO_VERSION}/{}",
        asset.name
    ))
    .map_err(|e| AppError::internal(format!("bad release URL: {e}")))
}

/// Project mirror (nginx mirror-cache on our server), same layout as GitHub.
const PROJECT_MIRROR: &str = "https://relay.dedseckms.ru/mirror/mihomo";
/// Public GitHub release proxies, prefixed to the release URL. Safe to use:
/// the archive is checked against the pinned SHA-256 before it is unpacked.
const GITHUB_PROXIES: &[&str] = &["https://ghfast.top/", "https://gh-proxy.com/"];

/// Download candidates in order: GitHub, the project mirror, public proxies.
pub(crate) fn candidate_urls(asset: &Asset) -> AppResult<Vec<Url>> {
    let github = release_url(asset)?;
    let mut out = vec![github.clone()];
    let bad = |e: url::ParseError| AppError::internal(format!("bad mirror URL: {e}"));
    out.push(
        Url::parse(&format!("{PROJECT_MIRROR}/{MIHOMO_VERSION}/{}", asset.name)).map_err(bad)?,
    );
    for p in GITHUB_PROXIES {
        out.push(Url::parse(&format!("{p}{github}")).map_err(bad)?);
    }
    Ok(out)
}

/// Where the downloaded executable lives.
pub(crate) fn managed_path(paths: &Paths) -> PathBuf {
    paths
        .mihomo()
        .join("bin")
        .join(MIHOMO_VERSION)
        .join(exe_name())
}

/// A sidecar shipped next to the launcher executable (Tauri `externalBin`).
fn bundled_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join(exe_name());
    p.is_file().then_some(p)
}

/// Returns a usable executable: `override_path`, else bundled, else managed
/// (downloading and verifying it first if missing).
pub(crate) async fn ensure(
    paths: &Paths,
    downloader: &dyn Downloader,
    sink: &dyn ProgressSink,
    override_path: Option<&Path>,
) -> AppResult<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.to_path_buf());
    }
    if let Some(p) = bundled_path() {
        return Ok(p);
    }
    let target = managed_path(paths);
    if target.is_file() {
        return Ok(target);
    }
    let asset = asset_for(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        AppError::new(
            ErrorCode::NotImplemented,
            "no mihomo build for this platform",
        )
        .detail("os", std::env::consts::OS)
        .detail("arch", std::env::consts::ARCH)
    })?;
    let dir = target
        .parent()
        .ok_or_else(|| AppError::internal("mihomo path has no parent"))?
        .to_path_buf();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| io_err(e, &dir))?;
    fetch_verified(downloader, sink, &asset, candidate_urls(&asset)?, &target).await?;
    Ok(target)
}

/// Tries `urls` in order until one yields an archive matching the pinned
/// digest, then installs it to `target`.
pub(crate) async fn fetch_verified(
    downloader: &dyn Downloader,
    sink: &dyn ProgressSink,
    asset: &Asset,
    urls: Vec<Url>,
    target: &Path,
) -> AppResult<()> {
    let asset = *asset;
    let dir = target
        .parent()
        .ok_or_else(|| AppError::internal("mihomo path has no parent"))?;
    let archive = dir.join(asset.name);
    let mut last: Option<AppError> = None;
    // One source at a time: a mirror serving a wrong file must not stop the
    // next one from being tried (the downloader can't verify SHA-256).
    for (i, url) in urls.into_iter().enumerate() {
        sink.check_cancelled()?;
        let _ = tokio::fs::remove_file(&archive).await;
        sink.progress(0, Some(asset.size), "vpn.download");
        let fetched = downloader
            .fetch_many(
                vec![DownloadReq {
                    urls: vec![url],
                    sha1: None,
                    size: Some(asset.size),
                    dest: archive.clone(),
                    kind: ResourceKind::Other,
                }],
                sink,
            )
            .await;
        let res = match fetched {
            Ok(()) => {
                let (archive2, target2) = (archive.clone(), target.to_path_buf());
                tokio::task::spawn_blocking(move || install(&archive2, &asset, &target2))
                    .await
                    .map_err(|e| AppError::internal(format!("unpack task failed: {e}")))
                    .and_then(|r| r)
            }
            Err(e) => Err(e),
        };
        let _ = tokio::fs::remove_file(&archive).await;
        match res {
            Ok(()) => {
                tracing::info!(version = MIHOMO_VERSION, source = i, "mihomo installed");
                return Ok(());
            }
            Err(e) if e.code == ErrorCode::Cancelled => return Err(e),
            Err(e) => {
                tracing::warn!(source = i, error = %e.message, "mihomo source failed");
                last = Some(e);
            }
        }
    }
    Err(unavailable(last, target))
}

/// Every source failed: tell the user how to get past it.
fn unavailable(last: Option<AppError>, target: &Path) -> AppError {
    let mut e = AppError::new(
        ErrorCode::Network,
        "cannot download mihomo from GitHub or its mirrors; set a manual proxy in settings \
         and retry, or place the mihomo executable manually",
    )
    .detail("hint", "useProxy")
    .detail("version", MIHOMO_VERSION)
    .detail("manualPath", target.display().to_string());
    if let Some(last) = last {
        e = e.detail("lastError", last.message);
    }
    e
}

/// Verifies `archive` against the pinned digest and unpacks the executable to
/// `target` atomically (`*.tmp` + rename).
pub(crate) fn install(archive: &Path, asset: &Asset, target: &Path) -> AppResult<()> {
    let bytes = std::fs::read(archive).map_err(|e| io_err(e, archive))?;
    let actual = hex::encode(Sha256::digest(&bytes));
    if !actual.eq_ignore_ascii_case(asset.sha256) {
        return Err(AppError::new(
            ErrorCode::HashMismatch,
            "mihomo archive failed verification",
        )
        .detail("expectedSha256", asset.sha256)
        .detail("actualSha256", actual));
    }
    let exe = unpack(&bytes, asset.packing)?;
    let tmp = target.with_extension("tmp");
    std::fs::write(&tmp, &exe).map_err(|e| io_err(e, &tmp))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| io_err(e, &tmp))?;
    }
    std::fs::rename(&tmp, target).map_err(|e| io_err(e, target))
}

fn unpack(bytes: &[u8], packing: Packing) -> AppResult<Vec<u8>> {
    let bad = |what: String| AppError::new(ErrorCode::Io, format!("cannot unpack mihomo: {what}"));
    let mut out = Vec::new();
    match packing {
        Packing::Gzip => {
            flate2::read::GzDecoder::new(bytes)
                .take(MAX_EXE)
                .read_to_end(&mut out)
                .map_err(|e| bad(e.to_string()))?;
        }
        Packing::Zip => {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
                .map_err(|e| bad(e.to_string()))?;
            let idx = (0..zip.len())
                .find(|&i| {
                    zip.by_index(i).is_ok_and(|f| {
                        f.is_file() && f.name().to_ascii_lowercase().ends_with(".exe")
                    })
                })
                .ok_or_else(|| bad("no executable in archive".into()))?;
            zip.by_index(idx)
                .map_err(|e| bad(e.to_string()))?
                .take(MAX_EXE)
                .read_to_end(&mut out)
                .map_err(|e| bad(e.to_string()))?;
        }
    }
    if out.is_empty() {
        return Err(bad("empty executable".into()));
    }
    Ok(out)
}

fn io_err(e: std::io::Error, path: &Path) -> AppError {
    AppError::new(ErrorCode::Io, e.to_string()).detail("path", path.display().to_string())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("lc-vpn-bin-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn every_supported_platform_is_pinned() {
        for (os, arch) in [
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ] {
            let a = asset_for(os, arch).unwrap();
            assert!(a.name.contains(MIHOMO_VERSION), "{}", a.name);
            assert_eq!(a.sha256.len(), 64);
            assert!(release_url(&a).unwrap().as_str().ends_with(a.name));
            let urls = candidate_urls(&a).unwrap();
            assert_eq!(urls[0], release_url(&a).unwrap());
            assert!(urls.len() >= 3);
            assert!(urls
                .iter()
                .all(|u| u.scheme() == "https" && u.as_str().ends_with(a.name)));
        }
        assert!(asset_for("macos", "x86_64").is_none());
    }

    #[test]
    fn install_verifies_and_unpacks() {
        let dir = tmp("gz");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(b"ELF-binary").unwrap();
        let archive_bytes = gz.finish().unwrap();
        let archive = dir.join("a.gz");
        std::fs::write(&archive, &archive_bytes).unwrap();
        let digest = hex::encode(Sha256::digest(&archive_bytes));
        let digest: &'static str = Box::leak(digest.into_boxed_str());
        let asset = Asset {
            name: "a.gz",
            sha256: digest,
            size: archive_bytes.len() as u64,
            packing: Packing::Gzip,
        };
        let target = dir.join("mihomo");
        install(&archive, &asset, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"ELF-binary");

        let wrong = Asset {
            sha256: "00",
            ..asset
        };
        let err = install(&archive, &wrong, &dir.join("x")).unwrap_err();
        assert_eq!(err.code, ErrorCode::HashMismatch);
        assert!(!dir.join("x").exists());

        // Zip with a single .exe entry.
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file::<_, ()>("readme.txt", zip::write::FileOptions::default())
                .unwrap();
            z.write_all(b"hi").unwrap();
            z.start_file::<_, ()>(
                "mihomo-windows-amd64.exe",
                zip::write::FileOptions::default(),
            )
            .unwrap();
            z.write_all(b"MZ-binary").unwrap();
            z.finish().unwrap();
        }
        assert_eq!(unpack(buf.get_ref(), Packing::Zip).unwrap(), b"MZ-binary");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// GitHub unreachable, first mirror serves a wrong file: the next mirror
    /// is used. Nothing reachable: a clear error suggesting a proxy.
    #[tokio::test]
    async fn falls_back_across_sources() {
        use lc_http::{DownloaderConfig, HttpDownloader, SystemProxyMode};
        use lc_runtime::NullSink;

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(b"ELF-binary").unwrap();
        let good = gz.finish().unwrap();
        let digest: &'static str = Box::leak(hex::encode(Sha256::digest(&good)).into_boxed_str());
        let asset = Asset {
            name: "m.gz",
            sha256: digest,
            size: good.len() as u64,
            packing: Packing::Gzip,
        };
        let bad = vec![0u8; good.len()];
        let app = axum::Router::new()
            .route("/bad/m.gz", axum::routing::get(move || async move { bad }))
            .route(
                "/good/m.gz",
                axum::routing::get(move || async move { good }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        // A port nobody listens on stands in for a blocked GitHub.
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = dead.local_addr().unwrap().port();
        drop(dead);

        let d = HttpDownloader::new(DownloaderConfig {
            system_proxy: SystemProxyMode::Off,
            attempts_per_url: 1,
            ..DownloaderConfig::default()
        })
        .unwrap();
        let u = |s: String| Url::parse(&s).unwrap();
        let dir = tmp("fallback");
        let target = dir.join("mihomo");
        let urls = vec![
            u(format!("http://127.0.0.1:{dead_port}/m.gz")),
            u(format!("http://127.0.0.1:{port}/bad/m.gz")),
            u(format!("http://127.0.0.1:{port}/good/m.gz")),
        ];
        fetch_verified(&d, &NullSink::default(), &asset, urls, &target)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"ELF-binary");
        assert!(!dir.join("m.gz").exists());

        let err = fetch_verified(
            &d,
            &NullSink::default(),
            &asset,
            vec![u(format!("http://127.0.0.1:{dead_port}/m.gz"))],
            &dir.join("other"),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::Network);
        assert_eq!(err.details.unwrap()["hint"], "useProxy");
        let _ = std::fs::remove_dir_all(dir);
    }
}
