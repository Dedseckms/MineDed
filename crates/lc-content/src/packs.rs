//! Modpack import/export: Modrinth `.mrpack`, CurseForge zip, launcher zip.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use lc_http::{DownloadReq, ResourceKind};
use lc_runtime::ProgressSink;
use lc_types::content::{ContentKind, ContentSourceKind};
use lc_types::instance::{
    Instance, InstancePatch, InstanceSpec, JavaCfg, LoaderRef, MemoryCfg, PackFormat,
};
use lc_types::version::LoaderKind;
use lc_types::{AppError, AppResult, ErrorCode, InstanceId};
use serde::{Deserialize, Serialize};

use crate::contract::PackIo;
use crate::curseforge::no_key_error;
use crate::index::{key, ContentIndex, IndexEntry};
use crate::service::{ContentServices, Inner};
use crate::util::{file_hashes, io_err, join, parse_url, safe_join, CONTENT_DIRS};

pub(crate) const MRPACK_INDEX: &str = "modrinth.index.json";
pub(crate) const CF_MANIFEST: &str = "manifest.json";
pub(crate) const LAUNCHER_MANIFEST: &str = "mineup-pack.json";

/// Hosts `.mrpack` downloads may come from (Modrinth spec + CurseForge CDN).
const ALLOWED_HOSTS: [&str; 6] = [
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
    "edge.forgecdn.net",
    "mediafilez.forgecdn.net",
];

/// Unpacking limits for untrusted archives.
const MAX_ENTRIES: usize = 20_000;
const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// `-XX:` options a foreign pack may set: GC / JIT / memory tuning only.
/// Anything that reads files or runs commands (`Flags`, `VMOptionsFile`,
/// `OnError`, `OnOutOfMemoryError`, `*Command*`, logging to paths, ...) is
/// not listed and therefore rejected.
const XX_ALLOWED: &[&str] = &[
    "AdaptiveSizePolicyWeight",
    "AggressiveOpts",
    "AllocatePrefetchStyle",
    "AlwaysActAsServerClassMachine",
    "AlwaysPreTouch",
    "CICompilerCount",
    "ConcGCThreads",
    "DisableExplicitGC",
    "DoEscapeAnalysis",
    "EliminateLocks",
    "G1ConcRefinementServiceIntervalMillis",
    "G1ConcRSHotCardLimit",
    "G1ConcRSLogCacheSize",
    "G1HeapRegionSize",
    "G1HeapWastePercent",
    "G1MaxNewSizePercent",
    "G1MixedGCCountTarget",
    "G1MixedGCLiveThresholdPercent",
    "G1NewSizePercent",
    "G1RSetUpdatingPauseTimePercent",
    "G1ReservePercent",
    "G1SATBBufferEnqueueingThresholdPercent",
    "GCTimeRatio",
    "InitialRAMPercentage",
    "InitiatingHeapOccupancyPercent",
    "MaxGCPauseMillis",
    "MaxHeapFreeRatio",
    "MaxInlineLevel",
    "MaxMetaspaceSize",
    "MaxRAMPercentage",
    "MaxTenuringThreshold",
    "MetaspaceSize",
    "MinHeapFreeRatio",
    "MinRAMPercentage",
    "NmethodSweepActivity",
    "OmitStackTraceInFastThrow",
    "ParallelGCThreads",
    "ParallelRefProcEnabled",
    "PerfDisableSharedMem",
    "RangeCheckElimination",
    "ReservedCodeCacheSize",
    "SegmentedCodeCache",
    "ShenandoahGCMode",
    "ShowCodeDetailsInExceptionMessages",
    "SoftRefLRUPolicyMSPerMB",
    "SurvivorRatio",
    "ThreadPriorityPolicy",
    "TieredCompilation",
    "TrustFinalNonStaticFields",
    "UnlockExperimentalVMOptions",
    "UseAdaptiveGCBoundary",
    "UseCompressedClassPointers",
    "UseCompressedOops",
    "UseCriticalJavaThreadPriority",
    "UseDynamicNumberOfGCThreads",
    "UseFastUnorderedTimeStamps",
    "UseFPUForSpilling",
    "UseG1GC",
    "UseLargePages",
    "UseLoopPredicate",
    "UseNUMA",
    "UseParallelGC",
    "UseSerialGC",
    "UseShenandoahGC",
    "UseStringDeduplication",
    "UseTransparentHugePages",
    "UseVectorCmov",
    "UseXMMForArrayCopy",
    "UseZGC",
    "ZCollectionInterval",
    "ZGenerational",
    "ZUncommit",
    "ZUncommitDelay",
];

/// `-D` property prefixes that change class loading, security, native
/// library lookup or logging configuration (log4j can load remote configs).
const D_DENIED: &[&str] = &[
    "java.",
    "javax.",
    "jdk.",
    "sun.",
    "com.sun.",
    "log4j",
    "org.apache.logging",
    "org.lwjgl.",
    "jna.",
    "jnidispatch.",
    "fml.",
    "forge",
    "mixin.",
    "fabric.",
    "loader.",
    "minecraft.",
];

/// Value of a sized option: digits with an optional unit (`512m`, `4G`, `80`).
fn is_size(v: &str) -> bool {
    let digits = v.trim_end_matches(['k', 'K', 'm', 'M', 'g', 'G']);
    !digits.is_empty() && digits.len() + 1 >= v.len() && digits.bytes().all(|b| b.is_ascii_digit())
}

fn safe_xx(opt: &str) -> bool {
    if let Some(name) = opt.strip_prefix('+').or_else(|| opt.strip_prefix('-')) {
        return XX_ALLOWED.contains(&name);
    }
    let Some((name, value)) = opt.split_once('=') else {
        return false;
    };
    if !XX_ALLOWED.contains(&name) {
        return false;
    }
    if name == "ShenandoahGCMode" {
        return matches!(value, "satb" | "iu" | "passive" | "generational");
    }
    is_size(value)
}

