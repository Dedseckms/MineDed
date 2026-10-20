//! Java runtimes: Mojang runtime manifest installs + detection of system Javas.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lc_http::{DownloadReq, Downloader, ResourceKind};
use lc_runtime::{Paths, ProgressSink};
use lc_types::java::JavaRuntime;
use lc_types::{AppError, AppResult, ErrorCode};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::util::StageSink;
use crate::util::{io_err, json_err, parse_url, read_json, sha1_hex, write_atomic, write_json};

pub(crate) const RUNTIME_MANIFEST_URL: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
const MARKER: &str = "mineup-runtime.json";
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Manifest formats
// ---------------------------------------------------------------------------

/// `all.json`: platform → component → builds.
pub(crate) type RuntimeAll = BTreeMap<String, BTreeMap<String, Vec<RuntimeBuild>>>;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RuntimeBuild {
    pub manifest: RuntimeManifestRef,
    pub version: RuntimeVersion,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RuntimeManifestRef {
    #[serde(default)]
    pub sha1: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RuntimeVersion {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ComponentManifest {
    pub files: BTreeMap<String, ComponentFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum ComponentFile {
    Directory,
    File {
        #[serde(default)]
        executable: bool,
        downloads: FileDownloads,
    },
    Link {
        target: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileDownloads {
    pub raw: RawDownload,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawDownload {
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

/// Written next to a managed runtime after a successful install.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Marker {
    component: String,
    major: u32,
    version: String,
    arch: String,
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Key of the current platform in `all.json`.
pub(crate) fn platform_key(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("windows-x64"),
        ("windows", "x86") => Some("windows-x86"),
        ("windows", "aarch64") => Some("windows-arm64"),
        ("linux", "x86_64") => Some("linux"),
        ("linux", "x86") => Some("linux-i386"),
        ("macos", "x86_64") => Some("mac-os"),
        ("macos", "aarch64") => Some("mac-os-arm64"),
        _ => None,
    }
}

/// Well-known Mojang component for a Java major.
pub(crate) fn component_for_major(major: u32) -> Option<&'static str> {
    match major {
        8 => Some("jre-legacy"),
        16 => Some("java-runtime-alpha"),
        17 => Some("java-runtime-gamma"),
        21 => Some("java-runtime-delta"),
        25 => Some("java-runtime-epsilon"),
        _ => None,
    }
}

/// Major from a Java/runtime version string: `1.8.0_392`, `8u51-...`, `17.0.15`, `21`.
pub(crate) fn parse_major(version: &str) -> Option<u32> {
    let v = version.trim().trim_matches('"');
    let digits = |s: &str| -> Option<u32> {
        let d: String = s.chars().take_while(char::is_ascii_digit).collect();
        d.parse().ok()
    };
    if let Some(rest) = v.strip_prefix("1.") {
        return digits(rest);
    }
    digits(v)
}

/// Picks `(component, build)`: explicit component, known mapping, then any
/// non-snapshot component whose version has the requested major.
pub(crate) fn pick_component<'a>(
    all: &'a RuntimeAll,
    platform: &str,
    major: u32,
    preferred: Option<&str>,
) -> Option<(String, &'a RuntimeBuild)> {
    let comps = all.get(platform)?;
    let get = |name: &str| comps.get(name).and_then(|b| b.first());
    if let Some(name) = preferred {
        if let Some(b) = get(name) {
            return Some((name.to_owned(), b));
        }
    }
    if let Some(name) = component_for_major(major) {
        if let Some(b) = get(name) {
            return Some((name.to_owned(), b));
        }
    }
    comps
        .iter()
        .filter(|(name, _)| name.starts_with("j") && !name.contains("snapshot"))
        .filter_map(|(name, builds)| builds.first().map(|b| (name, b)))
        .find(|(_, b)| parse_major(&b.version.name) == Some(major))
        .map(|(name, b)| (name.clone(), b))
}

/// Parsed output of `java -XshowSettings:properties -version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JavaProbe {
    pub version: String,
    pub major: u32,
    pub vendor: Option<String>,
    pub arch: String,
}

pub(crate) fn normalize_arch(a: &str) -> String {
    match a.trim() {
        "amd64" | "x86_64" | "x64" => "x64".into(),
        "x86" | "i386" | "i486" | "i586" | "i686" => "x86".into(),
        "aarch64" | "arm64" => "arm64".into(),
        other => other.into(),
    }
}

pub(crate) fn parse_java_output(out: &str) -> Option<JavaProbe> {
    let mut props = BTreeMap::new();
    for line in out.lines() {
        if let Some((k, v)) = line.trim().split_once(" = ") {
            props.insert(k.trim().to_owned(), v.trim().to_owned());
        }
    }
    let version = props.get("java.version").cloned().or_else(|| {
        // `openjdk version "17.0.2" 2022-01-18`
        out.lines().find_map(|l| {
            let l = l.trim();
            if !l.contains(" version ") {
                return None;
            }
            let start = l.find('"')? + 1;
            let end = start + l[start..].find('"')?;
            Some(l[start..end].to_owned())
        })
    })?;
    let major = parse_major(&version)?;
    let arch = props
        .get("os.arch")
        .map(|a| normalize_arch(a))
        .unwrap_or_else(|| {
            if out.contains("64-Bit") {
                normalize_arch(std::env::consts::ARCH)
            } else {
                "x86".into()
            }
        });
    Some(JavaProbe {
        version,
        major,
        vendor: props.get("java.vendor").cloned(),
        arch,
    })
}

/// Values of `JavaHome` / `Path` / `InstallationPath` from `reg query /s` output.
pub(crate) fn parse_reg_output(out: &str) -> Vec<PathBuf> {
    let mut res = Vec::new();
    for line in out.lines() {
        let mut parts = line.trim().splitn(3, "    ");
        let (Some(name), Some(kind), Some(value)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if !kind.trim().starts_with("REG_") {
            continue;
        }
        if matches!(name.trim(), "JavaHome" | "Path" | "InstallationPath") {
            let v = value.trim();
            if !v.is_empty() {
                res.push(PathBuf::from(v));
            }
        }
    }
    res
}

pub(crate) fn exe_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// Executable used to run the game: `javaw.exe` on Windows (no console window).
pub(crate) fn game_executable(java: &Path) -> PathBuf {
    if cfg!(windows) {
        let is_java = java
            .file_name()
            .map(|n| n.eq_ignore_ascii_case("java.exe"))
            .unwrap_or(false);
        if is_java {
            let w = java.with_file_name("javaw.exe");
            if w.exists() {
                return w;
            }
        }
    }
    java.to_path_buf()
}

pub(crate) fn current_arch() -> String {
    normalize_arch(std::env::consts::ARCH)
}

/// Creates a hidden-window command (no console flash on Windows).
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

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

pub(crate) struct JavaService {
    paths: Paths,
    dl: Arc<dyn Downloader>,
    detected: Mutex<Option<Vec<JavaRuntime>>>,
    install_lock: Mutex<()>,
}

impl JavaService {
    pub fn new(paths: Paths, dl: Arc<dyn Downloader>) -> Self {
        Self {
            paths,
            dl,
            detected: Mutex::new(None),
            install_lock: Mutex::new(()),
        }
    }

    fn runtime_from_marker(&self, dir: &Path, m: Marker) -> JavaRuntime {
        JavaRuntime {
            path: dir
                .join("bin")
                .join(exe_name())
                .to_string_lossy()
                .into_owned(),
            major: m.major,
            version: m.version,
            vendor: Some("Mojang".into()),
            arch: m.arch,
            managed: true,
        }
    }

    async fn managed_component(&self, component: &str) -> Option<JavaRuntime> {
        let dir = self.paths.java().join(component);
        let m: Marker = read_json(&dir.join(MARKER)).await.ok()?;
        let rt = self.runtime_from_marker(&dir, m);
        tokio::fs::try_exists(&rt.path)
            .await
            .unwrap_or(false)
            .then_some(rt)
    }

    /// Runtimes installed by the launcher under `Paths::java()`.
    pub async fn managed(&self) -> Vec<JavaRuntime> {
        let mut out = Vec::new();
        let Ok(mut rd) = tokio::fs::read_dir(self.paths.java()).await else {
            return out;
        };
        while let Ok(Some(e)) = rd.next_entry().await {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(rt) = self.managed_component(&name).await {
                out.push(rt);
            }
        }
        out.sort_by_key(|a| std::cmp::Reverse(a.major));
        out
    }

    pub async fn list(&self) -> AppResult<Vec<JavaRuntime>> {
        let mut out = self.managed().await;
        let cached = self.detected.lock().await.clone();
        let detected = match cached {
            Some(d) => d,
            None => self.detect().await?,
        };
        out.extend(detected);
        Ok(out)
    }

    pub async fn detect(&self) -> AppResult<Vec<JavaRuntime>> {
        let managed_root = self.paths.java();
        let candidates = candidate_paths().await;
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for exe in candidates {
            if exe.starts_with(&managed_root) {
                continue;
            }
            let key = match tokio::fs::canonicalize(&exe).await {
                Ok(p) => p,
                Err(_) => continue,
            };
            if !seen.insert(key.to_string_lossy().to_lowercase()) {
                continue;
            }
            if let Some(p) = probe(&exe).await {
                out.push(JavaRuntime {
                    path: exe.to_string_lossy().into_owned(),
                    major: p.major,
                    version: p.version,
                    vendor: p.vendor,
                    arch: p.arch,
                    managed: false,
                });
            }
        }
        out.sort_by_key(|a| std::cmp::Reverse(a.major));
        *self.detected.lock().await = Some(out.clone());
        Ok(out)
    }

    async fn runtime_manifest(&self) -> AppResult<RuntimeAll> {
        let url = parse_url(RUNTIME_MANIFEST_URL)?;
        let cache = self.paths.cache().join("java-runtime-all.json");
        match self.dl.get_bytes(&url, ResourceKind::JavaRuntime).await {
            Ok(bytes) => {
                let all: RuntimeAll = serde_json::from_slice(&bytes)
                    .map_err(|e| json_err("java runtime manifest", e))?;
                let _ = write_atomic(&cache, &bytes).await;
                Ok(all)
            }
            Err(e) => read_json(&cache).await.map_err(|_| e),
        }
    }

    pub async fn install(&self, major: u32, sink: &dyn ProgressSink) -> AppResult<JavaRuntime> {
        self.install_component(None, major, sink).await
    }

    /// Installs (or repairs) a Mojang runtime component into `Paths::java()/<component>`.
    pub async fn install_component(
        &self,
        preferred: Option<&str>,
        major: u32,
        sink: &dyn ProgressSink,
    ) -> AppResult<JavaRuntime> {
        let _guard = self.install_lock.lock().await;
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let platform = platform_key(os, arch).ok_or_else(|| {
            AppError::new(
                ErrorCode::JavaMissing,
                "no Mojang runtimes for this platform",
            )
            .detail("os", os)
            .detail("arch", arch)
        })?;
        let all = self.runtime_manifest().await?;
        let (component, build) =
            pick_component(&all, platform, major, preferred).ok_or_else(|| {
                AppError::new(
                    ErrorCode::JavaMissing,
                    format!("no Mojang runtime for Java {major}"),
                )
                .detail("platform", platform)
                .detail("major", major.to_string())
            })?;
        sink.check_cancelled()?;

        let murl = parse_url(&build.manifest.url)?;
        let bytes = self.dl.get_bytes(&murl, ResourceKind::JavaRuntime).await?;
        if let Some(want) = &build.manifest.sha1 {
            let got = sha1_hex(&bytes);
            if !got.eq_ignore_ascii_case(want) {
                return Err(
                    AppError::new(ErrorCode::HashMismatch, "runtime manifest hash")
                        .detail("expected", want.as_str())
                        .detail("actual", got)
                        .detail("url", build.manifest.url.as_str()),
                );
            }
        }
        let manifest: ComponentManifest = serde_json::from_slice(&bytes)
            .map_err(|e| json_err("runtime component manifest", e))?;

        let root = self.paths.java().join(&component);
        let plan = install_plan(&manifest, &root)?;
        for d in &plan.dirs {
            tokio::fs::create_dir_all(d)
                .await
                .map_err(|e| io_err(d, e))?;
        }
        let staged = StageSink {
            inner: sink,
            stage: "stages.java",
        };
        self.dl.fetch_many(plan.downloads, &staged).await?;
        sink.check_cancelled()?;
        apply_permissions_and_links(&plan.executables, &plan.links).await?;

        let java = root.join("bin").join(exe_name());
        if !tokio::fs::try_exists(&java).await.unwrap_or(false) {
            return Err(
                AppError::new(ErrorCode::JavaMissing, "runtime has no java executable")
                    .detail("path", java.to_string_lossy()),
            );
        }
        let marker = Marker {
            component: component.clone(),
            major: parse_major(&build.version.name).unwrap_or(major),
            version: build.version.name.clone(),
            arch: current_arch(),
        };
        write_json(&root.join(MARKER), &marker).await?;
        Ok(self.runtime_from_marker(&root, marker))
    }

    /// Managed runtime for `component`/`major`, installing it when missing;
    /// falls back to a detected system Java of the same major when offline.
    pub async fn ensure_component(
        &self,
        component: Option<&str>,
        major: u32,
        sink: &dyn ProgressSink,
    ) -> AppResult<JavaRuntime> {
        let name = component.or_else(|| component_for_major(major));
        if let Some(name) = name {
            if let Some(rt) = self.managed_component(name).await {
                return Ok(rt);
            }
        }
        if let Some(rt) = self.managed().await.into_iter().find(|r| r.major == major) {
            return Ok(rt);
        }
        match self.install_component(component, major, sink).await {
            Ok(rt) => Ok(rt),
            Err(e) if e.code == ErrorCode::Cancelled => Err(e),
            Err(e) => {
                let detected = self.detect().await.unwrap_or_default();
                let arch = current_arch();
                detected
                    .into_iter()
                    .filter(|r| r.major == major)
                    .max_by_key(|r| r.arch == arch)
                    .ok_or(e)
            }
        }
    }

    pub async fn ensure(&self, major: u32, sink: &dyn ProgressSink) -> AppResult<JavaRuntime> {
        self.ensure_component(None, major, sink).await
    }
}

/// What `install_component` does on disk, computed from the component manifest.
#[derive(Debug, Default)]
pub(crate) struct InstallPlan {
    pub dirs: Vec<PathBuf>,
    pub downloads: Vec<DownloadReq>,
    pub executables: Vec<PathBuf>,
    /// `(link path, target relative to the link)`.
    pub links: Vec<(PathBuf, String)>,
}

fn safe_join(root: &Path, rel: &str) -> AppResult<PathBuf> {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') || part.contains('\\') {
            return Err(
                AppError::new(ErrorCode::InvalidInput, "unsafe path in runtime manifest")
                    .detail("path", rel),
            );
        }
        p.push(part);
    }
    Ok(p)
}

pub(crate) fn install_plan(m: &ComponentManifest, root: &Path) -> AppResult<InstallPlan> {
    let mut plan = InstallPlan::default();
    for (rel, f) in &m.files {
        let dest = safe_join(root, rel)?;
        match f {
            ComponentFile::Directory => plan.dirs.push(dest),
            ComponentFile::File {
                executable,
                downloads,
            } => {
                if *executable {
                    plan.executables.push(dest.clone());
                }
                plan.downloads.push(DownloadReq {
                    urls: vec![parse_url(&downloads.raw.url)?],
                    sha1: downloads.raw.sha1.clone(),
                    size: downloads.raw.size,
                    dest,
                    kind: ResourceKind::JavaRuntime,
                });
            }
            ComponentFile::Link { target } => plan.links.push((dest, target.clone())),
        }
    }
    Ok(plan)
}

#[cfg(unix)]
async fn apply_permissions_and_links(
    executables: &[PathBuf],
    links: &[(PathBuf, String)],
) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;
    for exe in executables {
        tokio::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755))
            .await
            .map_err(|e| io_err(exe, e))?;
    }
    for (link, target) in links {
        let _ = tokio::fs::remove_file(link).await;
        if let Some(parent) = link.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| io_err(parent, e))?;
        }
        tokio::fs::symlink(target, link)
            .await
            .map_err(|e| io_err(link, e))?;
    }
    Ok(())
}

