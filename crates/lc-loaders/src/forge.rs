//! Forge and NeoForge: download the installer, read `install_profile.json`,
//! fetch libraries, run the client processors with the Java from `InstallCtx`
//! and write a version json that `inheritsFrom` vanilla.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use lc_http::{DownloadReq, Downloader, ResourceKind};
use lc_runtime::ProgressSink;
use lc_types::version::{LoaderKind, LoaderVersion};
use lc_types::{AppError, AppResult, ErrorCode};
use serde::{Deserialize, Serialize};

use crate::contract::{InstallCtx, LoaderInstaller, VersionProfileId};
use crate::fabric::{check_token, profile_path};
use crate::libs::{library_requests, LibFile, Library};
use crate::profile::{
    artifact_path, expand_arg, legacy_version_json, parse_profile, resolve_data_value, AnyProfile,
    InstallProfile, LegacyProfile,
};
use crate::stage::StageSink;
use crate::util::{
    cmp_versions, command, file_sha1, file_sha1_blocking, io_err, join, loader_err,
    maven_metadata_versions, maven_path, parse_json, parse_url, safe_join, sha1_hex, write_atomic,
    write_json,
};

const FORGE_MAVEN: &str = "https://maven.minecraftforge.net";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases";
const FORGE_PROMOTIONS: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const MOJANG_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const MARKER: &str = ".mineup-install.json";

/// Maps a maven-metadata version to a loader version (or drops it).
type VersionFilter = Box<dyn Fn(&str) -> Option<String> + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavor {
    Forge,
    NeoForge,
}

/// Installer for processor-based loaders (Forge, NeoForge).
#[derive(Clone)]
pub struct ForgeLikeInstaller {
    dl: Arc<dyn Downloader>,
    flavor: Flavor,
}

/// Maven coordinates of one installer build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Coords {
    pub repo: &'static str,
    pub group: &'static str,
    pub artifact: &'static str,
    pub version: String,
}

impl Coords {
    fn installer_rel(&self) -> String {
        format!(
            "{}/{}/{}/{}-{}-installer.jar",
            self.group.replace('.', "/"),
            self.artifact,
            self.version,
            self.artifact,
            self.version
        )
    }

