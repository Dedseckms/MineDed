//! Dependency check of local mod jars (`ContentService::check_local`).
//!
//! Reads `fabric.mod.json`, `quilt.mod.json`, `META-INF/mods.toml` and
//! `META-INF/neoforge.mods.toml` (plus ids of jar-in-jar libraries) and reports
//! missing / wrong-version / conflicting dependencies and jars built for another
//! loader or Minecraft version. Optional dependencies are ignored; version
//! requirements that cannot be evaluated are never reported.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use lc_types::content::{
    ContentKind, ContentSourceKind, ContentVersion, ContentVersionRef, LocalDepIssue,
    LocalDepIssueKind, SearchQuery, SortBy,
};
use lc_types::version::LoaderKind;
use lc_types::{AppError, AppResult, ErrorCode, InstanceId};
use serde::Deserialize;

use crate::contract::ContentSource;
use crate::index::{base_name, ContentIndex};
use crate::modmeta::{compare_versions, parse_fabric_mod_json, satisfies};
use crate::service::{target_of, DiskFile, Inner};
use crate::source::{Catalog, Target};
use crate::util::{check_file_name, join, loader_compatible};

/// Max size of a metadata file we read.
const MAX_ENTRY: u64 = 1024 * 1024;
/// Max size of a nested jar we unpack into memory.
const MAX_NESTED: u64 = 32 * 1024 * 1024;
/// Jar-in-jar nesting we follow.
const MAX_DEPTH: usize = 2;

/// Metadata format found in a jar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Fabric,
    Quilt,
    ModsToml,
    NeoToml,
}

impl Format {
    fn loader_name(self) -> &'static str {
        match self {
            Format::Fabric => "fabric",
            Format::Quilt => "quilt",
            Format::ModsToml => "forge",
            Format::NeoToml => "neoforge",
        }
    }
}

/// Version requirement of a dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Req {
    Any,
    /// Fabric/Quilt predicates (OR).
    Preds(Vec<String>),
    /// Maven version range (Forge/NeoForge).
    Maven(String),
    /// Written in a form we do not evaluate.
    Opaque(String),
}

impl Req {
    /// Requirement as written (`None` = any version).
    pub fn display(&self) -> Option<String> {
        match self {
            Req::Any => None,
            Req::Preds(p) => {
                let s = p.join(" || ");
                (!s.is_empty() && s != "*").then_some(s)
            }
            Req::Maven(s) | Req::Opaque(s) => {
                let s = s.trim();
                (!s.is_empty() && s != "*").then(|| s.to_owned())
            }
        }
    }

    /// `None` = cannot tell.
    pub fn matches(&self, version: &str, mc: &str) -> Option<bool> {
        match self {
            Req::Any => Some(true),
            Req::Preds(p) => preds_match(p, version),
            Req::Maven(r) => maven_matches(r, version, mc),
            Req::Opaque(_) => None,
        }
    }
}

/// A required or incompatible dependency; `ids` are alternatives (any one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dep {
    pub ids: Vec<String>,
    pub req: Req,
}

/// One metadata file of a jar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Meta {
    pub format: Format,
    /// Provided mod ids with their versions (`None` = unknown).
    pub ids: Vec<(String, Option<String>)>,
    pub depends: Vec<Dep>,
    pub breaks: Vec<Dep>,
}

/// Everything read from a mod jar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct JarInfo {
    pub metas: Vec<Meta>,
    /// Ids provided by bundled (jar-in-jar) libraries.
    pub nested_ids: Vec<(String, Option<String>)>,
}

/// Instance the jars are checked against.
#[derive(Debug, Clone)]
pub(crate) struct Ctx {
    pub mc: String,
    pub loader: LoaderKind,
    pub loader_version: Option<String>,
}

/// Mod id → versions of every provider among the enabled jars.
pub(crate) type Installed = HashMap<String, Vec<Option<String>>>;

// ---------------------------------------------------------------- reading

/// Reads metadata of a jar on disk (blocking). Unreadable jar → empty info.
pub(crate) fn read_jar_file(path: &Path) -> JarInfo {
    match std::fs::File::open(path) {
        Ok(f) => read_jar(std::io::BufReader::new(f), 0),
        Err(_) => JarInfo::default(),
    }
}