#[cfg(not(unix))]
async fn apply_permissions_and_links(
    _executables: &[PathBuf],
    _links: &[(PathBuf, String)],
) -> AppResult<()> {
    // Windows runtimes do not use symlinks or executable bits.
    Ok(())
}

async fn probe(exe: &Path) -> Option<JavaProbe> {
    let mut cmd = command(exe);
    cmd.arg("-XshowSettings:properties")
        .arg("-version")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(PROBE_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    parse_java_output(&text)
}

async fn candidate_paths() -> Vec<PathBuf> {
    let exe = exe_name();
    let mut homes: Vec<PathBuf> = Vec::new();
    if let Some(h) = std::env::var_os("JAVA_HOME") {
        homes.push(PathBuf::from(h));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            out.push(dir.join(exe));
        }
    }
    if cfg!(windows) {
        homes.extend(registry_homes().await);
    } else {
        for root in ["/usr/lib/jvm", "/usr/lib64/jvm", "/usr/java", "/opt/java"] {
            if let Ok(mut rd) = tokio::fs::read_dir(root).await {
                while let Ok(Some(e)) = rd.next_entry().await {
                    homes.push(e.path());
                }
            }
        }
    }
    for h in homes {
        out.push(h.join("bin").join(exe));
        out.push(h.join("jre").join("bin").join(exe));
    }
    let mut existing = Vec::new();
    for p in out {
        if tokio::fs::try_exists(&p).await.unwrap_or(false) {
            existing.push(p);
        }
    }
    existing
}

