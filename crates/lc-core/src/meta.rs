//! Mojang metadata formats: version manifest, version json, asset index,
//! plus rule evaluation and library resolution.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub(crate) const MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub(crate) const LIBRARIES_URL: &str = "https://libraries.minecraft.net/";
pub(crate) const RESOURCES_URL: &str = "https://resources.download.minecraft.net/";

// ---------------------------------------------------------------------------
// Version manifest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VersionManifest {
    pub versions: Vec<ManifestVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub release_time: String,
}

// ---------------------------------------------------------------------------
// Version json
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VersionJson {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits_from: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Arguments>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<BTreeMap<String, Artifact>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_version: Option<JavaVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<Logging>,
    /// Version whose client jar is used (loaders that do not ship their own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jar: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Argument {
    Plain(String),
    Ruled { rules: Vec<Rule>, value: StrOrVec },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum StrOrVec {
    One(String),
    Many(Vec<String>),
}

impl StrOrVec {
    pub fn items(&self) -> Vec<&str> {
        match self {
            Self::One(s) => vec![s.as_str()],
            Self::Many(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Rule {
    pub action: RuleAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<BTreeMap<String, bool>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct OsRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Library {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<LibDownloads>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<Rule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natives: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<Extract>,
    /// Maven repository root for libraries without `downloads` (Fabric style).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct LibDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifiers: Option<BTreeMap<String, Artifact>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Artifact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssetIndexRef {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JavaVersion {
    #[serde(default)]
    pub component: Option<String>,
    pub major_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Logging {
    #[serde(default)]
    pub client: Option<LoggingClient>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LoggingClient {
    pub argument: String,
    pub file: LogFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LogFile {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

// ---------------------------------------------------------------------------
// Asset index
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AssetIndex {
    pub objects: BTreeMap<String, AssetObject>,
    /// `legacy` index: copy objects to `assets/virtual/<id>/<name>`.
    #[serde(default, rename = "virtual")]
    pub is_virtual: bool,
    /// `pre-1.6` index: copy objects to `<game_dir>/resources/<name>`.
    #[serde(default)]
    pub map_to_resources: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AssetObject {
    pub hash: String,
    pub size: u64,
}

impl AssetObject {
    /// `xx/hash`, relative to `assets/objects` and to the resources host.
    pub fn rel_path(&self) -> String {
        let prefix = self.hash.get(..2).unwrap_or("00");
        format!("{prefix}/{}", self.hash)
    }
}

// ---------------------------------------------------------------------------
// Environment and rules
// ---------------------------------------------------------------------------

/// Feature flags referenced by argument rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Features {
    pub demo_user: bool,
    pub custom_resolution: bool,
    pub quick_play_multiplayer: bool,
}

impl Features {
    fn get(&self, key: &str) -> bool {
        match key {
            "is_demo_user" => self.demo_user,
            "has_custom_resolution" => self.custom_resolution,
            "is_quick_play_multiplayer" => self.quick_play_multiplayer,
            _ => false,
        }
    }
}

/// OS/arch the rules are evaluated against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Env {
    /// Mojang naming: `windows`, `linux`, `osx`.
    pub os: &'static str,
    /// `x86_64`, `x86`, `aarch64`, ...
    pub arch: &'static str,
    pub os_version: String,
    pub features: Features,
}

impl Env {
    pub fn current() -> Self {
        let os = match std::env::consts::OS {
            "windows" => "windows",
            "macos" => "osx",
            _ => "linux",
        };
        let os_version = sysinfo::System::os_version().unwrap_or_default();
        Self {
            os,
            arch: std::env::consts::ARCH,
            os_version,
            features: Features::default(),
        }
    }

    /// `${arch}` in natives classifiers.
    pub fn bits(&self) -> &'static str {
        match self.arch {
            "x86" | "arm" => "32",
            _ => "64",
        }
    }

    fn arch_matches(&self, rule_arch: &str) -> bool {
        match rule_arch {
            "x86" | "i386" | "i686" => self.arch == "x86",
            "x86_64" | "amd64" | "x64" => self.arch == "x86_64",
            "arm64" | "aarch64" => self.arch == "aarch64",
            other => other == self.arch,
        }
    }
}

impl Rule {
    fn matches(&self, env: &Env) -> bool {
        if let Some(os) = &self.os {
            if let Some(name) = &os.name {
                if name != env.os {
                    return false;
                }
            }
            if let Some(arch) = &os.arch {
                if !env.arch_matches(arch) {
                    return false;
                }
            }
            if let Some(ver) = &os.version {
                let ok = regex::Regex::new(ver)
                    .map(|re| re.is_match(&env.os_version))
                    .unwrap_or(false);
                if !ok {
                    return false;
                }
            }
        }
        if let Some(features) = &self.features {
            if features.iter().any(|(k, v)| env.features.get(k) != *v) {
                return false;
            }
        }
        true
    }
}

/// Mojang semantics: no rules = allowed; otherwise the last matching rule wins
/// and nothing matching means disallowed.
pub(crate) fn rules_allow(rules: &[Rule], env: &Env) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule.matches(env) {
            allowed = rule.action == RuleAction::Allow;
        }
    }
    allowed
}

// ---------------------------------------------------------------------------
// Libraries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LibUse {
    Classpath,
    /// Old-style natives jar: extracted into the natives dir, not on classpath.
    Native {
        exclude: Vec<String>,
    },
}

/// One file a library contributes for the current platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LibFile {
    /// Relative to `Paths::libraries()`, with `/` separators.
    pub path: String,
    /// `None` = nothing to download (generated by a loader installer).
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub usage: LibUse,
}

impl LibFile {
    pub fn local(&self, libraries: &std::path::Path) -> PathBuf {
        let mut p = libraries.to_path_buf();
        for part in self.path.split('/') {
            p.push(part);
        }
        p
    }
}

struct MavenName<'a> {
    group: &'a str,
    artifact: &'a str,
    version: &'a str,
    classifier: Option<&'a str>,
    ext: &'a str,
}

fn parse_maven(name: &str) -> Option<MavenName<'_>> {
    let (coords, ext) = match name.split_once('@') {
        Some((c, e)) => (c, e),
        None => (name, "jar"),
    };
    let mut it = coords.split(':');
    let group = it.next()?;
    let artifact = it.next()?;
    let version = it.next()?;
    let classifier = it.next();
    Some(MavenName {
        group,
        artifact,
        version,
        classifier,
        ext,
    })
}

/// `group:artifact:version[:classifier][@ext]` → maven repository path.
pub(crate) fn maven_path(name: &str, classifier: Option<&str>) -> Option<String> {
    let m = parse_maven(name)?;
    let classifier = classifier.or(m.classifier);
    let file = match classifier {
        Some(c) => format!("{}-{}-{c}.{}", m.artifact, m.version, m.ext),
        None => format!("{}-{}.{}", m.artifact, m.version, m.ext),
    };
    Some(format!(
        "{}/{}/{}/{file}",
        m.group.replace('.', "/"),
        m.artifact,
        m.version
    ))
}

/// Identity used to let a child profile override a parent library.
fn lib_key(name: &str) -> String {
    match parse_maven(name) {
        Some(m) => format!("{}:{}:{}", m.group, m.artifact, m.classifier.unwrap_or("")),
        None => name.to_owned(),
    }
}

fn join_repo(base: &str, path: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

fn non_empty(url: &str) -> Option<String> {
    (!url.is_empty()).then(|| url.to_owned())
}

/// Files of one library for `env` (empty when its rules exclude this platform).
pub(crate) fn resolve_library(lib: &Library, env: &Env) -> Vec<LibFile> {
    if !rules_allow(lib.rules.as_deref().unwrap_or(&[]), env) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let repo = lib.url.as_deref().unwrap_or(LIBRARIES_URL);

    // Old-style natives (LWJGL2, LWJGL3 before 1.19): classifier per OS.
    let mut has_natives = false;
    if let Some(classifier) = lib.natives.as_ref().and_then(|n| n.get(env.os)) {
        has_natives = true;
        let classifier = classifier.replace("${arch}", env.bits());
        let exclude = lib
            .extract
            .as_ref()
            .map(|e| e.exclude.clone())
            .unwrap_or_default();
        let from_downloads = lib
            .downloads
            .as_ref()
            .and_then(|d| d.classifiers.as_ref())
            .and_then(|c| c.get(&classifier));
        let file = match from_downloads {
            Some(a) => a
                .path
                .clone()
                .or_else(|| maven_path(&lib.name, Some(&classifier)))
                .map(|path| LibFile {
                    path,
                    url: non_empty(&a.url),
                    sha1: a.sha1.clone(),
                    size: a.size,
                    usage: LibUse::Native {
                        exclude: exclude.clone(),
                    },
                }),
            None => maven_path(&lib.name, Some(&classifier)).map(|path| LibFile {
                url: Some(join_repo(repo, &path)),
                path,
                sha1: None,
                size: None,
                usage: LibUse::Native {
                    exclude: exclude.clone(),
                },
            }),
        };
        out.extend(file);
    }

    match lib.downloads.as_ref() {
        Some(d) => {
            if let Some(a) = &d.artifact {
                if let Some(path) = a.path.clone().or_else(|| maven_path(&lib.name, None)) {
                    out.push(LibFile {
                        path,
                        url: non_empty(&a.url),
                        sha1: a.sha1.clone(),
                        size: a.size,
                        usage: LibUse::Classpath,
                    });
                }
            }
        }
        None if !has_natives => {
            if let Some(path) = maven_path(&lib.name, None) {
                out.push(LibFile {
                    url: Some(join_repo(repo, &path)),
                    path,
                    sha1: lib.sha1.clone(),
                    size: lib.size,
                    usage: LibUse::Classpath,
                });
            }
        }
        None => {}
    }
    out
}

/// All library files for `env`, first occurrence of a `group:artifact:classifier` wins.
pub(crate) fn resolve_libraries(libs: &[Library], env: &Env) -> Vec<LibFile> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for lib in libs {
        let files = resolve_library(lib, env);
        if files.is_empty() {
            continue;
        }
        // Old jsons list a library twice: plain and with `natives`; keep both.
        let key = format!("{}#{}", lib_key(&lib.name), lib.natives.is_some());
        if !seen.insert(key) {
            continue;
        }
        out.extend(files);
    }
    out
}

// ---------------------------------------------------------------------------
// inheritsFrom
// ---------------------------------------------------------------------------

/// Merges a loader profile over its parent. Child libraries come first so they
/// override the parent's copies; arguments are appended.
pub(crate) fn merge_inherited(child: VersionJson, parent: VersionJson) -> VersionJson {
    let mut libraries = child.libraries;
    libraries.extend(parent.libraries);
    let arguments = match (parent.arguments, child.arguments) {
        (Some(mut p), Some(c)) => {
            p.game.extend(c.game);
            p.jvm.extend(c.jvm);
            Some(p)
        }
        (p, c) => c.or(p),
    };
    VersionJson {
        id: child.id,
        inherits_from: parent.inherits_from,
        kind: child.kind.or(parent.kind),
        main_class: child.main_class.or(parent.main_class),
        minecraft_arguments: child.minecraft_arguments.or(parent.minecraft_arguments),
        arguments,
        libraries,
        asset_index: child.asset_index.or(parent.asset_index),
        assets: child.assets.or(parent.assets),
        downloads: child.downloads.or(parent.downloads),
        java_version: child.java_version.or(parent.java_version),
        logging: child.logging.or(parent.logging),
        jar: child.jar.or(parent.jar).or(Some(parent.id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(os: &'static str, arch: &'static str) -> Env {
        Env {
            os,
            arch,
            os_version: "10.0".into(),
            features: Features::default(),
        }
    }

    fn fixture(name: &str) -> VersionJson {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
    }

    #[test]
    fn maven_paths() {
        assert_eq!(
            maven_path("net.fabricmc:fabric-loader:0.16.9", None).unwrap(),
            "net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar"
        );
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.3:natives-windows", None).unwrap(),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"
        );
        assert_eq!(
            maven_path("a.b:c:1@zip", Some("x")).unwrap(),
            "a/b/c/1/c-1-x.zip"
        );
    }

    #[test]
    fn rules_semantics() {
        let allow_all_but_osx: Vec<Rule> = serde_json::from_str(
            r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#,
        )
        .unwrap();
        assert!(rules_allow(&allow_all_but_osx, &env("windows", "x86_64")));
        assert!(rules_allow(&allow_all_but_osx, &env("linux", "x86_64")));
        assert!(!rules_allow(&allow_all_but_osx, &env("osx", "aarch64")));

        let only_x86: Vec<Rule> =
            serde_json::from_str(r#"[{"action":"allow","os":{"arch":"x86"}}]"#).unwrap();
        assert!(rules_allow(&only_x86, &env("windows", "x86")));
        assert!(!rules_allow(&only_x86, &env("windows", "x86_64")));

        let win10: Vec<Rule> = serde_json::from_str(
            r#"[{"action":"allow","os":{"name":"windows","version":"^10\\."}}]"#,
        )
        .unwrap();
        assert!(rules_allow(&win10, &env("windows", "x86_64")));

        let feature: Vec<Rule> =
            serde_json::from_str(r#"[{"action":"allow","features":{"is_demo_user":true}}]"#)
                .unwrap();
        assert!(!rules_allow(&feature, &env("windows", "x86_64")));
        assert!(rules_allow(&[], &env("linux", "x86_64")));
    }

    #[test]
    fn legacy_natives_1_12_2() {
        let v = fixture("1.12.2.json");
        let files = resolve_libraries(&v.libraries, &env("windows", "x86_64"));
        let natives: Vec<_> = files
            .iter()
            .filter(|f| matches!(f.usage, LibUse::Native { .. }))
            .map(|f| f.path.as_str())
            .collect();
        assert!(natives.contains(
            &"org/lwjgl/lwjgl/lwjgl-platform/2.9.4-nightly-20150209/lwjgl-platform-2.9.4-nightly-20150209-natives-windows.jar"
        ));
        assert!(natives.contains(
            &"net/java/jinput/jinput-platform/2.0.5/jinput-platform-2.0.5-natives-windows.jar"
        ));
        // osx-only lwjgl 2.9.2 must be excluded, 2.9.4 included.
        assert!(!files.iter().any(|f| f.path.contains("2.9.2-nightly")));
        assert!(files.iter().any(
            |f| f.path.contains("lwjgl/2.9.4-nightly-20150209/lwjgl-2.9.4")
                && f.usage == LibUse::Classpath
        ));
        // Excludes carried over.
        let excl = files
            .iter()
            .find_map(|f| match &f.usage {
                LibUse::Native { exclude } => Some(exclude.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(excl, vec!["META-INF/".to_string()]);
        // java-objc-bridge is osx-only.
        assert!(!files.iter().any(|f| f.path.contains("java-objc-bridge")));

        let linux = resolve_libraries(&v.libraries, &env("linux", "x86_64"));
        assert!(linux
            .iter()
            .any(|f| f.path.ends_with("text2speech-1.10.3-natives-linux.jar")));
    }

    #[test]
    fn modern_natives_1_21_1() {
        let v = fixture("1.21.1.json");
        let win = resolve_libraries(&v.libraries, &env("windows", "x86_64"));
        assert!(win.iter().all(|f| f.usage == LibUse::Classpath));
        assert!(win
            .iter()
            .any(|f| f.path.ends_with("lwjgl-3.3.3-natives-windows.jar")));
        assert!(!win.iter().any(|f| f.path.contains("natives-linux")));
        assert!(!win.iter().any(|f| f.path.contains("natives-macos")));
        let linux = resolve_libraries(&v.libraries, &env("linux", "x86_64"));
        assert!(linux
            .iter()
            .any(|f| f.path.ends_with("lwjgl-3.3.3-natives-linux.jar")));
        assert!(!linux.iter().any(|f| f.path.contains("natives-windows")));
    }

    #[test]
    fn arch_placeholder_in_natives() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"tv.twitch:twitch-platform:5.16","natives":{"windows":"natives-windows-${arch}"}}"#,
        )
        .unwrap();
        let files = resolve_library(&lib, &env("windows", "x86_64"));
        assert_eq!(files.len(), 1);
        assert!(files[0]
            .path
            .ends_with("twitch-platform-5.16-natives-windows-64.jar"));
        assert!(files[0].url.as_deref().unwrap().starts_with(LIBRARIES_URL));
    }

    #[test]
    fn inherits_from_merge_overrides_libraries() {
        let parent = fixture("1.21.1.json");
        let child: VersionJson = serde_json::from_str(
            r#"{"id":"fabric-loader-0.16.9-1.21.1","inheritsFrom":"1.21.1",
                "mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient",
                "arguments":{"game":[],"jvm":["-DFabricMcEmu= net.minecraft.client.main.Main "]},
                "libraries":[{"name":"org.ow2.asm:asm:9.7.1","url":"https://maven.fabricmc.net/"}]}"#,
        )
        .unwrap();
        let merged = merge_inherited(child, parent);
        assert_eq!(merged.jar.as_deref(), Some("1.21.1"));
        assert_eq!(merged.java_version.unwrap().major_version, 21);
        assert!(merged.main_class.unwrap().contains("Knot"));
        let files = resolve_libraries(&merged.libraries, &env("windows", "x86_64"));
        let asm: Vec<_> = files
            .iter()
            .filter(|f| f.path.starts_with("org/ow2/asm/asm/"))
            .collect();
        assert_eq!(asm.len(), 1);
        assert!(asm[0].path.contains("9.7.1"));
        assert!(merged
            .arguments
            .unwrap()
            .jvm
            .iter()
            .any(|a| matches!(a, Argument::Plain(s) if s.contains("FabricMcEmu"))));
    }
}