fn safe_d(prop: &str) -> bool {
    let name = prop.split('=').next().unwrap_or("");
    let lower = name.to_ascii_lowercase();
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !D_DENIED.iter().any(|p| lower.starts_with(p))
}

/// JVM args accepted from a foreign launcher zip: heap/stack sizes,
/// allowlisted `-XX:` tuning flags and harmless `-D` properties.
/// Everything else is dropped and logged.
pub(crate) fn safe_jvm_args(args: &[String], sink: &dyn ProgressSink) -> Vec<String> {
    args.iter()
        .filter(|a| {
            let ok = if let Some(xx) = a.strip_prefix("-XX:") {
                safe_xx(xx)
            } else if let Some(d) = a.strip_prefix("-D") {
                safe_d(d)
            } else {
                ["-Xms", "-Xmx", "-Xss", "-Xmn"]
                    .iter()
                    .any(|p| a.strip_prefix(p).is_some_and(is_size))
            };
            if !ok {
                sink.log(&format!("import: dropped JVM argument {a}"));
            }
            ok
        })
        .cloned()
        .collect()
}

/// Game-dir folders never exported.
const EXPORT_SKIP: [&str; 9] = [
    "logs",
    "crash-reports",
    "screenshots",
    "saves",
    ".fabric",
    ".mixin.out",
    "downloads",
    "debug",
    ".cache",
];
const EXPORT_SKIP_FILES: [&str; 3] = ["usercache.json", "usernamecache.json", "launcher_log.txt"];

// ---------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MrpackIndex {
    pub format_version: u32,
    pub game: String,
    pub version_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub files: Vec<MrpackFile>,
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MrpackFile {
    pub path: String,
    pub hashes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    pub downloads: Vec<String>,
    #[serde(default)]
    pub file_size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfManifest {
    pub minecraft: CfMinecraft,
    #[serde(default = "manifest_type")]
    pub manifest_type: String,
    #[serde(default = "one")]
    pub manifest_version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub files: Vec<CfManifestFile>,
    #[serde(default = "overrides")]
    pub overrides: String,
}

fn manifest_type() -> String {
    "minecraftModpack".into()
}
fn one() -> u32 {
    1
}
fn overrides() -> String {
    "overrides".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfMinecraft {
    pub version: String,
    #[serde(default)]
    pub mod_loaders: Vec<CfModLoader>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CfModLoader {
    pub id: String,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CfManifestFile {
    #[serde(rename = "projectID")]
    pub project_id: u64,
    #[serde(rename = "fileID")]
    pub file_id: u64,
    #[serde(default = "yes")]
    pub required: bool,
}

fn yes() -> bool {
    true
}

/// Launcher zip header; the game dir lives under `game/`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherManifest {
    pub format: u32,
    pub name: String,
    pub mc: String,
    pub loader: LoaderRef,
    pub java: JavaCfg,
    pub memory: MemoryCfg,
    pub jvm_args: Vec<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Detected {
    Mrpack,
    CurseForge,
    Launcher,
}

pub(crate) fn detect(names: &[String]) -> Option<Detected> {
    let has = |n: &str| names.iter().any(|x| x == n);
    if has(MRPACK_INDEX) {
        Some(Detected::Mrpack)
    } else if has(LAUNCHER_MANIFEST) {
        Some(Detected::Launcher)
    } else if has(CF_MANIFEST) {
        Some(Detected::CurseForge)
    } else {
        None
    }
}

fn bad_pack(msg: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidInput, msg)
}

/// Minecraft version + loader from `.mrpack` dependencies.
pub(crate) fn mrpack_target(idx: &MrpackIndex) -> AppResult<(String, LoaderRef)> {
    let mc = idx
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(|| bad_pack("mrpack has no minecraft dependency"))?;
    let pairs = [
        ("fabric-loader", LoaderKind::Fabric),
        ("quilt-loader", LoaderKind::Quilt),
        ("forge", LoaderKind::Forge),
        ("neoforge", LoaderKind::NeoForge),
    ];
    let loader = pairs
        .iter()
        .find_map(|(k, kind)| {
            idx.dependencies.get(*k).map(|v| LoaderRef {
                kind: *kind,
                version: Some(v.clone()),
            })
        })
        .unwrap_or(LoaderRef {
            kind: LoaderKind::Vanilla,
            version: None,
        });
    Ok((mc, loader))
}

pub(crate) fn mrpack_loader_key(kind: LoaderKind) -> Option<&'static str> {
    match kind {
        LoaderKind::Fabric => Some("fabric-loader"),
        LoaderKind::Quilt => Some("quilt-loader"),
        LoaderKind::Forge => Some("forge"),
        LoaderKind::NeoForge => Some("neoforge"),
        LoaderKind::Vanilla => None,
    }
}

/// `forge-47.2.0` / `neoforge-21.1.77` / `fabric-0.16.9` / `quilt-0.26.0`.
pub(crate) fn cf_target(m: &CfManifest) -> AppResult<(String, LoaderRef)> {
    let mc = m.minecraft.version.clone();
    let primary = m
        .minecraft
        .mod_loaders
        .iter()
        .find(|l| l.primary)
        .or_else(|| m.minecraft.mod_loaders.first());
    let Some(l) = primary else {
        return Ok((
            mc,
            LoaderRef {
                kind: LoaderKind::Vanilla,
                version: None,
            },
        ));
    };
    let (name, ver) =
        l.id.split_once('-')
            .ok_or_else(|| bad_pack("bad modLoader id"))?;
    let kind = match name {
        "forge" => LoaderKind::Forge,
        "neoforge" => LoaderKind::NeoForge,
        "fabric" => LoaderKind::Fabric,
        "quilt" => LoaderKind::Quilt,
        _ => return Err(bad_pack(format!("unsupported loader {name}"))),
    };
    let ver = ver.strip_prefix(&format!("{mc}-")).unwrap_or(ver);
    Ok((
        mc,
        LoaderRef {
            kind,
            version: Some(ver.to_owned()),
        },
    ))
}

pub(crate) fn cf_loader_id(l: &LoaderRef) -> Option<String> {
    let v = l.version.as_deref()?;
    let name = match l.kind {
        LoaderKind::Forge => "forge",
        LoaderKind::NeoForge => "neoforge",
        LoaderKind::Fabric => "fabric",
        LoaderKind::Quilt => "quilt",
        LoaderKind::Vanilla => return None,
    };
    Some(format!("{name}-{v}"))
}

/// Download URL allowed for `.mrpack` files.
pub(crate) fn allowed_download(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .filter(|u| u.scheme() == "https")
        .and_then(|u| u.host_str().map(str::to_owned))
        .is_some_and(|h| ALLOWED_HOSTS.contains(&h.as_str()))
}

/// `https://cdn.modrinth.com/data/<project>/versions/<version>/<file>` → ids.
pub(crate) fn modrinth_ids_from_url(url: &str) -> Option<(String, String)> {
    let u = url::Url::parse(url).ok()?;
    if u.host_str()? != "cdn.modrinth.com" {
        return None;
    }
    let segs: Vec<&str> = u.path_segments()?.collect();
    match segs.as_slice() {
        ["data", project, "versions", version, _file] => {
            Some(((*project).to_owned(), (*version).to_owned()))
        }
        _ => None,
    }
}

fn kind_of_path(rel: &str) -> Option<(&'static str, ContentKind)> {
    let (dir, rest) = rel.split_once('/')?;
    if rest.contains('/') {
        return None;
    }
    CONTENT_DIRS
        .iter()
        .find(|(d, _)| *d == dir)
        .map(|(d, k)| (*d, *k))
}

// ---------------------------------------------------------------------------
// Zip helpers (blocking)
// ---------------------------------------------------------------------------

fn zip_err(e: zip::result::ZipError) -> AppError {
    AppError::new(ErrorCode::InvalidInput, format!("bad archive: {e}"))
}

fn open_zip(path: &Path) -> AppResult<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path).map_err(|e| io_err(path, e))?;
    zip::ZipArchive::new(f).map_err(zip_err)
}