async fn registry_homes() -> Vec<PathBuf> {
    const KEYS: &[&str] = &[
        r"HKLM\SOFTWARE\JavaSoft",
        r"HKLM\SOFTWARE\WOW6432Node\JavaSoft",
        r"HKLM\SOFTWARE\Eclipse Adoptium",
        r"HKLM\SOFTWARE\Eclipse Foundation",
        r"HKLM\SOFTWARE\Microsoft\JDK",
        r"HKLM\SOFTWARE\Azul Systems\Zulu",
        r"HKLM\SOFTWARE\BellSoft\Liberica",
    ];
    let mut out = Vec::new();
    for key in KEYS {
        let mut cmd = command(Path::new("reg"));
        cmd.args(["query", key, "/s"])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        if let Ok(Ok(o)) = tokio::time::timeout(PROBE_TIMEOUT, cmd.output()).await {
            out.extend(parse_reg_output(&String::from_utf8_lossy(&o.stdout)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> RuntimeAll {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-runtime-all.json");
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
    }

    #[test]
    fn majors() {
        assert_eq!(parse_major("1.8.0_392"), Some(8));
        assert_eq!(parse_major("8u51-cacert462b08"), Some(8));
        assert_eq!(parse_major("17.0.15"), Some(17));
        assert_eq!(parse_major("21"), Some(21));
        assert_eq!(parse_major("\"25.0.1\""), Some(25));
        assert_eq!(parse_major("abc"), None);
    }

    #[test]
    fn picks_components() {
        let all = all();
        let (c, _) = pick_component(&all, "windows-x64", 8, None).unwrap();
        assert_eq!(c, "jre-legacy");
        let (c, _) = pick_component(&all, "windows-x64", 17, None).unwrap();
        assert_eq!(c, "java-runtime-gamma");
        let (c, b) = pick_component(&all, "linux", 21, None).unwrap();
        assert_eq!(c, "java-runtime-delta");
        assert_eq!(parse_major(&b.version.name), Some(21));
        let (c, _) = pick_component(&all, "linux", 21, Some("java-runtime-delta")).unwrap();
        assert_eq!(c, "java-runtime-delta");
        // Unknown preferred component falls back to the major mapping.
        let (c, _) = pick_component(&all, "linux", 8, Some("nope")).unwrap();
        assert_eq!(c, "jre-legacy");
        assert!(pick_component(&all, "linux", 11, None).is_none());
        assert!(pick_component(&all, "plan9", 8, None).is_none());
    }

    #[test]
    fn platform_keys() {
        assert_eq!(platform_key("windows", "x86_64"), Some("windows-x64"));
        assert_eq!(platform_key("linux", "x86_64"), Some("linux"));
        assert_eq!(platform_key("linux", "aarch64"), None);
    }

    #[test]
    fn parses_java_properties() {
        let out = "Property settings:\n    java.vendor = Eclipse Adoptium\n    java.version = 21.0.5\n    os.arch = amd64\n\nopenjdk version \"21.0.5\" 2024-10-15 LTS\n";
        let p = parse_java_output(out).unwrap();
        assert_eq!(p.major, 21);
        assert_eq!(p.version, "21.0.5");
        assert_eq!(p.vendor.as_deref(), Some("Eclipse Adoptium"));
        assert_eq!(p.arch, "x64");

        let legacy = "java version \"1.8.0_51\"\nJava(TM) SE Runtime Environment (build 1.8.0_51-b16)\nJava HotSpot(TM) 64-Bit Server VM (build 25.51-b03, mixed mode)\n";
        let p = parse_java_output(legacy).unwrap();
        assert_eq!(p.major, 8);
        assert_eq!(p.version, "1.8.0_51");
        assert!(parse_java_output("garbage").is_none());
    }

    #[test]
    fn parses_registry() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\JavaSoft\\JDK\\21\r\n    JavaHome    REG_SZ    C:\\Program Files\\Java\\jdk-21\r\n    RuntimeLib    REG_SZ    C:\\x\\jvm.dll\r\n\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Eclipse Adoptium\\JRE\\8\\hotspot\\MSI\r\n    Path    REG_SZ    C:\\Program Files\\Eclipse Adoptium\\jre-8\\\r\n";
        let homes = parse_reg_output(out);
        assert_eq!(
            homes,
            vec![
                PathBuf::from(r"C:\Program Files\Java\jdk-21"),
                PathBuf::from(r"C:\Program Files\Eclipse Adoptium\jre-8\"),
            ]
        );
    }

    #[test]
    fn plan_from_component_manifest() {
        let m: ComponentManifest = serde_json::from_str(
            r#"{"files":{
                "bin":{"type":"directory"},
                "bin/java":{"type":"file","executable":true,"downloads":{"raw":{"sha1":"aa","size":1,"url":"https://piston-data.mojang.com/x"}}},
                "lib/libjli.so":{"type":"link","target":"../bin/jli"},
                "../evil":{"type":"directory"}
            }}"#,
        )
        .unwrap();
        assert!(install_plan(&m, Path::new("/rt")).is_err());

        let m: ComponentManifest = serde_json::from_str(
            r#"{"files":{
                "bin":{"type":"directory"},
                "bin/java":{"type":"file","executable":true,"downloads":{"raw":{"sha1":"aa","size":1,"url":"https://piston-data.mojang.com/x"}}},
                "lib/libjli.so":{"type":"link","target":"../bin/jli"}
            }}"#,
        )
        .unwrap();
        let plan = install_plan(&m, Path::new("/rt")).unwrap();
        assert_eq!(plan.dirs, vec![Path::new("/rt").join("bin")]);
        assert_eq!(plan.downloads.len(), 1);
        assert_eq!(plan.downloads[0].kind, ResourceKind::JavaRuntime);
        assert_eq!(
            plan.executables,
            vec![Path::new("/rt").join("bin").join("java")]
        );
        assert_eq!(plan.links[0].1, "../bin/jli");
    }
}