fn read_entry<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    max: u64,
) -> Option<Vec<u8>> {
    let entry = zip.by_name(name).ok()?;
    if entry.size() > max {
        return None;
    }
    let mut buf = Vec::new();
    entry.take(max).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Reads metadata from a jar (`depth` = jar-in-jar nesting level).
pub(crate) fn read_jar<R: Read + Seek>(reader: R, depth: usize) -> JarInfo {
    let Ok(mut zip) = zip::ZipArchive::new(reader) else {
        return JarInfo::default();
    };
    let jar_version = read_entry(&mut zip, "META-INF/MANIFEST.MF", MAX_ENTRY)
        .and_then(|b| manifest_version(&String::from_utf8_lossy(&b)));
    let mut info = JarInfo::default();
    if let Some(m) =
        read_entry(&mut zip, "fabric.mod.json", MAX_ENTRY).and_then(|b| parse_fabric(&b))
    {
        info.metas.push(m);
    }
    if let Some(m) = read_entry(&mut zip, "quilt.mod.json", MAX_ENTRY).and_then(|b| parse_quilt(&b))
    {
        info.metas.push(m);
    }
    for (name, format) in [
        ("META-INF/neoforge.mods.toml", Format::NeoToml),
        ("META-INF/mods.toml", Format::ModsToml),
    ] {
        if let Some(m) = read_entry(&mut zip, name, MAX_ENTRY)
            .and_then(|b| parse_mods_toml(&b, format, jar_version.as_deref()))
        {
            info.metas.push(m);
        }
    }
    if depth < MAX_DEPTH {
        let nested: Vec<String> = zip
            .file_names()
            .filter(|n| {
                (n.starts_with("META-INF/jars/") || n.starts_with("META-INF/jarjar/"))
                    && n.to_ascii_lowercase().ends_with(".jar")
            })
            .map(str::to_owned)
            .collect();
        for name in nested {
            let Some(bytes) = read_entry(&mut zip, &name, MAX_NESTED) else {
                continue;
            };
            let inner = read_jar(Cursor::new(bytes), depth + 1);
            for m in inner.metas {
                info.nested_ids.extend(m.ids);
            }
            info.nested_ids.extend(inner.nested_ids);
        }
    }
    info
}

fn manifest_version(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("Implementation-Version:"))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// Drops unresolved build placeholders (`${version}`).
fn real_version(v: &str) -> Option<String> {
    let v = v.trim();
    (!v.is_empty() && !v.contains("${")).then(|| v.to_owned())
}

fn parse_fabric(bytes: &[u8]) -> Option<Meta> {
    let m = parse_fabric_mod_json(bytes)?;
    let version = real_version(&m.version);
    let mut ids = vec![(m.id, version.clone())];
    ids.extend(m.provides.into_iter().map(|p| (p, version.clone())));
    let conv = |map: std::collections::BTreeMap<String, Vec<String>>| {
        map.into_iter()
            .map(|(id, preds)| Dep {
                ids: vec![id],
                req: Req::Preds(preds),
            })
            .collect()
    };
    Some(Meta {
        format: Format::Fabric,
        ids,
        depends: conv(m.depends),
        breaks: conv(m.breaks),
    })
}

#[derive(Deserialize)]
struct QuiltRoot {
    quilt_loader: QuiltLoader,
}

#[derive(Deserialize)]
struct QuiltLoader {
    id: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    provides: Vec<QProvide>,
    #[serde(default)]
    depends: Vec<QDep>,
    #[serde(default)]
    breaks: Vec<QDep>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum QProvide {
    Id(String),
    Obj {
        id: String,
        #[serde(default)]
        version: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum QDep {
    Id(String),
    Obj(QDepObj),
    AnyOf(Vec<QDep>),
}

#[derive(Deserialize)]
struct QDepObj {
    id: String,
    #[serde(default)]
    versions: Option<serde_json::Value>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    unless: Option<serde_json::Value>,
}

/// `maven.group:id` → `id`.
fn quilt_id(id: &str) -> String {
    id.rsplit(':').next().unwrap_or(id).to_owned()
}

fn quilt_req(v: Option<&serde_json::Value>) -> Req {
    match v {
        None => Req::Any,
        Some(serde_json::Value::String(s)) => Req::Preds(vec![s.clone()]),
        Some(serde_json::Value::Array(a)) => {
            let preds: Option<Vec<String>> =
                a.iter().map(|x| x.as_str().map(str::to_owned)).collect();
            preds.map_or_else(
                || Req::Opaque(v.map(|x| x.to_string()).unwrap_or_default()),
                Req::Preds,
            )
        }
        Some(other) => Req::Opaque(other.to_string()),
    }
}

/// Mandatory, unconditional entry → `Dep`; optional/conditional → `None`.
fn quilt_dep(d: &QDep) -> Option<Dep> {
    match d {
        QDep::Id(id) => Some(Dep {
            ids: vec![quilt_id(id)],
            req: Req::Any,
        }),
        QDep::Obj(o) => (!o.optional && o.unless.is_none()).then(|| Dep {
            ids: vec![quilt_id(&o.id)],
            req: quilt_req(o.versions.as_ref()),
        }),
        QDep::AnyOf(alts) => {
            let deps: Option<Vec<Dep>> = alts.iter().map(quilt_dep).collect();
            let deps = deps?;
            if deps.is_empty() {
                return None;
            }
            // Versions of alternatives are not compared (best effort).
            Some(Dep {
                ids: deps.into_iter().flat_map(|d| d.ids).collect(),
                req: Req::Any,
            })
        }
    }
}

fn parse_quilt(bytes: &[u8]) -> Option<Meta> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let root: QuiltRoot = serde_json::from_slice(bytes).ok()?;
    let q = root.quilt_loader;
    let version = real_version(&q.version);
    let mut ids = vec![(quilt_id(&q.id), version.clone())];
    for p in &q.provides {
        ids.push(match p {
            QProvide::Id(id) => (quilt_id(id), version.clone()),
            QProvide::Obj { id, version: v } => (
                quilt_id(id),
                v.as_deref()
                    .and_then(real_version)
                    .or_else(|| version.clone()),
            ),
        });
    }
    Some(Meta {
        format: Format::Quilt,
        ids,
        depends: q.depends.iter().filter_map(quilt_dep).collect(),
        breaks: q
            .breaks
            .iter()
            .filter_map(|d| match d {
                // `optional` has no meaning for breaks: every entry counts.
                QDep::Obj(o) if o.unless.is_none() => Some(Dep {
                    ids: vec![quilt_id(&o.id)],
                    req: quilt_req(o.versions.as_ref()),
                }),
                other => quilt_dep(other),
            })
            .collect(),
    })
}

#[derive(Deserialize)]
struct TomlMod {
    #[serde(rename = "modId")]
    mod_id: String,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Deserialize)]
struct TomlDep {
    #[serde(rename = "modId")]
    mod_id: String,
    #[serde(default)]
    mandatory: Option<bool>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default, rename = "versionRange")]
    version_range: Option<String>,
    #[serde(default)]
    side: Option<String>,
}