fn read_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> AppResult<Vec<u8>> {
    let mut e = zip.by_name(name).map_err(zip_err)?;
    let mut buf = Vec::new();
    e.read_to_end(&mut buf)
        .map_err(|e| bad_pack(format!("read {name}: {e}")))?;
    Ok(buf)
}

/// Extracts entries under `prefix/` into `dest` (paths sanitized).
fn extract_prefix(path: &Path, prefix: &str, dest: &Path) -> AppResult<usize> {
    let mut zip = open_zip(path)?;
    let prefix = format!("{}/", prefix.trim_end_matches('/'));
    if zip.len() > MAX_ENTRIES {
        return Err(bad_pack("archive has too many entries"));
    }
    let mut n = 0;
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).map_err(zip_err)?;
        let name = e.name().to_owned();
        let Some(rel) = name.strip_prefix(&prefix) else {
            continue;
        };
        if rel.is_empty() || e.is_dir() {
            continue;
        }
        let out = safe_join(dest, rel)?;
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| io_err(&out, e))?;
        // Real bytes are counted (declared sizes can lie).
        let left = MAX_TOTAL_BYTES - total;
        let copied =
            std::io::copy(&mut (&mut e).take(left + 1), &mut f).map_err(|e| io_err(&out, e))?;
        total += copied;
        if copied > left {
            drop(f);
            let _ = std::fs::remove_file(&out);
            return Err(bad_pack("archive is too large when unpacked"));
        }
        n += 1;
    }
    Ok(n)
}

fn entry_names(path: &Path) -> AppResult<Vec<String>> {
    let zip = open_zip(path)?;
    Ok(zip.file_names().map(str::to_owned).collect())
}

