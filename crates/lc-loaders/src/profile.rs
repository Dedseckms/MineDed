//! Forge / NeoForge `install_profile.json`: parsing and processor argument expansion.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lc_types::AppResult;
use serde::Deserialize;

use crate::libs::Library;
use crate::util::{loader_err, maven_path, safe_join};

/// Modern installer profile (spec ≥ 0 with `processors`, Forge 1.13+ / NeoForge).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstallProfile {
    pub minecraft: String,
    /// Path of the version json inside the installer (`/version.json`).
    #[serde(default = "default_json")]
    pub json: String,
    #[serde(default)]
    pub data: BTreeMap<String, DataEntry>,
    #[serde(default)]
    pub processors: Vec<Processor>,
    #[serde(default)]
    pub libraries: Vec<Library>,
}

fn default_json() -> String {
    "/version.json".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DataEntry {
    pub client: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Processor {
    #[serde(default)]
    pub sides: Option<Vec<String>>,
    pub jar: String,
    #[serde(default)]
    pub classpath: Vec<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub outputs: BTreeMap<String, String>,
}

impl Processor {
    pub fn runs_on_client(&self) -> bool {
        self.sides
            .as_ref()
            .map(|s| s.iter().any(|x| x == "client"))
            .unwrap_or(true)
    }

    /// `installertools --task DOWNLOAD_MOJMAPS`: the launcher downloads the
    /// mappings itself through `Downloader` instead of letting Java hit the network.
    pub fn mojmaps_output(&self) -> Option<&str> {
        if !self.args.iter().any(|a| a == "DOWNLOAD_MOJMAPS") {
            return None;
        }
        let pos = self.args.iter().position(|a| a == "--output")?;
        self.args.get(pos + 1).map(String::as_str)
    }
}

/// Legacy installer (Forge ≤ 1.12.2): `install` + embedded `versionInfo`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LegacyProfile {
    pub install: LegacyInstall,
    pub version_info: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LegacyInstall {
    /// Maven coords of the universal jar.
    pub path: String,
    /// Universal jar file inside the installer.
    pub file_path: String,
    pub minecraft: String,
}

pub(crate) enum AnyProfile {
    Modern(Box<InstallProfile>),
    Legacy(Box<LegacyProfile>),
}

pub(crate) fn parse_profile(bytes: &[u8]) -> AppResult<AnyProfile> {
    let v: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| loader_err(format!("bad install_profile.json: {e}")))?;
    if v.get("install").is_some() && v.get("versionInfo").is_some() {
        let p: LegacyProfile = serde_json::from_value(v)
            .map_err(|e| loader_err(format!("bad legacy install profile: {e}")))?;
        Ok(AnyProfile::Legacy(Box::new(p)))
    } else {
        let p: InstallProfile = serde_json::from_value(v)
            .map_err(|e| loader_err(format!("bad install profile: {e}")))?;
        Ok(AnyProfile::Modern(Box::new(p)))
    }
}

/// Absolute path of a maven artifact under `libraries`.
pub(crate) fn artifact_path(libraries: &Path, coords: &str) -> AppResult<PathBuf> {
    let rel = maven_path(coords)
        .ok_or_else(|| loader_err("bad maven coordinates").detail("coords", coords))?;
    safe_join(libraries, &rel)
}

/// Resolves a `data` value: `[coords]` → library path, `'literal'` → literal,
/// `/inside/installer` → path where `extract` put the file, anything else as is.
pub(crate) fn resolve_data_value(
    value: &str,
    libraries: &Path,
    extract: &mut dyn FnMut(&str) -> AppResult<PathBuf>,
) -> AppResult<String> {
    if let Some(coords) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        return Ok(artifact_path(libraries, coords)?
            .to_string_lossy()
            .into_owned());
    }
    if let Some(lit) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return Ok(lit.to_owned());
    }
    if value.starts_with('/') {
        return Ok(extract(value)?.to_string_lossy().into_owned());
    }
    Ok(value.to_owned())
}

/// Expands one processor argument: `[coords]` → library path, `{KEY}` → data.
pub(crate) fn expand_arg(
    arg: &str,
    data: &BTreeMap<String, String>,
    libraries: &Path,
) -> AppResult<String> {
    if let Some(coords) = arg.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        return Ok(artifact_path(libraries, coords)?
            .to_string_lossy()
            .into_owned());
    }
    let mut out = String::with_capacity(arg.len());
    let mut rest = arg;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let key = &after[..end];
        let val = data
            .get(key)
            .ok_or_else(|| loader_err("unknown processor variable").detail("key", key))?;
        out.push_str(val);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Rewrites the legacy `versionInfo` for the launcher: drops server-only