fn parse_mods_toml(bytes: &[u8], format: Format, jar_version: Option<&str>) -> Option<Meta> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).ok()?;
    let table: toml::Table = toml::from_str(text).ok()?;
    let mut ids = Vec::new();
    if let Some(toml::Value::Array(mods)) = table.get("mods") {
        for m in mods {
            let Ok(m) = m.clone().try_into::<TomlMod>() else {
                continue;
            };
            let version = match m.version.as_deref() {
                Some("${file.jarVersion}") => jar_version.and_then(real_version),
                Some(v) => real_version(v),
                None => jar_version.and_then(real_version),
            };
            ids.push((m.mod_id, version));
        }
    }
    if ids.is_empty() {
        return None;
    }
    let mut depends = Vec::new();
    let mut breaks = Vec::new();
    if let Some(toml::Value::Table(deps)) = table.get("dependencies") {
        for list in deps.values() {
            let toml::Value::Array(list) = list else {
                continue;
            };
            for d in list {
                let Ok(d) = d.clone().try_into::<TomlDep>() else {
                    continue;
                };
                if d.side
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case("server"))
                {
                    continue;
                }
                let kind = d.kind.as_deref().map(str::to_ascii_lowercase);
                let dep = Dep {
                    ids: vec![d.mod_id],
                    req: d
                        .version_range
                        .map_or(Req::Any, |r| Req::Maven(r.trim().to_owned())),
                };
                match kind.as_deref() {
                    Some("required") => depends.push(dep),
                    Some("incompatible") => breaks.push(dep),
                    Some(_) => {}
                    None if d.mandatory == Some(true) => depends.push(dep),
                    None => {}
                }
            }
        }
    }
    Some(Meta {
        format,
        ids,
        depends,
        breaks,
    })
}

// ---------------------------------------------------------------- versions

/// Fabric/Quilt predicates; unparseable version → only literal matches count.
fn preds_match(preds: &[String], version: &str) -> Option<bool> {
    if preds
        .iter()
        .any(|p| p.trim() == "*" || p.trim() == version.trim())
    {
        return Some(true);
    }
    compare_versions(version, version)?;
    satisfies(preds, version)
}

/// Strips `<mc>-` / `mc<mc>-` prefixes and build metadata; versions with
/// other dash-number parts (`1.20.1-4.2`) are ambiguous → `None`.
fn maven_norm(version: &str, mc: &str) -> Option<String> {
    let mut v = version.trim();
    for p in [format!("{mc}-"), format!("mc{mc}-")] {
        if let Some(rest) = v.strip_prefix(p.as_str()) {
            v = rest;
        }
    }
    let v = v.split('+').next().unwrap_or(v);
    if let Some((_, pre)) = v.split_once('-') {
        if pre.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
    }
    compare_versions(v, v)?;
    Some(v.to_owned())
}

/// Maven range (`[1.0,2.0)`, `[1.0,)`, `(,1.0]`, `[1.0]`, unions with `,`).
/// A bare version is only a recommendation and matches anything.
fn maven_matches(range: &str, version: &str, mc: &str) -> Option<bool> {
    let r = range.trim();
    if r.is_empty() || r == "*" || !r.starts_with(['[', '(']) {
        return Some(true);
    }
    let v = maven_norm(version, mc)?;
    let mut rest = r;
    let mut known = false;
    loop {
        rest = rest.trim_start().trim_start_matches(',').trim_start();
        if rest.is_empty() {
            break;
        }
        let lo_inc = match rest.chars().next() {
            Some('[') => true,
            Some('(') => false,
            _ => return None,
        };
        let end = rest.find([']', ')'])?;
        let hi_inc = rest[end..].starts_with(']');
        let inner = &rest[1..end];
        rest = &rest[end + 1..];
        let hit = match inner.split_once(',') {
            None => compare_versions(&v, inner.trim()).map(|o| o == Ordering::Equal),
            Some((lo, hi)) => {
                let (lo, hi) = (lo.trim(), hi.trim());
                let lo_ok = if lo.is_empty() {
                    Some(true)
                } else {
                    compare_versions(&v, lo)
                        .map(|o| o == Ordering::Greater || (lo_inc && o.is_eq()))
                };
                let hi_ok = if hi.is_empty() {
                    Some(true)
                } else {
                    compare_versions(&v, hi).map(|o| o == Ordering::Less || (hi_inc && o.is_eq()))
                };
                match (lo_ok, hi_ok) {
                    (Some(a), Some(b)) => Some(a && b),
                    (Some(false), None) | (None, Some(false)) => Some(false),
                    _ => None,
                }
            }
        };
        match hit {
            Some(true) => return Some(true),
            Some(false) => known = true,
            None => return None,
        }
    }
    known.then_some(false)
}