    fn installer_url(&self) -> String {
        format!("{}/{}", self.repo, self.installer_rel())
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Marker {
    installer_sha1: String,
}

#[derive(Debug, Deserialize)]
struct Promotions {
    #[serde(default)]
    promos: BTreeMap<String, String>,
}

impl ForgeLikeInstaller {
    pub fn forge(dl: Arc<dyn Downloader>) -> Self {
        Self {
            dl,
            flavor: Flavor::Forge,
        }
    }

    pub fn neoforge(dl: Arc<dyn Downloader>) -> Self {
        Self {
            dl,
            flavor: Flavor::NeoForge,
        }
    }

    /// Maven metadata URL + filter that maps a metadata version to a loader version.
    fn metadata(&self, mc: &str) -> (String, VersionFilter) {
        match self.flavor {
            Flavor::Forge => {
                let prefix = format!("{mc}-");
                (
                    format!("{FORGE_MAVEN}/net/minecraftforge/forge/maven-metadata.xml"),
                    Box::new(move |v: &str| v.strip_prefix(&prefix).map(str::to_owned)),
                )
            }
            Flavor::NeoForge if mc == "1.20.1" => (
                format!("{NEOFORGE_MAVEN}/net/neoforged/forge/maven-metadata.xml"),
                Box::new(|v: &str| v.strip_prefix("1.20.1-").map(str::to_owned)),
            ),
            Flavor::NeoForge => {
                let prefix = neoforge_prefix(mc);
                (
                    format!("{NEOFORGE_MAVEN}/net/neoforged/neoforge/maven-metadata.xml"),
                    Box::new(move |v: &str| {
                        prefix
                            .as_deref()
                            .filter(|p| v.starts_with(p))
                            .map(|_| v.to_owned())
                    }),
                )
            }
        }
    }

    pub(crate) fn coords(&self, mc: &str, loader_version: &str) -> Coords {
        match self.flavor {
            Flavor::Forge => Coords {
                repo: FORGE_MAVEN,
                group: "net.minecraftforge",
                artifact: "forge",
                version: if loader_version.starts_with(&format!("{mc}-")) {
                    loader_version.to_owned()
                } else {
                    format!("{mc}-{loader_version}")
                },
            },
            Flavor::NeoForge if mc == "1.20.1" => Coords {
                repo: NEOFORGE_MAVEN,
                group: "net.neoforged",
                artifact: "forge",
                version: if loader_version.starts_with("1.20.1-") {
                    loader_version.to_owned()
                } else {
                    format!("1.20.1-{loader_version}")
                },
            },
            Flavor::NeoForge => Coords {
                repo: NEOFORGE_MAVEN,
                group: "net.neoforged",
                artifact: "neoforge",
                version: loader_version.to_owned(),
            },
        }
    }

    fn kind_(&self) -> LoaderKind {
        match self.flavor {
            Flavor::Forge => LoaderKind::Forge,
            Flavor::NeoForge => LoaderKind::NeoForge,
        }
    }
}

/// `1.21.1` → `21.1.`, `1.21` → `21.0.`; year-style `26.1` → `26.1.`.
pub(crate) fn neoforge_prefix(mc: &str) -> Option<String> {
    let parts: Vec<&str> = mc.split('.').collect();
    if parts.iter().any(|p| p.parse::<u32>().is_err()) {
        return None;
    }
    match parts.as_slice() {
        ["1", minor] => Some(format!("{minor}.0.")),
        ["1", minor, patch] => Some(format!("{minor}.{patch}.")),
        [major, minor] => Some(format!("{major}.{minor}.")),
        [major, minor, patch] => Some(format!("{major}.{minor}.{patch}.")),
        _ => None,
    }
}

/// Loader versions from metadata, newest first.
pub(crate) fn versions_from_metadata(
    kind: LoaderKind,
    mc: &str,
    xml: &str,
    filter: &dyn Fn(&str) -> Option<String>,
    recommended: Option<&str>,
) -> Vec<LoaderVersion> {
    let mut list: Vec<String> = maven_metadata_versions(xml)
        .iter()
        .filter_map(|v| filter(v))
        .collect();
    list.sort_by(|a, b| cmp_versions(b, a));
    list.dedup();
    let mut out: Vec<LoaderVersion> = list
        .into_iter()
        .map(|v| {
            let l = v.to_ascii_lowercase();
            let stable = !(l.contains("beta") || l.contains("alpha") || l.contains("rc"));
            LoaderVersion {
                loader: kind,
                recommended: recommended == Some(v.as_str()),
                version: v,
                mc: mc.to_owned(),
                stable,
            }
        })
        .collect();
    if !out.iter().any(|v| v.recommended) {
        if let Some(first) = out.iter_mut().find(|v| v.stable) {
            first.recommended = true;
        }
    }
    out
}

#[async_trait]
impl LoaderInstaller for ForgeLikeInstaller {
    fn kind(&self) -> LoaderKind {
        self.kind_()
    }

    async fn versions(&self, mc: &str) -> AppResult<Vec<LoaderVersion>> {
        check_token("mc", mc)?;
        let (url, filter) = self.metadata(mc);
        let xml = self
            .dl
            .get_bytes(&parse_url(&url)?, ResourceKind::Loader)
            .await?;
        let xml = String::from_utf8_lossy(&xml);
        let recommended = if self.flavor == Flavor::Forge {
            match self
                .dl
                .get_bytes(&parse_url(FORGE_PROMOTIONS)?, ResourceKind::Loader)
                .await
            {
                Ok(b) => serde_json::from_slice::<Promotions>(&b)
                    .ok()
                    .and_then(|p| p.promos.get(&format!("{mc}-recommended")).cloned()),
                Err(_) => None,
            }
        } else {
            None
        };
        Ok(versions_from_metadata(
            self.kind_(),
            mc,
            &xml,
            &*filter,
            recommended.as_deref(),
        ))
    }

    async fn install(
        &self,
        ctx: &InstallCtx,
        mc: &str,
        loader_version: &str,
        sink: &dyn ProgressSink,
    ) -> AppResult<VersionProfileId> {
        check_token("mc", mc)?;
        check_token("loader", loader_version)?;
        let coords = self.coords(mc, loader_version);
        sink.log(&format!(
            "installing {}:{}",
            coords.artifact, coords.version
        ));
        let installer = fetch_installer(ctx, &coords, sink).await?;
        let installer_sha1 = file_sha1(&installer)
            .await?
            .ok_or_else(|| loader_err("installer vanished"))?;
        let inst = installer.clone();
        let (profile, version_bytes) =
            join(tokio::task::spawn_blocking(move || read_installer(&inst)).await)?;
        sink.check_cancelled()?;
        match profile {
            AnyProfile::Legacy(p) => install_legacy(ctx, mc, &installer, &p, sink).await,
            AnyProfile::Modern(p) => {
                let version_bytes =
                    version_bytes.ok_or_else(|| loader_err("installer has no version json"))?;
                install_modern(
                    ctx,
                    mc,
                    &installer,
                    &installer_sha1,
                    &p,
                    &version_bytes,
                    sink,
                )
                .await
            }
        }
    }
}

/// Downloads the installer into `libraries/` (maven layout), verified by the
/// `.sha1` published next to it. Offline: an existing installer is reused.
async fn fetch_installer(
    ctx: &InstallCtx,
    coords: &Coords,
    sink: &dyn ProgressSink,
) -> AppResult<PathBuf> {
    let dest = safe_join(&ctx.paths.libraries(), &coords.installer_rel())?;
    let url = coords.installer_url();
    let sha1 = match ctx
        .downloader
        .get_bytes(&parse_url(&format!("{url}.sha1"))?, ResourceKind::Loader)
        .await
    {
        Ok(b) => Some(parse_sha1_file(&b).ok_or_else(|| {
            AppError::new(ErrorCode::HashMismatch, "installer .sha1 is not a SHA-1")
                .detail("url", format!("{url}.sha1"))
        })?),
        Err(e) if e.code == ErrorCode::NotFound => {
            return Err(AppError::new(ErrorCode::NotFound, "loader build not found")
                .detail("url", url.as_str()));
        }
        Err(e) => {
            if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
                return Ok(dest);
            }
            return Err(e);
        }
    };
    let req = DownloadReq {
        urls: vec![parse_url(&url)?],
        sha1,
        size: None,
        dest: dest.clone(),
        kind: ResourceKind::Loader,
    };
    let staged = StageSink::new(sink, "stages.loaderInstaller");
    ctx.downloader.fetch_many(vec![req], &staged).await?;
    Ok(dest)
}

/// First token of a `.sha1` sidecar if it is a 40-char hex digest.
pub(crate) fn parse_sha1_file(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let s = text.split_whitespace().next()?.to_ascii_lowercase();
    (s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())).then_some(s)
}