/// Relative (`/`-separated) files under `root`, skipping export junk.
fn walk(root: &Path, skip_saves: bool) -> AppResult<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io_err(&dir, e)),
        };
        for e in rd {
            let e = e.map_err(|e| io_err(&dir, e))?;
            let name = e.file_name().to_string_lossy().into_owned();
            let r = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let ft = e.file_type().map_err(|err| io_err(&e.path(), err))?;
            if ft.is_dir() {
                if rel.is_empty()
                    && EXPORT_SKIP
                        .iter()
                        .any(|s| *s == name && (skip_saves || *s != "saves"))
                {
                    continue;
                }
                stack.push((e.path(), r));
            } else if ft.is_file() {
                if rel.is_empty() && EXPORT_SKIP_FILES.contains(&name.as_str()) {
                    continue;
                }
                out.push(r);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Writes a zip: `(name, bytes)` entries + files copied from disk.
pub(crate) fn write_zip(
    out: &Path,
    entries: &[(String, Vec<u8>)],
    files: &[(String, PathBuf)],
) -> AppResult<()> {
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    let mut tmp = out.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let f = std::fs::File::create(&tmp).map_err(|e| io_err(&tmp, e))?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let zerr = |e: zip::result::ZipError| AppError::new(ErrorCode::Io, format!("zip: {e}"));
    for (name, bytes) in entries {
        z.start_file(name.as_str(), opts).map_err(zerr)?;
        z.write_all(bytes).map_err(|e| io_err(&tmp, e))?;
    }
    for (name, path) in files {
        z.start_file(name.as_str(), opts).map_err(zerr)?;
        let mut src = std::fs::File::open(path).map_err(|e| io_err(path, e))?;
        std::io::copy(&mut src, &mut z).map_err(|e| io_err(path, e))?;
    }
    z.finish().map_err(zerr)?;
    std::fs::rename(&tmp, out).map_err(|e| io_err(out, e))
}

fn json_bytes<T: Serialize>(v: &T) -> AppResult<Vec<u8>> {
    serde_json::to_vec_pretty(v).map_err(|e| AppError::internal(format!("json encode: {e}")))
}

fn parse<T: for<'de> Deserialize<'de>>(bytes: &[u8], what: &str) -> AppResult<T> {
    serde_json::from_slice(bytes).map_err(|e| bad_pack(format!("bad {what}: {e}")))
}

// ---------------------------------------------------------------------------
// Import / export
// ---------------------------------------------------------------------------

/// Name/icon of a pack installed from a catalogue (override the archive's name).
#[derive(Debug, Clone, Default)]
pub(crate) struct PackMeta {
    pub name: Option<String>,
    pub icon_url: Option<String>,
}

/// Largest icon accepted from a catalogue.
const MAX_ICON_BYTES: usize = 2 * 1024 * 1024;

/// Extension of a PNG/JPEG/GIF/WebP image by its magic bytes.
pub(crate) fn image_ext(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("gif")
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// Moves every file under `src` into `dst`, merging folders and replacing files.
pub(crate) fn move_tree(src: &Path, dst: &Path) -> AppResult<()> {
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((from_dir, to_dir)) = stack.pop() {
        std::fs::create_dir_all(&to_dir).map_err(|e| io_err(&to_dir, e))?;
        let rd = std::fs::read_dir(&from_dir).map_err(|e| io_err(&from_dir, e))?;
        for e in rd {
            let e = e.map_err(|e| io_err(&from_dir, e))?;
            let from = e.path();
            let to = to_dir.join(e.file_name());
            let ft = e.file_type().map_err(|err| io_err(&from, err))?;
            if ft.is_dir() {
                stack.push((from, to));
            } else if ft.is_file() {
                if to.is_dir() {
                    return Err(bad_pack("pack file collides with a folder")
                        .detail("path", to.to_string_lossy()));
                }
                if std::fs::rename(&from, &to).is_err() {
                    std::fs::copy(&from, &to).map_err(|e| io_err(&to, e))?;
                    let _ = std::fs::remove_file(&from);
                }
            }
        }
    }
    Ok(())
}

/// `.mrpack` file entry accepted for download: (entry, url, sha1).
type Planned = (MrpackFile, String, String);

impl Inner {
    async fn create_instance(
        &self,
        name: &str,
        mc: String,
        loader: LoaderRef,
        sink: &dyn ProgressSink,
    ) -> AppResult<Instance> {
        let name = if name.trim().is_empty() {
            format!("{mc} pack")
        } else {
            name.trim().to_owned()
        };
        self.instances
            .create(
                InstanceSpec {
                    name,
                    mc,
                    loader,
                    preset: None,
                    icon: None,
                },
                sink,
            )
            .await
    }

    /// Fresh staging dir for a pack being unpacked (same volume as instances,
    /// so files are moved, not copied).
    async fn staging_dir(&self) -> AppResult<PathBuf> {
        let dir = self
            .paths
            .cache()
            .join("content")
            .join("staging")
            .join(InstanceId::new().to_string());
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| io_err(&dir, e))?;
        Ok(dir)
    }

    /// Creates the instance (installs vanilla + loader), moves the staged game
    /// files into it and sets the icon. A half-made instance is deleted on failure.
    #[allow(clippy::too_many_arguments)]
    async fn finish_pack(
        &self,
        staging: &Path,
        name: &str,
        mc: String,
        loader: LoaderRef,
        cidx: ContentIndex,
        meta: &PackMeta,
        sink: &dyn ProgressSink,
    ) -> AppResult<Instance> {
        sink.check_cancelled()?;
        sink.progress(0, None, "stages.packLoader");
        let name = meta.name.as_deref().unwrap_or(name);
        let inst = self.create_instance(name, mc, loader, sink).await?;
        let adopted = async {
            let (s, g) = (staging.to_path_buf(), self.instances.game_dir(inst.id));
            join(tokio::task::spawn_blocking(move || move_tree(&s, &g)).await)?;
            cidx.save(&self.instance_dir(inst.id)).await?;
            let Some(url) = &meta.icon_url else {
                return Ok(inst.clone());
            };
            match self.fetch_icon(inst.id, url).await {
                Some(icon) => {
                    self.instances
                        .update(
                            inst.id,
                            InstancePatch {
                                icon: Some(icon),
                                ..Default::default()
                            },
                        )
                        .await
                }
                None => {
                    sink.log("modpack icon could not be downloaded");
                    Ok(inst.clone())
                }
            }
        }
        .await;
        if adopted.is_err() {
            let _ = self.instances.delete(inst.id).await;
        }
        adopted
    }

    /// Best effort: catalogue icon saved as `icon.<ext>` in the instance dir.
    /// Icons have no published hash; the size and image format are checked.
    async fn fetch_icon(&self, id: InstanceId, url: &str) -> Option<String> {
        let u = url::Url::parse(url)
            .ok()
            .filter(|u| u.scheme() == "https")?;
        let bytes = self.dl.get_bytes(&u, ResourceKind::Other).await.ok()?;
        if bytes.len() > MAX_ICON_BYTES {
            return None;
        }
        let name = format!("icon.{}", image_ext(&bytes)?);
        let path = self.instance_dir(id).join(&name);
        tokio::fs::write(&path, &bytes).await.ok()?;
        Some(name)
    }

    async fn import_mrpack(
        &self,
        path: &Path,
        meta: &PackMeta,
        sink: &dyn ProgressSink,
    ) -> AppResult<Instance> {
        sink.progress(0, None, "stages.packResolve");
        let p = path.to_path_buf();
        let bytes = join(
            tokio::task::spawn_blocking(move || read_entry(&mut open_zip(&p)?, MRPACK_INDEX)).await,
        )?;
        let idx: MrpackIndex = parse(&bytes, MRPACK_INDEX)?;
        if idx.game != "minecraft" {
            return Err(bad_pack("mrpack is not for minecraft"));
        }
        let (mc, loader) = mrpack_target(&idx)?;
        // Validate everything before downloading anything.
        let mut planned: Vec<Planned> = Vec::new();
        for f in &idx.files {
            if f.env
                .as_ref()
                .and_then(|e| e.get("client"))
                .map(String::as_str)
                == Some("unsupported")
            {
                continue;
            }
            let url = f
                .downloads
                .iter()
                .find(|u| allowed_download(u))
                .cloned()
                .ok_or_else(|| {
                    bad_pack("mrpack file has no allowed download").detail("path", &f.path)
                })?;
            let sha1 = f
                .hashes
                .get("sha1")
                .cloned()
                .ok_or_else(|| bad_pack("mrpack file has no sha1").detail("path", &f.path))?;
            if !f.hashes.contains_key("sha512") {
                return Err(bad_pack("mrpack file has no sha512").detail("path", &f.path));
            }
            safe_join(Path::new("game"), &f.path)?;
            planned.push((f.clone(), url, sha1));
        }
        sink.check_cancelled()?;

        let staging = self.staging_dir().await?;
        let r = async {
            let cidx = self.stage_mrpack(path, &planned, &staging, sink).await?;
            self.finish_pack(&staging, &idx.name, mc, loader, cidx, meta, sink)
                .await
        }
        .await;
        let _ = tokio::fs::remove_dir_all(&staging).await;
        r
    }

    /// Downloads the pack files and unpacks the overrides into `staging`.
    async fn stage_mrpack(
        &self,
        path: &Path,
        planned: &[Planned],
        staging: &Path,
        sink: &dyn ProgressSink,
    ) -> AppResult<ContentIndex> {
        sink.progress(0, Some(planned.len() as u64), "stages.packMods");
        let reqs = planned
            .iter()
            .map(|(f, url, sha1)| {
                Ok(DownloadReq {
                    urls: vec![parse_url(url)?],
                    sha1: Some(sha1.clone()),
                    size: (f.file_size > 0).then_some(f.file_size),
                    dest: safe_join(staging, &f.path)?,
                    kind: ResourceKind::Content,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        if !reqs.is_empty() {
            self.dl.fetch_many(reqs, sink).await?;
        }

        let mut cidx = ContentIndex::default();
        for (f, url, sha1) in planned {
            let dest = safe_join(staging, &f.path)?;
            let (_, sha512) = file_hashes(&dest).await?;
            if let Some(want) = f.hashes.get("sha512") {
                if !want.eq_ignore_ascii_case(&sha512) {
                    let _ = tokio::fs::remove_file(&dest).await;
                    return Err(AppError::new(ErrorCode::HashMismatch, "sha512 mismatch")
                        .detail("path", f.path.as_str()));
                }
            }
            if let (Some((dir, kind)), Some((project, version))) =
                (kind_of_path(&f.path), modrinth_ids_from_url(url))
            {
                let file = f.path.rsplit('/').next().unwrap_or_default();
                cidx.files.insert(
                    key(dir, file),
                    IndexEntry {
                        kind,
                        source: ContentSourceKind::Modrinth,
                        project_id: project,
                        version_id: version,
                        title: None,
                        version_number: None,
                        sha1: Some(sha1.clone()),
                        sha512: Some(sha512),
                        size: Some(f.file_size),
                        url: Some(url.clone()),
                    },
                );
            }
        }
        sink.check_cancelled()?;
        sink.progress(0, None, "stages.packOverrides");
        let (p, g) = (path.to_path_buf(), staging.to_path_buf());
        join(
            tokio::task::spawn_blocking(move || -> AppResult<()> {
                extract_prefix(&p, "overrides", &g)?;
                extract_prefix(&p, "client-overrides", &g)?;
                Ok(())
            })
            .await,
        )?;
        Ok(cidx)
    }

    async fn import_cf(
        &self,
        path: &Path,
        meta: &PackMeta,
        sink: &dyn ProgressSink,
    ) -> AppResult<Instance> {
        let cf = self.curseforge.clone().ok_or_else(no_key_error)?;
        sink.progress(0, None, "stages.packResolve");
        let p = path.to_path_buf();
        let bytes = join(
            tokio::task::spawn_blocking(move || read_entry(&mut open_zip(&p)?, CF_MANIFEST)).await,
        )?;
        let m: CfManifest = parse(&bytes, CF_MANIFEST)?;
        let (mc, loader) = cf_target(&m)?;
        let ids: Vec<u64> = m
            .files
            .iter()
            .filter(|f| f.required)
            .map(|f| f.file_id)
            .collect();
        let files = cf.files_with_mods(&ids).await?;
        if files.len() < ids.len() {
            sink.log(&format!(
                "{} pack file(s) are no longer available on CurseForge",
                ids.len() - files.len()
            ));
        }
        // (url, sha1, size, relative path, index key, entry)
        let mut planned = Vec::new();
        let mut manual = 0usize;
        for (v, kind, rf) in &files {
            let Ok(dir) = crate::util::kind_dir(*kind) else {
                continue;
            };
            crate::util::check_file_name(&v.file_name)?;
            let Some(url) = &rf.url else {
                manual += 1;
                sink.log(&format!(
                    "manual download required: {} → {} (put it into {dir}/)",
                    v.file_name, rf.manual_url
                ));
                continue;
            };
            let entry = IndexEntry {
                kind: *kind,
                source: ContentSourceKind::CurseForge,
                project_id: v.project_id.clone(),
                version_id: v.version_id.clone(),
                title: None,
                version_number: Some(v.version_number.clone()),
                sha1: rf.sha1.clone(),
                sha512: None,
                size: Some(v.size),
                url: Some(url.clone()),
            };
            planned.push((
                parse_url(url)?,
                rf.sha1.clone(),
                v.size,
                (dir, v.file_name.clone()),
                key(dir, &v.file_name),
                entry,
            ));
        }
        if manual > 0 {
            sink.log(&format!(
                "{manual} file(s) must be downloaded by hand (allowModDistribution=false)"
            ));
        }
        sink.check_cancelled()?;

        let staging = self.staging_dir().await?;
        let r = async {
            sink.progress(0, Some(planned.len() as u64), "stages.packMods");
            let mut reqs = Vec::new();
            let mut cidx = ContentIndex::default();
            for (url, sha1, size, (dir, name), k, entry) in planned {
                reqs.push(DownloadReq {
                    urls: vec![url],
                    sha1,
                    size: (size > 0).then_some(size),
                    dest: staging.join(dir).join(name),
                    kind: ResourceKind::Content,
                });
                cidx.files.insert(k, entry);
            }
            if !reqs.is_empty() {
                self.dl.fetch_many(reqs, sink).await?;
            }
            sink.check_cancelled()?;
            sink.progress(0, None, "stages.packOverrides");
            let (p, g, o) = (path.to_path_buf(), staging.clone(), m.overrides.clone());
            join(tokio::task::spawn_blocking(move || extract_prefix(&p, &o, &g)).await)?;
            self.finish_pack(&staging, &m.name, mc, loader, cidx, meta, sink)
                .await
        }
        .await;
        let _ = tokio::fs::remove_dir_all(&staging).await;
        r
    }

    /// Downloads a catalogue modpack into the cache (hash-checked) and imports it.
    async fn install_remote(
        &self,
        source: ContentSourceKind,
        project_id: &str,
        version_id: Option<&str>,
        sink: &dyn ProgressSink,
    ) -> AppResult<Instance> {
        sink.progress(0, None, "stages.packDownload");
        let (pf, want) = match source {
            ContentSourceKind::Modrinth => (
                self.modrinth.pack_file(project_id, version_id).await?,
                Detected::Mrpack,
            ),
            ContentSourceKind::CurseForge => (
                self.curseforge
                    .clone()
                    .ok_or_else(no_key_error)?
                    .pack_file(project_id, version_id)
                    .await?,
                Detected::CurseForge,
            ),
            ContentSourceKind::Local => {
                return Err(bad_pack("local files have no remote catalogue"));
            }
        };
        if !allowed_download(&pf.url) {
            return Err(bad_pack("modpack download host is not allowed").detail("url", &pf.url));
        }
        let sha1 = pf.sha1.to_ascii_lowercase();
        if sha1.len() != 40 || !sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad_pack("bad modpack sha1").detail("sha1", &pf.sha1));
        }
        sink.log(&format!(
            "modpack {} (version {}): {}",
            pf.title, pf.version_id, pf.file_name
        ));
        let ext = if want == Detected::Mrpack {
            "mrpack"
        } else {
            "zip"
        };
        let dest = self
            .paths
            .cache()
            .join("content")
            .join("packs")
            .join(format!("{sha1}.{ext}"));
        self.dl
            .fetch_many(
                vec![DownloadReq {
                    urls: vec![parse_url(&pf.url)?],
                    sha1: Some(sha1),
                    size: (pf.size > 0).then_some(pf.size),
                    dest: dest.clone(),
                    kind: ResourceKind::Content,
                }],
                sink,
            )
            .await?;
        if let Some(want512) = &pf.sha512 {
            let (_, got) = file_hashes(&dest).await?;
            if !want512.eq_ignore_ascii_case(&got) {
                let _ = tokio::fs::remove_file(&dest).await;
                return Err(AppError::new(ErrorCode::HashMismatch, "sha512 mismatch")
                    .detail("file", &pf.file_name));
            }
        }
        sink.check_cancelled()?;
        let d = dest.clone();
        let names = join(tokio::task::spawn_blocking(move || entry_names(&d)).await)?;
        if detect(&names) != Some(want) {
            let _ = tokio::fs::remove_file(&dest).await;
            return Err(bad_pack("downloaded modpack has an unexpected format"));
        }
        let meta = PackMeta {
            name: Some(pf.title.clone()),
            icon_url: pf.icon_url.clone(),
        };
        let r = if want == Detected::Mrpack {
            self.import_mrpack(&dest, &meta, sink).await
        } else {
            self.import_cf(&dest, &meta, sink).await
        };
        // Kept on failure: a retry reuses the verified download.
        if r.is_ok() {
            let _ = tokio::fs::remove_file(&dest).await;
        }
        r
    }

    async fn import_launcher(&self, path: &Path, sink: &dyn ProgressSink) -> AppResult<Instance> {
        let p = path.to_path_buf();
        let (bytes, index) = join(
            tokio::task::spawn_blocking(move || -> AppResult<(Vec<u8>, Option<Vec<u8>>)> {
                let mut z = open_zip(&p)?;
                let m = read_entry(&mut z, LAUNCHER_MANIFEST)?;
                let idx = read_entry(&mut z, crate::index::INDEX_FILE).ok();
                Ok((m, idx))
            })
            .await,
        )?;
        let m: LauncherManifest = parse(&bytes, LAUNCHER_MANIFEST)?;
        let inst = self
            .create_instance(&m.name, m.mc.clone(), m.loader.clone(), sink)
            .await?;
        let game = self.instances.game_dir(inst.id);
        let (p, g) = (path.to_path_buf(), game.clone());
        join(tokio::task::spawn_blocking(move || extract_prefix(&p, "game", &g)).await)?;
        if let Some(b) = index {
            let idx: ContentIndex = serde_json::from_slice(&b).unwrap_or_default();
            idx.save(&self.instance_dir(inst.id)).await?;
        }
        self.instances
            .update(
                inst.id,
                InstancePatch {
                    // Never trust an executable path from a foreign archive.
                    java: Some(JavaCfg {
                        path: None,
                        major: m.java.major,
                    }),
                    memory: Some(m.memory),
                    jvm_args: Some(safe_jvm_args(&m.jvm_args, sink)),
                    ..Default::default()
                },
            )
            .await
    }

    async fn export(
        &self,
        id: InstanceId,
        format: PackFormat,
        out: &Path,
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        let inst = self.instances.get(id).await?;
        let game = self.instances.game_dir(id);
        let idx = ContentIndex::load(&self.instance_dir(id)).await?;
        let g = game.clone();
        let full = format == PackFormat::LauncherZip;
        let files = join(tokio::task::spawn_blocking(move || walk(&g, !full)).await)?;
        sink.progress(0, Some(files.len() as u64), "stages.packExport");

        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        let mut copies: Vec<(String, PathBuf)> = Vec::new();
        match format {
            PackFormat::LauncherZip => {
                let m = LauncherManifest {
                    format: 1,
                    name: inst.name.clone(),
                    mc: inst.mc.clone(),
                    loader: inst.loader.clone(),
                    java: inst.java.clone(),
                    memory: inst.memory,
                    jvm_args: inst.jvm_args.clone(),
                    icon: inst.icon.clone(),
                };
                entries.push((LAUNCHER_MANIFEST.into(), json_bytes(&m)?));
                entries.push((crate::index::INDEX_FILE.into(), json_bytes(&idx)?));
                for rel in files {
                    copies.push((format!("game/{rel}"), safe_join(&game, &rel)?));
                }
            }
            PackFormat::Mrpack => {
                let mut deps = BTreeMap::new();
                deps.insert("minecraft".to_owned(), inst.mc.clone());
                if let Some(k) = mrpack_loader_key(inst.loader.kind) {
                    let v = inst.loader.version.clone().ok_or_else(|| {
                        bad_pack("loader version unknown; repair the instance first")
                    })?;
                    deps.insert(k.to_owned(), v);
                }
                let mut mfiles = Vec::new();
                for rel in files {
                    let remote = idx.files.get(&rel).filter(|e| {
                        e.url.as_deref().is_some_and(allowed_download)
                            && e.sha1.is_some()
                            && e.sha512.is_some()
                    });
                    match remote {
                        Some(e) => {
                            let mut hashes = BTreeMap::new();
                            hashes.insert("sha1".into(), e.sha1.clone().unwrap_or_default());
                            hashes.insert("sha512".into(), e.sha512.clone().unwrap_or_default());
                            mfiles.push(MrpackFile {
                                path: rel.clone(),
                                hashes,
                                env: None,
                                downloads: vec![e.url.clone().unwrap_or_default()],
                                file_size: e.size.unwrap_or(0),
                            });
                        }
                        None => copies.push((format!("overrides/{rel}"), safe_join(&game, &rel)?)),
                    }
                }
                let index = MrpackIndex {
                    format_version: 1,
                    game: "minecraft".into(),
                    version_id: "1.0.0".into(),
                    name: inst.name.clone(),
                    summary: None,
                    files: mfiles,
                    dependencies: deps,
                };
                entries.push((MRPACK_INDEX.into(), json_bytes(&index)?));
            }
            PackFormat::CurseForgeZip => {
                let mut cfiles = Vec::new();
                for rel in files {
                    let cf = idx
                        .files
                        .get(&rel)
                        .filter(|e| e.source == ContentSourceKind::CurseForge)
                        .and_then(|e| {
                            Some((
                                e.project_id.parse::<u64>().ok()?,
                                e.version_id.parse::<u64>().ok()?,
                            ))
                        });
                    match cf {
                        Some((p, f)) => cfiles.push(CfManifestFile {
                            project_id: p,
                            file_id: f,
                            required: true,
                        }),
                        None => copies.push((format!("overrides/{rel}"), safe_join(&game, &rel)?)),
                    }
                }
                let manifest = CfManifest {
                    minecraft: CfMinecraft {
                        version: inst.mc.clone(),
                        mod_loaders: cf_loader_id(&inst.loader)
                            .map(|id| vec![CfModLoader { id, primary: true }])
                            .unwrap_or_default(),
                    },
                    manifest_type: manifest_type(),
                    manifest_version: 1,
                    name: inst.name.clone(),
                    version: "1.0.0".into(),
                    author: String::new(),
                    files: cfiles,
                    overrides: overrides(),
                };
                entries.push((CF_MANIFEST.into(), json_bytes(&manifest)?));
            }
        }
        let out = out.to_path_buf();
        let n = copies.len() as u64;
        join(tokio::task::spawn_blocking(move || write_zip(&out, &entries, &copies)).await)?;
        sink.progress(n, Some(n), "stages.packExport");
        Ok(())
    }
}

#[async_trait]
impl PackIo for ContentServices {
    async fn install_remote(
        &self,
        source: ContentSourceKind,
        project_id: &str,
        version_id: Option<&str>,
        sink: &dyn ProgressSink,
    ) -> AppResult<Instance> {
        self.inner
            .install_remote(source, project_id, version_id, sink)
            .await
    }

    async fn import(&self, path: &str, sink: &dyn ProgressSink) -> AppResult<Instance> {
        let p = PathBuf::from(path);
        let pp = p.clone();
        let names = join(tokio::task::spawn_blocking(move || entry_names(&pp)).await)?;
        match detect(&names) {
            Some(Detected::Mrpack) => {
                self.inner
                    .import_mrpack(&p, &PackMeta::default(), sink)
                    .await
            }
            Some(Detected::CurseForge) => {
                self.inner.import_cf(&p, &PackMeta::default(), sink).await
            }
            Some(Detected::Launcher) => self.inner.import_launcher(&p, sink).await,
            None => Err(bad_pack("unknown modpack format")),
        }
    }

    async fn export(
        &self,
        instance: InstanceId,
        format: PackFormat,
        path: &str,
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        if path.trim().is_empty() {
            return Err(bad_pack("empty export path"));
        }
        self.inner
            .export(instance, format, Path::new(path), sink)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mrpack_fixture() {
        let idx: MrpackIndex = parse(
            include_bytes!("../tests/fixtures/packs/modrinth.index.json"),
            "x",
        )
        .unwrap();
        let (mc, loader) = mrpack_target(&idx).unwrap();
        assert_eq!(mc, "1.21.1");
        assert_eq!(loader.kind, LoaderKind::Fabric);
        assert_eq!(loader.version.as_deref(), Some("0.16.9"));
        assert_eq!(idx.files.len(), 2);
        assert!(allowed_download(&idx.files[0].downloads[0]));
        assert_eq!(
            modrinth_ids_from_url(&idx.files[0].downloads[0]),
            Some(("AANobbMI".into(), "u1OEbNKx".into()))
        );
        // Re-serialization keeps the spec field names.
        let v = serde_json::to_value(&idx).unwrap();
        assert_eq!(v["formatVersion"], 1);
        assert!(v["files"][0]["fileSize"].is_u64());
    }

    #[test]
    fn foreign_jvm_args_are_filtered() {
        let args: Vec<String> = [
            "-Xmx4G",
            "-Xms1G",
            "-Dfoo=bar",
            "-XX:+UseG1GC",
            "-XX:OnOutOfMemoryError=calc.exe",
            "-XX:OnError=cmd",
            "-XX:+ShowCodeDetailsInExceptionMessages",
            "-XX:AbortVMOnExceptionCommand=x",
            "-javaagent:evil.jar",
            "-agentpath:x.dll",
            "-cp",
            "-XX:Flags=/tmp/evil",
            "-XX:VMOptionsFile=evil.txt",
            "-XX:+UnlockDiagnosticVMOptions",
            "-XX:ErrorFile=/tmp/x",
            "-XX:MaxGCPauseMillis=50",
            "-XX:MaxGCPauseMillis=50;rm",
            "-XX:G1HeapRegionSize=16M",
            "-XX:-UseZGC",
            "-XX:UseG1GC",
            "-XX:LogFile=x",
            "-Xmx4G;calc",
            "-Djava.library.path=evil",
            "-Dlog4j.configurationFile=http://x/y.xml",
            "-Djava.system.class.loader=Evil",
            "-Dfml.ignoreInvalidMinecraftCertificates=true",
            "-Dsome.mod.setting=1",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        let out = safe_jvm_args(&args, &lc_runtime::NullSink::default());
        assert_eq!(
            out,
            vec![
                "-Xmx4G",
                "-Xms1G",
                "-Dfoo=bar",
                "-XX:+UseG1GC",
                "-XX:+ShowCodeDetailsInExceptionMessages",
                "-XX:MaxGCPauseMillis=50",
                "-XX:G1HeapRegionSize=16M",
                "-XX:-UseZGC",
                "-Dsome.mod.setting=1",
            ]
        );
    }

    #[test]
    fn rejects_foreign_hosts() {
        assert!(!allowed_download("https://evil.example/x.jar"));
        assert!(!allowed_download("http://cdn.modrinth.com/x.jar"));
        assert!(allowed_download(
            "https://cdn.modrinth.com/data/a/versions/b/c.jar"
        ));
    }

    #[test]
    fn parses_cf_manifest_fixture() {
        let m: CfManifest =
            parse(include_bytes!("../tests/fixtures/packs/manifest.json"), "x").unwrap();
        let (mc, loader) = cf_target(&m).unwrap();
        assert_eq!(mc, "1.20.1");
        assert_eq!(loader.kind, LoaderKind::Forge);
        assert_eq!(loader.version.as_deref(), Some("47.3.0"));
        assert_eq!(m.files.len(), 2);
        assert_eq!(m.files[0].project_id, 238222);
        assert_eq!(cf_loader_id(&loader).as_deref(), Some("forge-47.3.0"));
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["files"][0]["projectID"], 238222);
        assert_eq!(v["minecraft"]["modLoaders"][0]["id"], "forge-47.3.0");
    }

    #[test]
    fn neoforge_cf_id_strips_mc() {
        let m: CfManifest = serde_json::from_str(
            r#"{"minecraft":{"version":"1.20.1","modLoaders":[{"id":"neoforge-1.20.1-47.1.106","primary":true}]}}"#,
        )
        .unwrap();
        let (_, l) = cf_target(&m).unwrap();
        assert_eq!(l.kind, LoaderKind::NeoForge);
        assert_eq!(l.version.as_deref(), Some("47.1.106"));
    }

    #[test]
    fn detection_and_zip_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        std::fs::create_dir_all(game.join("config")).unwrap();
        std::fs::create_dir_all(game.join("logs")).unwrap();
        std::fs::create_dir_all(game.join("saves/w")).unwrap();
        std::fs::write(game.join("config/a.toml"), "x=1").unwrap();
        std::fs::write(game.join("logs/latest.log"), "log").unwrap();
        std::fs::write(game.join("saves/w/level.dat"), "lvl").unwrap();
        std::fs::write(game.join("options.txt"), "o").unwrap();
        let files = walk(&game, true).unwrap();
        assert_eq!(files, vec!["config/a.toml", "options.txt"]);
        assert_eq!(walk(&game, false).unwrap().len(), 3);

        let out = dir.path().join("p.mrpack");
        let copies: Vec<_> = files
            .iter()
            .map(|r| (format!("overrides/{r}"), game.join(r)))
            .collect();
        write_zip(&out, &[(MRPACK_INDEX.into(), b"{}".to_vec())], &copies).unwrap();
        let names = entry_names(&out).unwrap();
        assert_eq!(detect(&names), Some(Detected::Mrpack));
        let target = dir.path().join("new");
        assert_eq!(extract_prefix(&out, "overrides", &target).unwrap(), 2);
        assert_eq!(
            std::fs::read_to_string(target.join("config/a.toml")).unwrap(),
            "x=1"
        );
        assert_eq!(
            detect(&["manifest.json".into()]),
            Some(Detected::CurseForge)
        );
        assert_eq!(detect(&["x".into()]), None);
    }

    #[test]
    fn zip_slip_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("evil.zip");
        write_zip(
            &out,
            &[("overrides/../../evil.txt".into(), b"x".to_vec())],
            &[],
        )
        .unwrap();
        assert!(extract_prefix(&out, "overrides", &dir.path().join("g")).is_err());
    }
}