// ---------------------------------------------------------------- analysis

/// Ids provided by every enabled jar (all formats and jar-in-jar libraries).
pub(crate) fn installed_ids<'a>(jars: impl IntoIterator<Item = &'a JarInfo>) -> Installed {
    let mut out: Installed = HashMap::new();
    for j in jars {
        let ids = j
            .metas
            .iter()
            .flat_map(|m| m.ids.iter())
            .chain(&j.nested_ids);
        for (id, v) in ids {
            out.entry(id.clone()).or_default().push(v.clone());
        }
    }
    out
}

/// Metadata formats the instance loader reads, in preference order.
fn accepted(loader: LoaderKind) -> &'static [Format] {
    match loader {
        LoaderKind::Fabric => &[Format::Fabric],
        LoaderKind::Quilt => &[Format::Quilt, Format::Fabric],
        LoaderKind::Forge => &[Format::ModsToml],
        LoaderKind::NeoForge => &[Format::NeoToml, Format::ModsToml],
        _ => &[],
    }
}

enum Builtin {
    /// Checked against the instance Minecraft version.
    Minecraft,
    /// Loader id; checked against the loader version on this loader.
    Loader(LoaderKind),
    /// Not checked (`java`, language providers).
    Ignored,
}

fn builtin(id: &str) -> Option<Builtin> {
    Some(match id {
        "minecraft" => Builtin::Minecraft,
        "fabricloader" | "fabric-loader" => Builtin::Loader(LoaderKind::Fabric),
        "quilt_loader" => Builtin::Loader(LoaderKind::Quilt),
        "forge" => Builtin::Loader(LoaderKind::Forge),
        "neoforge" => Builtin::Loader(LoaderKind::NeoForge),
        "java" | "javafml" | "lowcodefml" | "mcp" => Builtin::Ignored,
        _ => return None,
    })
}

fn issue(file: &str, kind: LocalDepIssueKind, mod_id: &str, req: Option<String>) -> LocalDepIssue {
    LocalDepIssue {
        file_name: file.to_owned(),
        kind,
        mod_id: mod_id.to_owned(),
        version_req: req,
        suggestion: None,
        suggestion_title: None,
    }
}

/// `Some(true)` = the built-in requirement holds, `Some(false)` = violated.
fn builtin_matches(b: &Builtin, req: &Req, ctx: &Ctx) -> Option<bool> {
    match b {
        Builtin::Minecraft => req.matches(&ctx.mc, &ctx.mc),
        Builtin::Loader(kind) if *kind == ctx.loader => {
            let v = ctx.loader_version.as_deref()?;
            req.matches(v, &ctx.mc)
        }
        _ => None,
    }
}

/// Issues of one jar (suggestions are filled in later).
pub(crate) fn check_jar(
    file: &str,
    info: &JarInfo,
    ctx: &Ctx,
    installed: &Installed,
) -> Vec<LocalDepIssue> {
    let mut out: Vec<LocalDepIssue> = Vec::new();
    let mut push = |i: LocalDepIssue| {
        if !out.iter().any(|o| o.kind == i.kind && o.mod_id == i.mod_id) {
            out.push(i);
        }
    };
    if info.metas.is_empty() {
        return Vec::new();
    }
    let Some(meta) = accepted(ctx.loader)
        .iter()
        .find_map(|f| info.metas.iter().find(|m| m.format == *f))
    else {
        let expects = info.metas[0].format.loader_name();
        push(issue(file, LocalDepIssueKind::WrongLoader, expects, None));
        return out;
    };
    // `mods.toml` is shared by Forge and NeoForge 1.20.1: tell them apart by deps.
    if meta.format == Format::ModsToml {
        let wants = |id: &str| meta.depends.iter().any(|d| d.ids.iter().any(|i| i == id));
        if ctx.loader == LoaderKind::NeoForge && wants("forge") && ctx.mc != "1.20.1" {
            push(issue(file, LocalDepIssueKind::WrongLoader, "forge", None));
            return out;
        }
        if ctx.loader == LoaderKind::Forge && wants("neoforge") {
            push(issue(
                file,
                LocalDepIssueKind::WrongLoader,
                "neoforge",
                None,
            ));
            return out;
        }
    }
    let own: Vec<&str> = meta.ids.iter().map(|(id, _)| id.as_str()).collect();

    for d in &meta.depends {
        let Some(first) = d.ids.first() else {
            continue;
        };
        if let Some(b) = builtin(first) {
            if builtin_matches(&b, &d.req, ctx) == Some(false) {
                push(issue(
                    file,
                    LocalDepIssueKind::WrongLoader,
                    first,
                    d.req.display(),
                ));
            }
            continue;
        }
        let versions: Vec<&Option<String>> = d
            .ids
            .iter()
            .filter_map(|id| installed.get(id))
            .flatten()
            .collect();
        if versions.is_empty() {
            push(issue(
                file,
                LocalDepIssueKind::Missing,
                first,
                d.req.display(),
            ));
            continue;
        }
        let ok = versions.iter().any(|v| match v {
            None => true,
            Some(v) => d.req.matches(v, &ctx.mc) != Some(false),
        });
        if !ok {
            push(issue(
                file,
                LocalDepIssueKind::WrongVersion,
                first,
                d.req.display(),
            ));
        }
    }

    for d in &meta.breaks {
        for id in &d.ids {
            if own.contains(&id.as_str()) {
                continue;
            }
            if let Some(b) = builtin(id) {
                if builtin_matches(&b, &d.req, ctx) == Some(true) {
                    push(issue(
                        file,
                        LocalDepIssueKind::WrongLoader,
                        id,
                        d.req.display(),
                    ));
                }
                continue;
            }
            let hit = installed.get(id).is_some_and(|vs| {
                vs.iter().any(|v| match v {
                    None => d.req == Req::Any,
                    Some(v) => d.req.matches(v, &ctx.mc) == Some(true),
                })
            });
            if hit {
                push(issue(
                    file,
                    LocalDepIssueKind::Conflict,
                    id,
                    d.req.display(),
                ));
            }
        }
    }
    out
}

