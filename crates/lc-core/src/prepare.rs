//! Downloads everything a resolved profile needs: client jar, libraries,
//! natives jars, asset index + objects, logging config.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use lc_http::{DownloadReq, Downloader, ResourceKind};
use lc_runtime::{Paths, ProgressSink};
use lc_types::{AppError, AppResult, ErrorCode};

use crate::meta::{resolve_libraries, AssetIndex, Env, LibFile, VersionJson, RESOURCES_URL};
use crate::util::{io_err, join, parse_url, read_json, safe_component, safe_join, StageSink};

/// Where the game should look for assets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssetsInfo {
    pub index_id: String,
    /// Value of `${game_assets}` (virtual dir, resources dir or assets root).
    pub game_assets: PathBuf,
}

/// Result of preparing a profile; input of the launch plan builder.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    pub profile: VersionJson,
    pub libs: Vec<LibFile>,
    pub client_jar: PathBuf,
    pub assets: AssetsInfo,
    /// Rendered `-Dlog4j.configurationFile=...` argument.
    pub log_arg: Option<String>,
}

pub(crate) fn client_jar_id(profile: &VersionJson) -> &str {
    profile.jar.as_deref().unwrap_or(&profile.id)
}

pub(crate) fn client_jar_path(paths: &Paths, profile: &VersionJson) -> AppResult<PathBuf> {
    let id = client_jar_id(profile);
    safe_component(
        &safe_component(&paths.versions(), id)?,
        &format!("{id}.jar"),
    )
}

fn req(
    url: &str,
    sha1: Option<String>,
    size: Option<u64>,
    dest: PathBuf,
    kind: ResourceKind,
) -> AppResult<DownloadReq> {
    Ok(DownloadReq {
        urls: vec![parse_url(url)?],
        sha1,
        size,
        dest,
        kind,
    })
}

/// Download requests for libraries (deduplicated by destination).
pub(crate) fn library_requests(libs: &[LibFile], libraries: &Path) -> AppResult<Vec<DownloadReq>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for lib in libs {
        let Some(url) = &lib.url else { continue };
        let dest = lib.local(libraries)?;
        if seen.insert(dest.clone()) {
            out.push(req(
                url,
                lib.sha1.clone(),
                lib.size,
                dest,
                ResourceKind::Library,
            )?);
        }
    }
    Ok(out)
}

/// Download requests for asset objects (deduplicated by hash).
pub(crate) fn asset_requests(index: &AssetIndex, assets: &Path) -> AppResult<Vec<DownloadReq>> {
    let objects = assets.join("objects");
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for obj in index.objects.values() {
        if !seen.insert(obj.hash.clone()) {
            continue;
        }
        let rel = obj.rel_path()?;
        let dest = safe_join(&objects, &rel)?;
        out.push(req(
            &format!("{RESOURCES_URL}{rel}"),
            Some(obj.hash.clone()),
            Some(obj.size),
            dest,
            ResourceKind::Asset,
        )?);
    }
    Ok(out)
}

async fn fetch_stage(
    dl: &dyn Downloader,
    reqs: Vec<DownloadReq>,
    verify: bool,
    stage: &'static str,
    sink: &dyn ProgressSink,
) -> AppResult<()> {
    sink.check_cancelled()?;
    let reqs = if verify {
        reqs
    } else {
        // Fast relaunch: only fetch what is missing, skip hashing.
        let mut missing = Vec::new();
        for r in reqs {
            if !tokio::fs::try_exists(&r.dest).await.unwrap_or(false) {
                missing.push(r);
            }
        }
        missing
    };
    if reqs.is_empty() {
        sink.progress(1, Some(1), stage);
        return Ok(());
    }
    let staged = StageSink { inner: sink, stage };
    dl.fetch_many(reqs, &staged).await
}

/// Fetches the log4j config and renders its JVM argument. Optional: some
/// mirrors (BMCLAPI) 404 it, so a failed download only drops the argument and
/// the game logs plain text instead (the log parser handles both forms).
async fn fetch_log_config(
    paths: &Paths,
    dl: &dyn Downloader,
    profile: &VersionJson,
    verify: bool,
    sink: &dyn ProgressSink,
) -> AppResult<Option<String>> {
    let Some(c) = profile.logging.as_ref().and_then(|l| l.client.as_ref()) else {
        return Ok(None);
    };
    let dest = safe_component(&paths.assets().join("log_configs"), &c.file.id)?;
    let r = req(
        &c.file.url,
        c.file.sha1.clone(),
        c.file.size,
        dest.clone(),
        ResourceKind::VersionMeta,
    )?;
    match fetch_stage(dl, vec![r], verify, "stages.versionMeta", sink).await {
        Ok(()) => Ok(Some(c.argument.replace("${path}", &dest.to_string_lossy()))),
        Err(e) if e.code == ErrorCode::Cancelled => Err(e),
        Err(e) => {
            sink.log(&format!(
                "logging config unavailable, using plain console output: {}",
                e.message
            ));
            Ok(None)
        }
    }
}