fn open_zip(path: &Path) -> AppResult<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path).map_err(|e| io_err(path, e))?;
    zip::ZipArchive::new(f).map_err(|e| {
        loader_err(format!("bad installer archive: {e}")).detail("path", path.to_string_lossy())
    })
}

const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BUNDLED_BYTES: u64 = 256 * 1024 * 1024;

fn zip_read(
    zip: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
    limit: u64,
) -> AppResult<Option<Vec<u8>>> {
    let name = name.trim_start_matches('/');
    let entry = match zip.by_name(name) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(loader_err(format!("installer read failed: {e}"))),
    };
    if entry.size() > limit {
        return Err(loader_err("installer entry is too large").detail("entry", name));
    }
    let mut buf = Vec::new();
    entry
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| loader_err(format!("installer read failed: {e}")))?;
    if buf.len() as u64 > limit {
        return Err(loader_err("installer entry is too large").detail("entry", name));
    }
    Ok(Some(buf))
}

/// `install_profile.json` and (modern) the version json bytes.
fn read_installer(path: &Path) -> AppResult<(AnyProfile, Option<Vec<u8>>)> {
    let mut zip = open_zip(path)?;
    let bytes = zip_read(&mut zip, "install_profile.json", MAX_METADATA_BYTES)?
        .ok_or_else(|| loader_err("installer has no install_profile.json"))?;
    let profile = parse_profile(&bytes)?;
    let version = match &profile {
        AnyProfile::Modern(p) => zip_read(&mut zip, &p.json, MAX_METADATA_BYTES)?,
        AnyProfile::Legacy(_) => None,
    };
    Ok((profile, version))
}

/// Extracts `maven/<path>` entries bundled in the installer for the given
/// libraries; returns the libraries that still need a download.
fn extract_bundled(
    installer: &Path,
    libs: &[LibFile],
    libraries: &Path,
) -> AppResult<Vec<LibFile>> {
    let mut zip = open_zip(installer)?;
    let mut rest = Vec::new();
    for lib in libs {
        let dest = lib.local(libraries)?;
        let have = file_sha1_blocking(&dest)?;
        let ok = match (&have, &lib.sha1) {
            (Some(h), Some(want)) => h.eq_ignore_ascii_case(want),
            (Some(_), None) => lib.url.is_none(),
            (None, _) => false,
        };
        if ok {
            continue;
        }
        match zip_read(&mut zip, &format!("maven/{}", lib.path), MAX_BUNDLED_BYTES)? {
            Some(bytes) => {
                if let Some(want) = &lib.sha1 {
                    let got = sha1_hex(&bytes);
                    if !got.eq_ignore_ascii_case(want) {
                        return Err(AppError::new(
                            ErrorCode::HashMismatch,
                            "bundled library hash mismatch",
                        )
                        .detail("path", lib.path.as_str())
                        .detail("expected", want.as_str())
                        .detail("actual", got));
                    }
                }
                write_blocking(&dest, &bytes)?;
            }
            None => rest.push(lib.clone()),
        }
    }
    Ok(rest)
}