/// Normalised name for matching a mod id against Modrinth slugs/titles.
pub(crate) fn norm_name(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

// ---------------------------------------------------------------- service

/// (mod id, requirement as written).
type ReqKey = (String, Option<String>);
/// Version to install + project title.
type Suggestion = (ContentVersionRef, String);

impl Inner {
    pub async fn check_local(
        &self,
        id: InstanceId,
        file_names: Vec<String>,
    ) -> AppResult<Vec<LocalDepIssue>> {
        let inst = self.instances.get(id).await?;
        let game = self.instances.game_dir(id);
        let idx = ContentIndex::load(&self.instance_dir(id)).await?;
        let mods: Vec<DiskFile> = self
            .scan(&game)
            .await?
            .into_iter()
            .filter(|f| {
                f.kind == ContentKind::Mod && f.base().to_ascii_lowercase().ends_with(".jar")
            })
            .collect();

        // Files to check: the given ones, or every enabled mod not from a catalogue.
        let mut targets: Vec<DiskFile> = Vec::new();
        if file_names.is_empty() {
            targets.extend(
                mods.iter()
                    .filter(|f| {
                        f.enabled
                            && idx
                                .files
                                .get(&f.key())
                                .is_none_or(|e| e.source == ContentSourceKind::Local)
                    })
                    .cloned(),
            );
        } else {
            for name in &file_names {
                check_file_name(name)?;
                let want = base_name(name);
                let f = mods.iter().find(|f| f.base() == want).ok_or_else(|| {
                    AppError::new(ErrorCode::NotFound, "content file not found")
                        .detail("file", name)
                })?;
                if !targets.iter().any(|t| t.name == f.name) {
                    targets.push(f.clone());
                }
            }
        }
        if targets.is_empty() {
            return Ok(Vec::new());
        }

        let dir = game.join("mods");
        let to_read: Vec<(String, bool)> = mods
            .iter()
            .filter(|f| f.enabled || targets.iter().any(|t| t.name == f.name))
            .map(|f| (f.name.clone(), f.enabled))
            .collect();
        let infos = join(
            tokio::task::spawn_blocking(move || {
                Ok(to_read
                    .into_iter()
                    .map(|(name, enabled)| {
                        let info = read_jar_file(&dir.join(&name));
                        (name, enabled, info)
                    })
                    .collect::<Vec<_>>())
            })
            .await,
        )?;
        let installed = installed_ids(infos.iter().filter(|(_, e, _)| *e).map(|(_, _, i)| i));
        let ctx = Ctx {
            mc: inst.mc.clone(),
            loader: inst.loader.kind,
            loader_version: inst.loader.version.clone(),
        };
        let mut issues = Vec::new();
        for t in &targets {
            if let Some((_, _, info)) = infos.iter().find(|(n, _, _)| *n == t.name) {
                issues.extend(check_jar(t.base(), info, &ctx, &installed));
            }
        }

        // Suggestions: best effort, one lookup per (mod id, requirement).
        let target = target_of(&inst);
        let mut cache: HashMap<ReqKey, Option<Suggestion>> = HashMap::new();
        let mut reqs: HashMap<ReqKey, Req> = HashMap::new();
        for t in &targets {
            if let Some((_, _, info)) = infos.iter().find(|(n, _, _)| *n == t.name) {
                for m in &info.metas {
                    for d in &m.depends {
                        if let Some(first) = d.ids.first() {
                            reqs.insert((first.clone(), d.req.display()), d.req.clone());
                        }
                    }
                }
            }
        }
        let mut offline = false;
        for i in &mut issues {
            if !matches!(
                i.kind,
                LocalDepIssueKind::Missing | LocalDepIssueKind::WrongVersion
            ) || offline
            {
                continue;
            }
            let k = (i.mod_id.clone(), i.version_req.clone());
            if !cache.contains_key(&k) {
                let req = reqs.get(&k).cloned().unwrap_or(Req::Any);
                let found = match self.suggest(&i.mod_id, &req, &target).await {
                    Ok(f) => f,
                    Err(e) if e.code == ErrorCode::Network => {
                        offline = true;
                        None
                    }
                    Err(_) => None,
                };
                cache.insert(k.clone(), found);
            }
            if let Some(Some((r, title))) = cache.get(&k) {
                i.suggestion = Some(r.clone());
                i.suggestion_title = Some(title.clone());
            }
        }
        Ok(issues)
    }

    /// Modrinth version for mod id `id` that satisfies `req` on `target`.
    async fn suggest(&self, id: &str, req: &Req, target: &Target) -> AppResult<Option<Suggestion>> {
        if target.loader == LoaderKind::Vanilla {
            return Ok(None);
        }
        let cat = self.modrinth.clone();
        let mut cands = vec![id.to_owned()];
        let dashed = id.replace('_', "-");
        if dashed != id {
            cands.push(dashed);
        }
        let mut project = None;
        for c in &cands {
            match cat.project(c).await {
                Ok(p) if p.kind == ContentKind::Mod => {
                    project = Some(p);
                    break;
                }
                Ok(_) => {}
                Err(e) if e.code == ErrorCode::Network => return Err(e),
                Err(_) => {}
            }
        }
        if project.is_none() {
            let q = SearchQuery {
                source: ContentSourceKind::Modrinth,
                kind: ContentKind::Mod,
                text: id.to_owned(),
                mc: Some(target.mc.clone()),
                loader: Some(target.loader),
                categories: Vec::new(),
                sort: SortBy::Relevance,
                offset: 0,
                limit: 10,
            };
            let want = norm_name(id);
            let page = cat.search(&q).await?;
            if let Some(hit) = page
                .items
                .iter()
                .find(|h| norm_name(&h.slug) == want || norm_name(&h.title) == want)
            {
                project = Some(cat.project(&hit.project_id).await?);
            }
        }
        let Some(p) = project else {
            return Ok(None);
        };
        let list = cat
            .versions_ranked(&p.id, &target.filter(ContentKind::Mod))
            .await?;
        let ok = |v: &ContentVersion| {
            loader_compatible(&v.loaders, target.loader)
                && req.matches(&v.version_number, &target.mc) != Some(false)
        };
        let pick = list
            .iter()
            .find(|(v, release)| *release && ok(v))
            .or_else(|| list.iter().find(|(v, _)| ok(v)));
        Ok(pick.map(|(v, _)| {
            (
                ContentVersionRef {
                    source: ContentSourceKind::Modrinth,
                    project_id: p.id.clone(),
                    version_id: Some(v.version_id.clone()),
                },
                p.title.clone(),
            )
        }))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    /// In-memory zip with the given entries.
    pub fn jar(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn info(entries: &[(&str, Vec<u8>)]) -> JarInfo {
        read_jar(Cursor::new(jar(entries)), 0)
    }

    fn ctx(mc: &str, loader: LoaderKind, v: &str) -> Ctx {
        Ctx {
            mc: mc.into(),
            loader,
            loader_version: Some(v.into()),
        }
    }

    fn kinds(issues: &[LocalDepIssue]) -> Vec<(LocalDepIssueKind, &str)> {
        issues.iter().map(|i| (i.kind, i.mod_id.as_str())).collect()
    }

    fn fabric(json: serde_json::Value) -> (&'static str, Vec<u8>) {
        ("fabric.mod.json", json.to_string().into_bytes())
    }

    #[test]
    fn maven_ranges() {
        assert_eq!(maven_matches("[1.0,2.0)", "1.5", "1.20.1"), Some(true));
        assert_eq!(maven_matches("[1.0,2.0)", "2.0", "1.20.1"), Some(false));
        assert_eq!(maven_matches("[1.0,)", "0.9", "1.20.1"), Some(false));
        assert_eq!(maven_matches("(,1.0]", "1.0", "1.20.1"), Some(true));
        assert_eq!(maven_matches("[1.2]", "1.2.0", "1.20.1"), Some(true));
        assert_eq!(maven_matches("[1,2),[3,)", "3.1", "1.20.1"), Some(true));
        assert_eq!(maven_matches("1.0", "0.1", "1.20.1"), Some(true));
        assert_eq!(maven_matches("*", "x", "1.20.1"), Some(true));
        assert_eq!(
            maven_matches("[47,)", "1.20.1-47.2.0", "1.20.1"),
            Some(true)
        );
        assert_eq!(maven_matches("[4,)", "1.19.2-4.2", "1.20.1"), None);
        assert_eq!(maven_matches("[1.0,)", "abc", "1.20.1"), None);
    }

    #[test]
    fn fabric_missing_wrong_version_conflict_and_mc() {
        let a = info(&[fabric(serde_json::json!({
            "id": "a", "version": "1.0.0",
            "depends": {"fabricloader": ">=0.15", "minecraft": "~1.20.1",
                        "fabric-api": "*", "lib": ">=2.0", "java": ">=17"},
            "recommends": {"rec": "*"}, "suggests": {"sug": "*"},
            "breaks": {"bad": "*", "old": "<1.0"}
        }))]);
        let lib = info(&[fabric(serde_json::json!({"id": "lib", "version": "1.5.0"}))]);
        let bad = info(&[fabric(serde_json::json!({"id": "bad", "version": "3"}))]);
        let old = info(&[fabric(serde_json::json!({"id": "old", "version": "1.2"}))]);
        let installed = installed_ids([&a, &lib, &bad, &old]);
        let c = ctx("1.21.1", LoaderKind::Fabric, "0.14.0");
        let issues = check_jar("a.jar", &a, &c, &installed);
        let k = kinds(&issues);
        assert!(k.contains(&(LocalDepIssueKind::WrongLoader, "fabricloader")));
        assert!(k.contains(&(LocalDepIssueKind::WrongLoader, "minecraft")));
        assert!(k.contains(&(LocalDepIssueKind::Missing, "fabric-api")));
        assert!(k.contains(&(LocalDepIssueKind::WrongVersion, "lib")));
        assert!(k.contains(&(LocalDepIssueKind::Conflict, "bad")));
        assert_eq!(k.len(), 5, "{k:?}");
        let lib_issue = issues.iter().find(|i| i.mod_id == "lib").unwrap();
        assert_eq!(lib_issue.version_req.as_deref(), Some(">=2.0"));
        assert_eq!(lib_issue.file_name, "a.jar");

        // All fine on the right instance with fabric-api provided by a jar-in-jar.
        let api_inner = jar(&[fabric(
            serde_json::json!({"id": "fabric-api-base", "version": "0.4.0"}),
        )]);
        let api = info(&[
            fabric(
                serde_json::json!({"id": "fabric-api", "version": "0.92.0+1.20.1", "provides": ["fabric"]}),
            ),
            ("META-INF/jars/base.jar", api_inner),
        ]);
        assert!(api.nested_ids.iter().any(|(id, _)| id == "fabric-api-base"));
        let lib2 = info(&[fabric(serde_json::json!({"id": "lib", "version": "2.1.0"}))]);
        let installed = installed_ids([&a, &lib2, &api]);
        let c = ctx("1.20.1", LoaderKind::Fabric, "0.16.0");
        assert!(check_jar("a.jar", &a, &c, &installed).is_empty());
    }

    #[test]
    fn wrong_loader_and_quilt_reads_fabric() {
        let f = info(&[fabric(serde_json::json!({"id": "f", "version": "1"}))]);
        let installed = installed_ids([&f]);
        let k = check_jar(
            "f.jar",
            &f,
            &ctx("1.20.1", LoaderKind::Forge, "47.2.0"),
            &installed,
        );
        assert_eq!(kinds(&k), vec![(LocalDepIssueKind::WrongLoader, "fabric")]);
        assert!(check_jar(
            "f.jar",
            &f,
            &ctx("1.20.1", LoaderKind::Quilt, "0.26"),
            &installed
        )
        .is_empty());
        // Not a mod at all → nothing.
        let none = info(&[("readme.txt", b"x".to_vec())]);
        assert!(check_jar(
            "n.jar",
            &none,
            &ctx("1.20.1", LoaderKind::Fabric, "1"),
            &installed
        )
        .is_empty());
    }

    #[test]
    fn quilt_metadata() {
        let q = info(&[(
            "quilt.mod.json",
            serde_json::json!({"schema_version": 1, "quilt_loader": {
                "group": "org.x", "id": "q", "version": "1.0.0",
                "provides": ["q_alias", {"id": "q_api", "version": "2.0.0"}],
                "depends": [
                    {"id": "quilt_loader", "versions": ">=0.20"},
                    "org.y:needed",
                    {"id": "opt", "optional": true},
                    {"id": "cond", "unless": "other"},
                    [{"id": "alt_a"}, {"id": "alt_b"}],
                    {"id": "lib", "versions": [">=2.0"]}
                ],
                "breaks": [{"id": "enemy"}]
            }})
            .to_string()
            .into_bytes(),
        )]);
        let meta = &q.metas[0];
        assert_eq!(meta.format, Format::Quilt);
        assert!(meta.ids.contains(&("q_api".into(), Some("2.0.0".into()))));
        let alt_b = info(&[fabric(serde_json::json!({"id": "alt_b", "version": "1"}))]);
        let lib = info(&[fabric(serde_json::json!({"id": "lib", "version": "1.0"}))]);
        let enemy = info(&[fabric(serde_json::json!({"id": "enemy", "version": "1"}))]);
        let installed = installed_ids([&q, &alt_b, &lib, &enemy]);
        let k = check_jar(
            "q.jar",
            &q,
            &ctx("1.20.1", LoaderKind::Quilt, "0.19.0"),
            &installed,
        );
        let mut got = kinds(&k);
        got.sort_by_key(|(_, id)| id.to_string());
        assert_eq!(
            got,
            vec![
                (LocalDepIssueKind::Conflict, "enemy"),
                (LocalDepIssueKind::WrongVersion, "lib"),
                (LocalDepIssueKind::Missing, "needed"),
                (LocalDepIssueKind::WrongLoader, "quilt_loader"),
            ]
        );
        // Quilt-only jar on Fabric.
        let k = check_jar(
            "q.jar",
            &q,
            &ctx("1.20.1", LoaderKind::Fabric, "0.16"),
            &installed,
        );
        assert_eq!(kinds(&k), vec![(LocalDepIssueKind::WrongLoader, "quilt")]);
    }

    const FORGE_TOML: &str = r#"
modLoader = "javafml"
loaderVersion = "[47,)"
[[mods]]
modId = "fmod"
version = "${file.jarVersion}"
[[dependencies.fmod]]
modId = "forge"
mandatory = true
versionRange = "[47.1,)"
side = "BOTH"
[[dependencies.fmod]]
modId = "minecraft"
mandatory = true
versionRange = "[1.20.1,1.21)"
side = "BOTH"
[[dependencies.fmod]]
modId = "geckolib"
mandatory = true
versionRange = "[4.2,)"
side = "BOTH"
[[dependencies.fmod]]
modId = "jei"
mandatory = false
versionRange = "*"
side = "CLIENT"
[[dependencies.fmod]]
modId = "serveronly"
mandatory = true
side = "SERVER"
"#;

    #[test]
    fn forge_mods_toml() {
        let fm = info(&[
            (
                "META-INF/MANIFEST.MF",
                b"Manifest-Version: 1.0\r\nImplementation-Version: 2.3.1\r\n".to_vec(),
            ),
            ("META-INF/mods.toml", FORGE_TOML.as_bytes().to_vec()),
        ]);
        assert_eq!(
            fm.metas[0].ids,
            vec![("fmod".to_owned(), Some("2.3.1".to_owned()))]
        );
        let installed = installed_ids([&fm]);
        let k = check_jar(
            "f.jar",
            &fm,
            &ctx("1.20.1", LoaderKind::Forge, "47.2.0"),
            &installed,
        );
        assert_eq!(kinds(&k), vec![(LocalDepIssueKind::Missing, "geckolib")]);
        assert_eq!(k[0].version_req.as_deref(), Some("[4.2,)"));

        let k = check_jar(
            "f.jar",
            &fm,
            &ctx("1.21.1", LoaderKind::Forge, "1.21.1-46.0.1"),
            &installed,
        );
        let k = kinds(&k);
        assert!(k.contains(&(LocalDepIssueKind::WrongLoader, "minecraft")));
        assert!(k.contains(&(LocalDepIssueKind::WrongLoader, "forge")));

        // Old geckolib → wrong version; Forge mod on NeoForge 1.21 → wrong loader.
        let gl = info(&[(
            "META-INF/mods.toml",
            b"[[mods]]\nmodId=\"geckolib\"\nversion=\"4.1.0\"\n".to_vec(),
        )]);
        let installed = installed_ids([&fm, &gl]);
        let k = check_jar(
            "f.jar",
            &fm,
            &ctx("1.20.1", LoaderKind::Forge, "47.2.0"),
            &installed,
        );
        assert_eq!(
            kinds(&k),
            vec![(LocalDepIssueKind::WrongVersion, "geckolib")]
        );
        let k = check_jar(
            "f.jar",
            &fm,
            &ctx("1.21.1", LoaderKind::NeoForge, "21.1.1"),
            &installed,
        );
        assert_eq!(kinds(&k), vec![(LocalDepIssueKind::WrongLoader, "forge")]);
    }

    #[test]
    fn neoforge_mods_toml() {
        let toml = r#"
[[mods]]
modId = "nmod"
version = "1.0.0"
[[dependencies.nmod]]
modId = "neoforge"
type = "required"
versionRange = "[21.1.50,)"
[[dependencies.nmod]]
modId = "sodium"
type = "incompatible"
versionRange = "*"
[[dependencies.nmod]]
modId = "emi"
type = "optional"
[[dependencies.nmod]]
modId = "architectury"
type = "required"
versionRange = "[13,)"
"#;
        let nm = info(&[("META-INF/neoforge.mods.toml", toml.as_bytes().to_vec())]);
        assert_eq!(nm.metas[0].format, Format::NeoToml);
        let sodium = info(&[(
            "META-INF/neoforge.mods.toml",
            b"[[mods]]\nmodId=\"sodium\"\nversion=\"0.6.0\"\n".to_vec(),
        )]);
        let arch = info(&[(
            "META-INF/neoforge.mods.toml",
            b"[[mods]]\nmodId=\"architectury\"\nversion=\"13.0.8\"\n".to_vec(),
        )]);
        let installed = installed_ids([&nm, &sodium, &arch]);
        let k = check_jar(
            "n.jar",
            &nm,
            &ctx("1.21.1", LoaderKind::NeoForge, "21.1.10"),
            &installed,
        );
        let mut got = kinds(&k);
        got.sort_by_key(|(_, id)| id.to_string());
        assert_eq!(
            got,
            vec![
                (LocalDepIssueKind::WrongLoader, "neoforge"),
                (LocalDepIssueKind::Conflict, "sodium"),
            ]
        );
        let k = check_jar(
            "n.jar",
            &nm,
            &ctx("1.21.1", LoaderKind::Forge, "52.0.1"),
            &installed,
        );
        assert_eq!(
            kinds(&k),
            vec![(LocalDepIssueKind::WrongLoader, "neoforge")]
        );
    }

    #[test]
    fn unparseable_versions_are_not_reported() {
        let a = info(&[fabric(serde_json::json!({
            "id": "a", "version": "1", "depends": {"minecraft": "1.20.x", "lib": ">=2.0"}
        }))]);
        let lib = info(&[fabric(
            serde_json::json!({"id": "lib", "version": "${version}"}),
        )]);
        let installed = installed_ids([&a, &lib]);
        // Snapshot MC and placeholder version: unknown → silent.
        assert!(check_jar(
            "a.jar",
            &a,
            &ctx("24w14a", LoaderKind::Fabric, "0.16"),
            &installed
        )
        .is_empty());
    }
}