/// Ensures all files of `profile` exist. `verify = false` skips files that are
/// already present (used by `LaunchOpts::offline_fast`).
pub(crate) async fn prepare(
    paths: &Paths,
    dl: &dyn Downloader,
    profile: VersionJson,
    game_dir: &Path,
    env: &Env,
    verify: bool,
    sink: &dyn ProgressSink,
) -> AppResult<Prepared> {
    let assets_root = paths.assets();

    // Stage 1: asset index + logging config (the rest depends on the index).
    let mut meta = Vec::new();
    let index_ref = profile.asset_index.clone();
    let index_path = match &index_ref {
        Some(i) => Some(safe_component(
            &assets_root.join("indexes"),
            &format!("{}.json", i.id),
        )?),
        None => None,
    };
    if let (Some(i), Some(p)) = (&index_ref, &index_path) {
        meta.push(req(
            &i.url,
            i.sha1.clone(),
            i.size,
            p.clone(),
            ResourceKind::VersionMeta,
        )?);
    }
    fetch_stage(dl, meta, verify, "stages.versionMeta", sink).await?;
    let log_arg = fetch_log_config(paths, dl, &profile, verify, sink).await?;

    // Stage 2: client jar.
    let client_jar = client_jar_path(paths, &profile)?;
    let client = profile.downloads.as_ref().and_then(|d| d.get("client"));
    match client {
        Some(a) if !a.url.is_empty() => {
            let r = req(
                &a.url,
                a.sha1.clone(),
                a.size,
                client_jar.clone(),
                ResourceKind::ClientJar,
            )?;
            fetch_stage(dl, vec![r], verify, "stages.client", sink).await?;
        }
        _ => {
            if !tokio::fs::try_exists(&client_jar).await.unwrap_or(false) {
                return Err(AppError::new(ErrorCode::NotFound, "client jar is missing")
                    .detail("path", client_jar.to_string_lossy()));
            }
        }
    }

    // Stage 3: libraries (classpath + natives jars).
    let libs = resolve_libraries(&profile.libraries, env)?;
    let lib_reqs = library_requests(&libs, &paths.libraries())?;
    fetch_stage(dl, lib_reqs, verify, "stages.libraries", sink).await?;
    for lib in libs.iter().filter(|l| l.url.is_none()) {
        let p = lib.local(&paths.libraries())?;
        if !tokio::fs::try_exists(&p).await.unwrap_or(false) {
            sink.log(&format!(
                "missing library without download url: {}",
                lib.path
            ));
        }
    }

    // Stage 4: assets.
    let assets = match (&index_ref, &index_path) {
        (Some(i), Some(p)) => {
            let index: AssetIndex = read_json(p).await?;
            let reqs = asset_requests(&index, &assets_root)?;
            fetch_stage(dl, reqs, verify, "stages.assets", sink).await?;
            let game_assets = if index.map_to_resources {
                game_dir.join("resources")
            } else if index.is_virtual {
                safe_component(&assets_root.join("virtual"), &i.id)?
            } else {
                assets_root.clone()
            };
            if index.map_to_resources || index.is_virtual {
                copy_legacy_assets(index, assets_root.join("objects"), game_assets.clone()).await?;
            }
            AssetsInfo {
                index_id: i.id.clone(),
                game_assets,
            }
        }
        _ => AssetsInfo {
            index_id: profile
                .assets
                .clone()
                .unwrap_or_else(|| "legacy".to_owned()),
            game_assets: assets_root.clone(),
        },
    };

    Ok(Prepared {
        profile,
        libs,
        client_jar,
        assets,
        log_arg,
    })
}

/// Copies hashed objects to their named paths for `virtual`/`map_to_resources` indexes.
/// Every name is validated before anything is copied.
pub(crate) async fn copy_legacy_assets(
    index: AssetIndex,
    objects: PathBuf,
    target: PathBuf,
) -> AppResult<()> {
    let mut pairs = Vec::with_capacity(index.objects.len());
    for (name, obj) in &index.objects {
        let src = safe_join(&objects, &obj.rel_path()?)?;
        let dst = safe_join(&target, name).map_err(|e| e.detail("asset", name.as_str()))?;
        pairs.push((src, dst, obj.size));
    }
    join(
        tokio::task::spawn_blocking(move || {
            for (src, dst, size) in pairs {
                if std::fs::metadata(&dst)
                    .map(|m| m.len() == size)
                    .unwrap_or(false)
                {
                    continue;
                }
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
                }
                std::fs::copy(&src, &dst).map_err(|e| io_err(&src, e))?;
            }
            Ok(())
        })
        .await,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_types::ErrorCode;

    fn index(name: &str) -> AssetIndex {
        let json = format!(
            r#"{{"virtual":true,"objects":{{{}:{{"hash":"aa11223344556677889900aabbccddeeff001122","size":1}}}}}}"#,
            serde_json::to_string(name).unwrap()
        );
        serde_json::from_str(&json).unwrap()
    }

    #[tokio::test]
    async fn asset_name_with_backslash_stays_inside() {
        let dir = tempfile::tempdir().unwrap();
        let objects = dir.path().join("objects");
        let src = objects
            .join("aa")
            .join("aa11223344556677889900aabbccddeeff001122");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, b"x").unwrap();
        let target = dir.path().join("virtual").join("legacy");

        for bad in [
            r"..\..\evil.txt",
            "../../evil.txt",
            "C:/evil.txt",
            "/evil.txt",
            "a//b",
        ] {
            let err = copy_legacy_assets(index(bad), objects.clone(), target.clone())
                .await
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad}");
        }
        assert!(!dir.path().join("evil.txt").exists());
        assert!(!target.exists());

        copy_legacy_assets(index("sound/a.ogg"), objects, target.clone())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(target.join("sound").join("a.ogg")).unwrap(),
            b"x"
        );
    }

    #[test]
    fn unsafe_ids_rejected() {
        let paths = Paths::new("/data");
        let mut v = VersionJson {
            id: "1.21.1".into(),
            ..Default::default()
        };
        assert!(client_jar_path(&paths, &v).is_ok());
        v.jar = Some("../../x".into());
        assert_eq!(
            client_jar_path(&paths, &v).unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }
}