fn write_blocking(dest: &Path, bytes: &[u8]) -> AppResult<()> {
    crate::util::ensure_no_links(dest)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    crate::util::ensure_no_links(&tmp)?;
    std::fs::write(&tmp, bytes).map_err(|e| io_err(&tmp, e))?;
    std::fs::rename(&tmp, dest).map_err(|e| io_err(dest, e))
}

#[derive(Debug, Deserialize)]
struct Manifest {
    versions: Vec<ManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    id: String,
    url: String,
    #[serde(default)]
    sha1: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VanillaDownloads {
    #[serde(default)]
    downloads: BTreeMap<String, VanillaArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
struct VanillaArtifact {
    url: String,
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

/// Vanilla json `downloads` (fetching the json via Mojang's manifest when core
/// has not stored it yet).
async fn vanilla_downloads(
    ctx: &InstallCtx,
    mc: &str,
    sink: &dyn ProgressSink,
) -> AppResult<BTreeMap<String, VanillaArtifact>> {
    let path = profile_path(ctx, mc)?;
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        let bytes = ctx
            .downloader
            .get_bytes(&parse_url(MOJANG_MANIFEST)?, ResourceKind::VersionMeta)
            .await?;
        let m: Manifest = parse_json(&bytes, "version manifest")?;
        let e = m.versions.into_iter().find(|v| v.id == mc).ok_or_else(|| {
            AppError::new(ErrorCode::NotFound, "unknown Minecraft version").detail("mc", mc)
        })?;
        let req = DownloadReq {
            urls: vec![parse_url(&e.url)?],
            sha1: e.sha1,
            size: None,
            dest: path.clone(),
            kind: ResourceKind::VersionMeta,
        };
        ctx.downloader
            .fetch_many(vec![req], &StageSink::new(sink, "stages.versionMeta"))
            .await?;
    }
    let bytes = tokio::fs::read(&path).await.map_err(|e| io_err(&path, e))?;
    let v: VanillaDownloads = parse_json(&bytes, "vanilla version json")?;
    Ok(v.downloads)
}

fn artifact_req(a: &VanillaArtifact, dest: PathBuf, kind: ResourceKind) -> AppResult<DownloadReq> {
    Ok(DownloadReq {
        urls: vec![parse_url(&a.url)?],
        sha1: a.sha1.clone(),
        size: a.size,
        dest,
        kind,
    })
}

async fn read_marker(path: &Path) -> Option<Marker> {
    let bytes = tokio::fs::read(path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[allow(clippy::too_many_arguments)]
async fn install_modern(
    ctx: &InstallCtx,
    mc: &str,
    installer: &Path,
    installer_sha1: &str,
    profile: &InstallProfile,
    version_bytes: &[u8],
    sink: &dyn ProgressSink,
) -> AppResult<VersionProfileId> {
    if profile.minecraft != mc {
        return Err(loader_err("installer targets another Minecraft version")
            .detail("expected", mc)
            .detail("actual", profile.minecraft.as_str()));
    }
    let mut version: serde_json::Value = parse_json(version_bytes, "loader version json")?;
    let id = version
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| loader_err("version json has no id"))?;
    check_token("profile id", &id)?;
    let version_libs: Vec<Library> = version
        .get("libraries")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| loader_err(format!("bad version libraries: {e}")))?
        .unwrap_or_default();
    let libraries = ctx.paths.libraries();

    // 1. Libraries: bundled ones from the installer, the rest via Downloader.
    let all: Vec<Library> = profile
        .libraries
        .iter()
        .chain(version_libs.iter())
        .cloned()
        .collect();
    let files: Vec<LibFile> = all.iter().filter_map(Library::file).collect();
    let (inst, libs_dir) = (installer.to_path_buf(), libraries.clone());
    let missing =
        join(tokio::task::spawn_blocking(move || extract_bundled(&inst, &files, &libs_dir)).await)?;
    let to_fetch: Vec<Library> = all
        .into_iter()
        .filter(|l| {
            l.file()
                .map(|f| f.url.is_some() && missing.iter().any(|m| m.path == f.path))
                .unwrap_or(false)
        })
        .collect();
    let reqs = library_requests(&to_fetch, &libraries)?;
    ctx.downloader
        .fetch_many(reqs, &StageSink::new(sink, "stages.loaderLibraries"))
        .await?;
    sink.check_cancelled()?;

    // 2. Vanilla client jar (processor input) + its copy used by the launch.
    let downloads = vanilla_downloads(ctx, mc, sink).await?;
    let client = downloads
        .get("client")
        .ok_or_else(|| loader_err("vanilla json has no client download"))?;
    let vanilla_jar = safe_join(&ctx.paths.versions(), &format!("{mc}/{mc}.jar"))?;
    ctx.downloader
        .fetch_many(
            vec![artifact_req(
                client,
                vanilla_jar.clone(),
                ResourceKind::ClientJar,
            )?],
            &StageSink::new(sink, "stages.client"),
        )
        .await?;

    // 3. Processors (skipped when a previous install of the same installer succeeded).
    let marker_path = safe_join(&ctx.paths.versions(), &format!("{id}/{MARKER}"))?;
    let json_path = profile_path(ctx, &id)?;
    let marker_ok = read_marker(&marker_path).await
        == Some(Marker {
            installer_sha1: installer_sha1.to_owned(),
        })
        && tokio::fs::try_exists(&json_path).await.unwrap_or(false);
    let work = safe_join(&ctx.paths.cache(), &format!("loaders/{id}"))?;
    let data = build_data(ctx, mc, installer, profile, &vanilla_jar, &work).await?;
    let processors: Vec<_> = profile
        .processors
        .iter()
        .filter(|p| p.runs_on_client())
        .collect();
    let all_outputs_ok = outputs_valid(&processors, &data, &libraries).await?;
    if !(marker_ok && all_outputs_ok) {
        let total = processors.len() as u64;
        for (i, p) in processors.iter().enumerate() {
            sink.check_cancelled()?;
            sink.progress(i as u64, Some(total), "stages.loaderProcessors");
            if let Some(out) = p.mojmaps_output() {
                let dest = PathBuf::from(expand_arg(out, &data, &libraries)?);
                let maps = downloads
                    .get("client_mappings")
                    .ok_or_else(|| loader_err("vanilla json has no client_mappings"))?;
                ctx.downloader
                    .fetch_many(
                        vec![artifact_req(maps, dest, ResourceKind::Library)?],
                        &StageSink::new(sink, "stages.loaderLibraries"),
                    )
                    .await?;
                continue;
            }
            if processor_outputs_ok(p, &data, &libraries).await? && !p.outputs.is_empty() {
                continue;
            }
            run_processor(ctx, p, &data, &libraries, &work, sink).await?;
            if !processor_outputs_ok(p, &data, &libraries).await? {
                return Err(loader_err("processor output hash mismatch").detail("jar", &p.jar));
            }
        }
        sink.progress(total, Some(total), "stages.loaderProcessors");
    }

    // 4. Version json. `jar = id` makes core put a copy of the vanilla jar at
    // `versions/<id>/<id>.jar`, which Forge's `-DignoreList=${version_name}.jar`
    // expects (otherwise the vanilla jar ends up as a module and clashes).
    if let Some(obj) = version.as_object_mut() {
        obj.insert("inheritsFrom".into(), mc.into());
        obj.insert("jar".into(), id.clone().into());
    }
    let own_jar = safe_join(&ctx.paths.versions(), &format!("{id}/{id}.jar"))?;
    if file_sha1(&own_jar).await? != file_sha1(&vanilla_jar).await? {
        if let Some(parent) = own_jar.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| io_err(parent, e))?;
        }
        tokio::fs::copy(&vanilla_jar, &own_jar)
            .await
            .map_err(|e| io_err(&own_jar, e))?;
    }
    write_json(&json_path, &version).await?;
    write_json(
        &marker_path,
        &Marker {
            installer_sha1: installer_sha1.to_owned(),
        },
    )
    .await?;
    let _ = tokio::fs::remove_dir_all(&work).await;
    Ok(id)
}