/// libraries, upgrades the old Forge maven URL and marks the universal jar
/// (extracted from the installer) as not downloadable.
pub(crate) fn legacy_version_json(
    p: &LegacyProfile,
    universal_sha1: &str,
) -> AppResult<(String, serde_json::Value)> {
    let mut v = p.version_info.clone();
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .map(str::to_owned)
        .ok_or_else(|| loader_err("legacy versionInfo has no id"))?;
    let universal_rel = maven_path(&p.install.path)
        .ok_or_else(|| loader_err("bad universal coords").detail("coords", &p.install.path))?;
    let obj = v
        .as_object_mut()
        .ok_or_else(|| loader_err("legacy versionInfo is not an object"))?;
    obj.insert("inheritsFrom".into(), p.install.minecraft.clone().into());
    obj.entry("jar")
        .or_insert_with(|| p.install.minecraft.clone().into());
    let libs = obj
        .get("libraries")
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for mut lib in libs {
        let Some(o) = lib.as_object_mut() else {
            continue;
        };
        if o.get("clientreq").and_then(|c| c.as_bool()) == Some(false) {
            continue;
        }
        o.remove("clientreq");
        o.remove("serverreq");
        o.remove("checksums");
        let name = o
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_owned();
        if name == p.install.path {
            o.remove("url");
            o.insert(
                "downloads".into(),
                serde_json::json!({"artifact": {
                    "path": universal_rel,
                    "url": "",
                    "sha1": universal_sha1,
                }}),
            );
        } else if let Some(url) = o.get("url").and_then(|u| u.as_str()) {
            if url.contains("files.minecraftforge.net") {
                o.insert("url".into(), "https://maven.minecraftforge.net/".into());
            }
        }
        out.push(lib);
    }
    obj.insert("libraries".into(), serde_json::Value::Array(out));
    Ok((id, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn expands_args() {
        let libs = Path::new("/libs");
        let d = data(&[("SIDE", "client"), ("MINECRAFT_JAR", "/v/1.20.1.jar")]);
        assert_eq!(expand_arg("{SIDE}", &d, libs).unwrap(), "client");
        assert_eq!(
            expand_arg("--x={MINECRAFT_JAR}", &d, libs).unwrap(),
            "--x=/v/1.20.1.jar"
        );
        let p = expand_arg("[net.minecraft:client:1.20.1:srg]", &d, libs).unwrap();
        assert!(Path::new(&p).ends_with("net/minecraft/client/1.20.1/client-1.20.1-srg.jar"));
        assert!(expand_arg("{NOPE}", &d, libs).is_err());
        assert_eq!(expand_arg("plain", &d, libs).unwrap(), "plain");
    }

    #[test]
    fn modern_profile_fixture() {
        let AnyProfile::Modern(p) = parse_profile(include_bytes!(
            "../tests/fixtures/forge-1.20.1-install_profile.json"
        ))
        .unwrap() else {
            panic!("expected modern profile");
        };
        assert_eq!(p.minecraft, "1.20.1");
        assert_eq!(p.json, "/version.json");
        let client: Vec<_> = p.processors.iter().filter(|x| x.runs_on_client()).collect();
        assert_eq!(client.len(), 2);
        assert_eq!(client[0].mojmaps_output(), Some("{MOJMAPS}"));
        assert_eq!(client[1].mojmaps_output(), None);
        assert_eq!(p.libraries.len(), 2);

        // Data resolution + expansion of the binarypatcher arguments.
        let libs = Path::new("/libs");
        let mut ex = |x: &str| Ok(PathBuf::from("/work").join(x.trim_start_matches('/')));
        let mut data = BTreeMap::new();
        for (k, v) in &p.data {
            data.insert(
                k.clone(),
                resolve_data_value(&v.client, libs, &mut ex).unwrap(),
            );
        }
        data.insert("MINECRAFT_JAR".into(), "/v/1.20.1.jar".into());
        let args: Vec<String> = client[1]
            .args
            .iter()
            .map(|a| expand_arg(a, &data, libs).unwrap())
            .collect();
        assert_eq!(args[1], "/v/1.20.1.jar");
        assert!(Path::new(&args[3])
            .ends_with("net/minecraftforge/forge/1.20.1-47.3.0/forge-1.20.1-47.3.0-client.jar"));
        assert!(Path::new(&args[5]).ends_with("data/client.lzma"));
        let (out, sha) = client[1].outputs.iter().next().unwrap();
        assert_eq!(
            expand_arg(sha, &data, libs).unwrap(),
            "51bc8a1ab4a51d4bdcd2d81b2bd5b9a6c1e5e4b8"
        );
        assert!(expand_arg(out, &data, libs)
            .unwrap()
            .ends_with("client.jar"));
    }

    #[test]
    fn legacy_profile_fixture() {
        let AnyProfile::Legacy(p) = parse_profile(include_bytes!(
            "../tests/fixtures/forge-1.12.2-install_profile.json"
        ))
        .unwrap() else {
            panic!("expected legacy profile");
        };
        let (id, v) = legacy_version_json(&p, "abc").unwrap();
        assert_eq!(id, "1.12.2-forge-14.23.5.2860");
        assert_eq!(v["inheritsFrom"], "1.12.2");
        let libs = v["libraries"].as_array().unwrap();
        // jline is server-only.
        assert_eq!(libs.len(), 3);
        assert_eq!(libs[0]["downloads"]["artifact"]["url"], "");
        assert_eq!(libs[0]["downloads"]["artifact"]["sha1"], "abc");
        assert_eq!(libs[2]["url"], "https://maven.minecraftforge.net/");
        assert!(libs[2].get("checksums").is_none());
        let parsed: Vec<Library> = serde_json::from_value(v["libraries"].clone()).unwrap();
        assert!(parsed[0].file().unwrap().url.is_none());
        assert!(parsed[1]
            .file()
            .unwrap()
            .url
            .unwrap()
            .starts_with("https://libraries.minecraft.net/"));
    }

    #[test]
    fn resolves_data_values() {
        let libs = Path::new("/libs");
        let mut extracted = Vec::new();
        let mut ex = |p: &str| {
            extracted.push(p.to_owned());
            Ok(PathBuf::from("/tmp").join(p.trim_start_matches('/')))
        };
        assert_eq!(resolve_data_value("'abc'", libs, &mut ex).unwrap(), "abc");
        assert!(resolve_data_value("[a:b:1@txt]", libs, &mut ex)
            .unwrap()
            .ends_with("b-1.txt"));
        assert!(resolve_data_value("/data/client.lzma", libs, &mut ex)
            .unwrap()
            .ends_with("client.lzma"));
        assert_eq!(extracted, vec!["/data/client.lzma"]);
    }
}
