//! Fabric mod metadata (`fabric.mod.json`): id, version, `provides`,
//! `depends` / `breaks` and a matcher for Fabric version predicates.
//!
//! Used by the resolver to avoid installing sets that the loader rejects with
//! "Incompatible mods found" (e.g. Iris built for Sodium 0.6 next to Sodium 0.8).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;

/// Max size of `fabric.mod.json` we are willing to read.
const MAX_META: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModMeta {
    pub id: String,
    pub version: String,
    pub provides: Vec<String>,
    /// mod id → alternatives (OR) of predicate strings.
    pub depends: BTreeMap<String, Vec<String>>,
    pub breaks: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Preds {
    One(String),
    Many(Vec<String>),
}

impl Preds {
    fn into_vec(self) -> Vec<String> {
        match self {
            Preds::One(s) => vec![s],
            Preds::Many(v) => v,
        }
    }
}

#[derive(Deserialize)]
struct RawMeta {
    id: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    provides: Vec<String>,
    #[serde(default)]
    depends: BTreeMap<String, Preds>,
    #[serde(default)]
    breaks: BTreeMap<String, Preds>,
}

/// Parses `fabric.mod.json` contents.
pub(crate) fn parse_fabric_mod_json(bytes: &[u8]) -> Option<ModMeta> {
    // Some mods ship a BOM.
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let raw: RawMeta = serde_json::from_slice(bytes).ok()?;
    let conv = |m: BTreeMap<String, Preds>| {
        m.into_iter()
            .map(|(k, v)| (k, v.into_vec()))
            .collect::<BTreeMap<_, _>>()
    };
    Some(ModMeta {
        id: raw.id,
        version: raw.version,
        provides: raw.provides,
        depends: conv(raw.depends),
        breaks: conv(raw.breaks),
    })
}

/// Reads `fabric.mod.json` from a mod jar (blocking).
pub(crate) fn read_jar_meta(path: &Path) -> Option<ModMeta> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let entry = zip.by_name("fabric.mod.json").ok()?;
    if entry.size() > MAX_META {
        return None;
    }
    let mut buf = Vec::new();
    entry.take(MAX_META).read_to_end(&mut buf).ok()?;
    parse_fabric_mod_json(&buf)
}

/// Async wrapper around [`read_jar_meta`].
pub(crate) async fn read_jar_meta_async(path: std::path::PathBuf) -> Option<ModMeta> {
    tokio::task::spawn_blocking(move || read_jar_meta(&path))
        .await
        .ok()
        .flatten()
}

/// Semantic version as Fabric understands it: numeric components,
/// optional pre-release (`-...`), build metadata (`+...`) ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ver {
    comps: Vec<u64>,
    /// `None` = release. `Some([])` = lowest possible pre-release (bounds).
    pre: Option<Vec<String>>,
}

fn cmp_pre_ident(a: &str, b: &str) -> Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        _ => a.cmp(b),
    }
}

impl Ord for Ver {
    fn cmp(&self, o: &Self) -> Ordering {
        let n = self.comps.len().max(o.comps.len());
        for i in 0..n {
            let a = self.comps.get(i).copied().unwrap_or(0);
            let b = o.comps.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                Ordering::Equal => {}
                other => return other,
            }
        }
        match (&self.pre, &o.pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    match cmp_pre_ident(x, y) {
                        Ordering::Equal => {}
                        other => return other,
                    }
                }
                a.len().cmp(&b.len())
            }
        }
    }
}

impl PartialOrd for Ver {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Compares two semantic-ish versions; `None` if either cannot be parsed.
pub(crate) fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    Some(parse_ver(a)?.cmp(&parse_ver(b)?))
}

/// Parsed predicate version: numeric prefix + optional wildcard position.
struct PVer {
    ver: Ver,
    /// Index of the first `x`/`X`/`*` component, if any.
    wildcard: Option<usize>,
}

fn parse_ver(s: &str) -> Option<Ver> {
    parse_pver(s)
        .filter(|p| p.wildcard.is_none())
        .map(|p| p.ver)
}

fn parse_pver(s: &str) -> Option<PVer> {
    let s = s.split('+').next().unwrap_or("");
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, Some(p.split('.').map(str::to_owned).collect())),
        None => (s, None),
    };
    if core.is_empty() {
        return None;
    }
    let mut comps = Vec::new();
    let mut wildcard = None;
    for (i, part) in core.split('.').enumerate() {
        if matches!(part, "x" | "X" | "*") {
            wildcard = Some(i);
            break;
        }
        comps.push(part.parse::<u64>().ok()?);
    }
    if comps.is_empty() && wildcard.is_none() {
        return None;
    }
    Some(PVer {
        ver: Ver { comps, pre },
        wildcard,
    })
}

/// Lowest version with `comps` as prefix.
fn floor(comps: &[u64]) -> Ver {
    Ver {
        comps: comps.to_vec(),
        pre: Some(Vec::new()),
    }
}