/// `data` map of the profile plus the installer's built-in variables.
async fn build_data(
    ctx: &InstallCtx,
    mc: &str,
    installer: &Path,
    profile: &InstallProfile,
    vanilla_jar: &Path,
    work: &Path,
) -> AppResult<BTreeMap<String, String>> {
    let libraries = ctx.paths.libraries();
    let (inst, work, entries) = (
        installer.to_path_buf(),
        work.to_path_buf(),
        profile.data.clone(),
    );
    let mut data = join(
        tokio::task::spawn_blocking(move || -> AppResult<BTreeMap<String, String>> {
            let mut zip = open_zip(&inst)?;
            let mut extract = |p: &str| -> AppResult<PathBuf> {
                let dest = safe_join(&work, p.trim_start_matches('/'))?;
                let bytes = zip_read(&mut zip, p, MAX_BUNDLED_BYTES)?
                    .ok_or_else(|| loader_err("installer entry missing").detail("entry", p))?;
                write_blocking(&dest, &bytes)?;
                Ok(dest)
            };
            let mut out = BTreeMap::new();
            for (k, v) in &entries {
                out.insert(
                    k.clone(),
                    resolve_data_value(&v.client, &libraries, &mut extract)?,
                );
            }
            Ok(out)
        })
        .await,
    )?;
    let lossy = |p: &Path| p.to_string_lossy().into_owned();
    data.insert("SIDE".into(), "client".into());
    data.insert("MINECRAFT_JAR".into(), lossy(vanilla_jar));
    data.insert("MINECRAFT_VERSION".into(), mc.to_owned());
    data.insert("ROOT".into(), lossy(ctx.paths.root()));
    data.insert("INSTALLER".into(), lossy(installer));
    data.insert("LIBRARY_DIR".into(), lossy(&ctx.paths.libraries()));
    Ok(data)
}