/// Lowest version above every version with `comps[..len]` as prefix.
fn ceil(comps: &[u64], len: usize) -> Option<Ver> {
    if len == 0 {
        return None;
    }
    let mut c: Vec<u64> = comps.iter().copied().take(len).collect();
    c.resize(len, 0);
    let last = c.last_mut()?;
    *last = last.saturating_add(1);
    Some(floor(&c))
}

/// One term (`>=1.2`, `~0.6.0`, `0.6.x`, `*`). `None` = cannot evaluate.
fn term_matches(term: &str, v: &Ver) -> Option<bool> {
    let term = term.trim();
    if term.is_empty() || term == "*" {
        return Some(true);
    }
    let (op, rest) = ["<=", ">=", "<", ">", "=", "~", "^"]
        .iter()
        .find_map(|op| term.strip_prefix(op).map(|r| (*op, r.trim())))
        .unwrap_or(("", term));
    let p = parse_pver(rest)?;
    let base = &p.ver;
    match p.wildcard {
        Some(w) => {
            let lo = floor(&base.comps);
            let hi = ceil(&base.comps, w);
            let below_hi = hi.as_ref().is_none_or(|h| v < h);
            Some(match op {
                "" | "=" | "~" | "^" => *v >= lo && below_hi,
                ">=" => *v >= lo,
                ">" => hi.as_ref().is_some_and(|h| v >= h),
                "<" => *v < lo,
                "<=" => below_hi,
                _ => return None,
            })
        }
        None => Some(match op {
            "" | "=" => v == base || v.cmp(base) == Ordering::Equal,
            ">=" => v >= base,
            ">" => v > base,
            "<" => v < base,
            "<=" => v <= base,
            // ~1.2.3 → >=1.2.3 <1.3.0-; ~1 → >=1 <2
            "~" => {
                v >= base && ceil(&base.comps, base.comps.len().clamp(1, 2)).is_none_or(|h| *v < h)
            }
            // ^1.2.3 → >=1.2.3 <2.0.0-
            "^" => v >= base && ceil(&base.comps, 1).is_none_or(|h| *v < h),
            _ => return None,
        }),
    }
}

/// One predicate string: space-separated terms (AND), `||` alternatives (OR).
fn pred_matches(pred: &str, v: &Ver) -> Option<bool> {
    let mut any_known = false;
    for alt in pred.split("||") {
        let mut all = true;
        for term in alt.split_whitespace() {
            match term_matches(term, v)? {
                true => {}
                false => all = false,
            }
        }
        any_known = true;
        if all {
            return Some(true);
        }
    }
    any_known.then_some(false)
}

/// Whether `version` satisfies any of `preds` (Fabric semantics: list = OR).
/// `None` when the version or a predicate cannot be parsed (unknown, not a
/// violation).
pub(crate) fn satisfies(preds: &[String], version: &str) -> Option<bool> {
    if preds.is_empty() {
        return Some(true);
    }
    let Some(v) = parse_ver(version) else {
        // Non-semantic versions only match literally or `*`.
        return Some(preds.iter().any(|p| p.trim() == "*" || p.trim() == version));
    };
    let mut known = false;
    for p in preds {
        match pred_matches(p, &v) {
            Some(true) => return Some(true),
            Some(false) => known = true,
            None => {}
        }
    }
    known.then_some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn predicates() {
        assert_eq!(satisfies(&s(&["0.6.x"]), "0.6.13+mc1.21.1"), Some(true));
        assert_eq!(satisfies(&s(&["0.6.x"]), "0.8.13+mc1.21.1"), Some(false));
        assert_eq!(satisfies(&s(&[">=0.6.0 <0.7.0"]), "0.6.0"), Some(true));
        assert_eq!(satisfies(&s(&[">=0.6.0"]), "0.6.0-beta.2"), Some(false));
        assert_eq!(satisfies(&s(&["<0.6.0", ">0.6.x"]), "0.8.13"), Some(true));
        assert_eq!(satisfies(&s(&["<0.6.0 || >0.6.x"]), "0.6.5"), Some(false));
        assert_eq!(satisfies(&s(&["~1.21"]), "1.21.1"), Some(true));
        assert_eq!(satisfies(&s(&["~1.21.1"]), "1.22"), Some(false));
        assert_eq!(satisfies(&s(&["^0.15.0"]), "0.16.9"), Some(true));
        assert_eq!(satisfies(&s(&["^1.0.0"]), "2.0.0"), Some(false));
        assert_eq!(satisfies(&s(&["*"]), "anything"), Some(true));
        assert_eq!(satisfies(&s(&["1.21.1"]), "1.21.1"), Some(true));
        assert_eq!(satisfies(&s(&["1.21.1"]), "1.21"), Some(false));
        assert_eq!(satisfies(&s(&[">=weird"]), "1.0"), None);
    }

    #[test]
    fn parses_fixture() {
        let m = parse_fabric_mod_json(include_bytes!(
            "../tests/fixtures/fabric/iris-1.8.8.fabric.mod.json"
        ))
        .unwrap();
        assert_eq!(m.id, "iris");
        assert!(m.breaks.contains_key("sodium"));
        assert!(m.depends.contains_key("sodium"));
    }
}