async fn processor_outputs_ok(
    p: &crate::profile::Processor,
    data: &BTreeMap<String, String>,
    libraries: &Path,
) -> AppResult<bool> {
    for (file, hash) in &p.outputs {
        let path = PathBuf::from(expand_arg(file, data, libraries)?);
        let want = expand_arg(hash, data, libraries)?;
        let want = want.trim_matches('\'');
        match file_sha1(&path).await? {
            Some(got) if got.eq_ignore_ascii_case(want) => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}

async fn outputs_valid(
    ps: &[&crate::profile::Processor],
    data: &BTreeMap<String, String>,
    libraries: &Path,
) -> AppResult<bool> {
    for p in ps {
        if !processor_outputs_ok(p, data, libraries).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn main_class(jar: &Path) -> AppResult<String> {
    let mut zip = open_zip(jar)?;
    let manifest = zip_read(&mut zip, "META-INF/MANIFEST.MF", MAX_METADATA_BYTES)?
        .ok_or_else(|| loader_err("processor jar has no manifest"))?;
    let text = String::from_utf8_lossy(&manifest);
    text.lines()
        .find_map(|l| l.strip_prefix("Main-Class:"))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| loader_err("processor jar has no Main-Class"))
}

async fn run_processor(
    ctx: &InstallCtx,
    p: &crate::profile::Processor,
    data: &BTreeMap<String, String>,
    libraries: &Path,
    work: &Path,
    sink: &dyn ProgressSink,
) -> AppResult<()> {
    let jar = artifact_path(libraries, &p.jar)?;
    let jar2 = jar.clone();
    let main = join(tokio::task::spawn_blocking(move || main_class(&jar2)).await)?;
    let mut cp = vec![jar];
    for c in &p.classpath {
        cp.push(artifact_path(libraries, c)?);
    }
    let sep = if cfg!(windows) { ";" } else { ":" };
    let cp = cp
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(sep);
    let mut args = Vec::with_capacity(p.args.len());
    for a in &p.args {
        args.push(expand_arg(a, data, libraries)?);
    }
    tokio::fs::create_dir_all(work)
        .await
        .map_err(|e| io_err(work, e))?;
    sink.log(&format!("processor {} ({main})", p.jar));
    let mut cmd = command(&ctx.java);
    cmd.arg("-cp")
        .arg(&cp)
        .arg(&main)
        .args(&args)
        .current_dir(work)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| {
        AppError::new(ErrorCode::JavaMissing, format!("failed to start java: {e}"))
            .detail("path", ctx.java.to_string_lossy())
    })?;
    let out = tokio::select! {
        r = child.wait_with_output() => r.map_err(|e| loader_err(format!("processor failed: {e}")))?,
        _ = sink.cancel_token().cancelled() => {
            return Err(AppError::new(ErrorCode::Cancelled, "cancelled by user"));
        }
    };
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = text.lines().rev().take(30).collect();
        for l in tail.iter().rev() {
            sink.log(l);
        }
        return Err(loader_err("forge processor failed")
            .detail("jar", p.jar.as_str())
            .detail("exit", out.status.code().unwrap_or(-1).to_string()));
    }
    Ok(())
}

async fn install_legacy(
    ctx: &InstallCtx,
    mc: &str,
    installer: &Path,
    p: &LegacyProfile,
    sink: &dyn ProgressSink,
) -> AppResult<VersionProfileId> {
    if p.install.minecraft != mc {
        return Err(loader_err("installer targets another Minecraft version")
            .detail("expected", mc)
            .detail("actual", p.install.minecraft.as_str()));
    }
    let rel = maven_path(&p.install.path).ok_or_else(|| loader_err("bad universal coords"))?;
    let dest = safe_join(&ctx.paths.libraries(), &rel)?;
    let (inst, file, d) = (
        installer.to_path_buf(),
        p.install.file_path.clone(),
        dest.clone(),
    );
    let sha1 = join(
        tokio::task::spawn_blocking(move || -> AppResult<String> {
            let mut zip = open_zip(&inst)?;
            let bytes = zip_read(&mut zip, &file, MAX_BUNDLED_BYTES)?
                .ok_or_else(|| loader_err("installer has no universal jar"))?;
            let sha = sha1_hex(&bytes);
            if file_sha1_blocking(&d)?.as_deref() != Some(sha.as_str()) {
                write_blocking(&d, &bytes)?;
            }
            Ok(sha)
        })
        .await,
    )?;
    let (id, json) = legacy_version_json(p, &sha1)?;
    check_token("profile id", &id)?;
    let libs: Vec<Library> = json
        .get("libraries")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| loader_err(format!("bad legacy libraries: {e}")))?
        .unwrap_or_default();
    let reqs = library_requests(&libs, &ctx.paths.libraries())?;
    ctx.downloader
        .fetch_many(reqs, &StageSink::new(sink, "stages.loaderLibraries"))
        .await?;
    let path = profile_path(ctx, &id)?;
    write_atomic(
        &path,
        &serde_json::to_vec_pretty(&json).map_err(|e| AppError::internal(e.to_string()))?,
    )
    .await?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_reads_reject_attacker_declared_allocation_sizes() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bomb.jar");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file(
            "install_profile.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes[central + 24..central + 28].copy_from_slice(&(u32::MAX - 1).to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let error = zip_read(
            &mut open_zip(&path).unwrap(),
            "install_profile.json",
            MAX_METADATA_BYTES,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::LoaderInstall);
        assert!(error.message.contains("too large"));
    }

    #[test]
    fn zip_reads_enforce_actual_payload_limit() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.jar");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("data", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"payload").unwrap();
        zip.finish().unwrap();
        assert!(zip_read(&mut open_zip(&path).unwrap(), "data", 6).is_err());
        assert_eq!(
            zip_read(&mut open_zip(&path).unwrap(), "data", 7)
                .unwrap()
                .unwrap(),
            b"payload"
        );
        let mut bytes = std::fs::read(&path).unwrap();
        let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(zip_read(&mut open_zip(&path).unwrap(), "data", 6).is_err());
    }

    #[test]
    fn sha1_sidecar_parsing() {
        let h = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        assert_eq!(parse_sha1_file(h.as_bytes()).as_deref(), Some(h));
        assert_eq!(
            parse_sha1_file(format!("{}  forge.jar\n", h.to_uppercase()).as_bytes()).as_deref(),
            Some(h)
        );
        assert_eq!(parse_sha1_file(b"<html>blocked</html>"), None);
        assert_eq!(parse_sha1_file(b""), None);
    }

    #[test]
    fn neoforge_prefixes() {
        assert_eq!(neoforge_prefix("1.21.1").as_deref(), Some("21.1."));
        assert_eq!(neoforge_prefix("1.21").as_deref(), Some("21.0."));
        assert_eq!(neoforge_prefix("1.20.4").as_deref(), Some("20.4."));
        assert_eq!(neoforge_prefix("24w14a"), None);
    }

    #[test]
    fn coords_and_urls() {
        let f = ForgeLikeInstaller::forge(Arc::new(lc_http::StubDownloader));
        let c = f.coords("1.20.1", "47.3.0");
        assert_eq!(
            c.installer_url(),
            "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.3.0/forge-1.20.1-47.3.0-installer.jar"
        );
        assert_eq!(f.coords("1.20.1", "1.20.1-47.3.0").version, "1.20.1-47.3.0");
        let n = ForgeLikeInstaller::neoforge(Arc::new(lc_http::StubDownloader));
        assert_eq!(
            n.coords("1.21.1", "21.1.77").installer_url(),
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.77/neoforge-21.1.77-installer.jar"
        );
        assert_eq!(n.coords("1.20.1", "47.1.106").artifact, "forge");
    }

    #[test]
    fn forge_versions_sorted_and_recommended() {
        let xml = "<versions><version>1.20.1-47.2.0</version><version>1.20.1-47.10.1</version>\
                   <version>1.20.1-47.3.0</version><version>1.19.2-43.2.0</version></versions>";
        let pre = "1.20.1-".to_owned();
        let filter = move |v: &str| v.strip_prefix(&pre).map(str::to_owned);
        let v = versions_from_metadata(LoaderKind::Forge, "1.20.1", xml, &filter, Some("47.3.0"));
        let names: Vec<_> = v.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(names, vec!["47.10.1", "47.3.0", "47.2.0"]);
        assert!(v[1].recommended && !v[0].recommended);
    }

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            for (name, data) in entries {
                z.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                z.write_all(data).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[tokio::test]
    async fn modern_install_without_processors_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let paths = lc_runtime::Paths::new(dir.path());
        let dl = Arc::new(crate::mock::MockDl::default());
        let bundled = b"bundled-universal".to_vec();
        let remote = b"remote-lib".to_vec();
        let profile = serde_json::json!({
            "spec": 1, "minecraft": "1.20.1", "json": "/version.json", "data": {},
            "processors": [{"sides": ["server"], "jar": "x:y:1", "args": []}],
            "libraries": [
                {"name": "net.minecraftforge:forge:1.20.1-47.3.0:universal",
                 "downloads": {"artifact": {"path": "net/minecraftforge/forge/1.20.1-47.3.0/forge-1.20.1-47.3.0-universal.jar",
                   "url": "", "sha1": sha1_hex(&bundled)}}}
            ]
        });
        let version = serde_json::json!({
            "id": "1.20.1-forge-47.3.0", "inheritsFrom": "1.20.1",
            "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
            "libraries": [{"name": "a.b:c:1", "downloads": {"artifact": {
                "path": "a/b/c/1/c-1.jar", "url": "https://maven.example/a/b/c/1/c-1.jar",
                "sha1": sha1_hex(&remote)}}}]
        });
        let installer = zip_bytes(&[
            ("install_profile.json", profile.to_string().as_bytes()),
            ("version.json", version.to_string().as_bytes()),
            (
                "maven/net/minecraftforge/forge/1.20.1-47.3.0/forge-1.20.1-47.3.0-universal.jar",
                &bundled,
            ),
        ]);
        let base = "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.3.0/forge-1.20.1-47.3.0-installer.jar";
        dl.put(&format!("{base}.sha1"), sha1_hex(&installer));
        dl.put(base, installer);
        dl.put("https://maven.example/a/b/c/1/c-1.jar", remote);
        let client = b"vanilla-client".to_vec();
        dl.put("https://piston-data.mojang.com/client.jar", client.clone());
        let vanilla = serde_json::json!({"id": "1.20.1", "downloads": {"client": {
            "url": "https://piston-data.mojang.com/client.jar", "sha1": sha1_hex(&client)}}});
        let vdir = paths.versions().join("1.20.1");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("1.20.1.json"), vanilla.to_string()).unwrap();

        let ctx = InstallCtx {
            paths: paths.clone(),
            downloader: dl.clone(),
            java: PathBuf::from("java"),
        };
        let inst = ForgeLikeInstaller::forge(dl.clone());
        let sink = lc_runtime::NullSink::default();
        let id = inst.install(&ctx, "1.20.1", "47.3.0", &sink).await.unwrap();
        assert_eq!(id, "1.20.1-forge-47.3.0");
        let json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(paths.versions().join(&id).join(format!("{id}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(json["inheritsFrom"], "1.20.1");
        assert_eq!(json["jar"], id.as_str());
        assert_eq!(
            std::fs::read(paths.versions().join(&id).join(format!("{id}.jar"))).unwrap(),
            client
        );
        assert!(paths
            .libraries()
            .join("net/minecraftforge/forge/1.20.1-47.3.0/forge-1.20.1-47.3.0-universal.jar")
            .exists());
        assert!(paths.libraries().join("a/b/c/1/c-1.jar").exists());

        // Re-run: same result (repair path).
        let again = inst.install(&ctx, "1.20.1", "47.3.0", &sink).await.unwrap();
        assert_eq!(again, id);
    }

    #[tokio::test]
    async fn wrong_minecraft_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let paths = lc_runtime::Paths::new(dir.path());
        let dl = Arc::new(crate::mock::MockDl::default());
        let profile = serde_json::json!({"minecraft": "1.19.2", "json": "/version.json"});
        let version = serde_json::json!({"id": "x"});
        let installer = zip_bytes(&[
            ("install_profile.json", profile.to_string().as_bytes()),
            ("version.json", version.to_string().as_bytes()),
        ]);
        let base = "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-1.0/forge-1.20.1-1.0-installer.jar";
        dl.put(&format!("{base}.sha1"), sha1_hex(&installer));
        dl.put(base, installer);
        let ctx = InstallCtx {
            paths,
            downloader: dl.clone(),
            java: PathBuf::from("java"),
        };
        let err = ForgeLikeInstaller::forge(dl)
            .install(&ctx, "1.20.1", "1.0", &lc_runtime::NullSink::default())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::LoaderInstall);
    }

    #[test]
    fn neoforge_versions_filtered() {
        let xml = "<version>21.1.77</version><version>21.0.5-beta</version>\
                   <version>21.1.9</version><version>20.4.237</version>";
        let pre = neoforge_prefix("1.21.1").unwrap();
        let filter = move |v: &str| v.starts_with(&pre).then(|| v.to_owned());
        let v = versions_from_metadata(LoaderKind::NeoForge, "1.21.1", xml, &filter, None);
        let names: Vec<_> = v.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(names, vec!["21.1.77", "21.1.9"]);
        assert!(v[0].recommended);
    }
}
