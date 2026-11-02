//! Stand-ins for pack files the launcher may not download itself (CurseForge
//! `allowModDistribution=false`, an unavailable file or a missing hash).
//!
//! A whole modpack must not fail because one of its files is opt-out: the same
//! mod version is taken from Modrinth when it can be identified with confidence,
//! otherwise the file is only listed with a direct link for the user.

use std::collections::HashSet;

use lc_runtime::ProgressSink;
use lc_types::content::{ContentKind, ContentSourceKind, ContentVersion, SearchQuery, SortBy};
use lc_types::version::LoaderKind;
use serde::{Deserialize, Serialize};

use crate::contract::ContentSource;
use crate::service::Inner;
use crate::source::{Catalog, ProjectInfo, Target};
use crate::util::loader_compatible;

/// Modrinth search hits inspected per stand-in lookup.
const MAX_HITS: u32 = 5;

/// A pack file that cannot be downloaded by the launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Missing {
    /// CurseForge project slug and title: the name of the same mod on Modrinth.
    pub slug: String,
    pub title: String,
    pub kind: ContentKind,
    pub version_number: String,
    pub file_name: String,
}

impl Missing {
    pub(crate) fn of(v: &ContentVersion, p: &ProjectInfo) -> Self {
        Self {
            slug: p.slug.clone(),
            title: p.title.clone(),
            kind: p.kind,
            version_number: v.version_number.clone(),
            file_name: v.file_name.clone(),
        }
    }
}

/// A Modrinth stand-in with everything needed to download it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StandIn {
    pub version: ContentVersion,
    pub title: String,
    pub url: String,
    pub sha1: String,
    pub sha512: Option<String>,
}

/// Pack files the user has to download by hand (`manual-downloads.json` of the
/// instance; also logged into the task).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManualFile {
    pub file_name: String,
    pub title: String,
    /// Direct link to this exact file (game version, loader and file id).
    pub url: String,
}

/// Lowercase letters and digits only (`Just Enough Items` → `justenoughitems`).
pub(crate) fn norm(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Maximal runs of digits (`sodium-mc1.20.1-0.5.8` → `1 20 1 0 5 8`).
fn numbers(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        match (c.is_ascii_digit(), start) {
            (true, None) => start = Some(i),
            (false, Some(from)) => {
                out.push(&s[from..i]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        out.push(&s[from..]);
    }
    out
}

/// `tail` is the last `n` numeric components of `hay` (`n ≥ 2`).
fn ends_with_run(hay: &[&str], tail: &[&str]) -> bool {
    tail.len() >= 2
        && tail.len() <= hay.len()
        && hay[hay.len() - tail.len()..].iter().eq(tail.iter())
}

/// Both strings carry the same version, compared by their last numeric
/// components (`mc1.20.1-0.5.8` ≡ `sodium-fabric-mc1.20.1-0.5.8`, but not `0.5.7`).
fn same_version(a: &str, b: &str) -> bool {
    let (a, b) = (numbers(a), numbers(b));
    ends_with_run(&a, &b) || ends_with_run(&b, &a)
}

/// The Modrinth project is the CurseForge one (`slug`/`title`, case and
/// punctuation ignored).
pub(crate) fn same_project(missing: &Missing, slug: &str, title: &str) -> bool {
    let want: Vec<String> = [missing.slug.as_str(), missing.title.as_str()]
        .into_iter()
        .map(norm)
        .filter(|n| n.len() >= 3)
        .collect();
    [slug, title]
        .into_iter()
        .map(norm)
        .any(|n| !n.is_empty() && want.contains(&n))
}

/// The candidate is the same file: built for this game version and loader and
/// carrying the same mod version (identical file name, or the same version
/// numbers at the end of the name / version string).
pub(crate) fn version_matches(missing: &Missing, v: &ContentVersion, target: &Target) -> bool {
    if !fits(missing.kind, v, target) {
        return false;
    }
    if norm(&missing.file_name) == norm(&v.file_name) {
        return true;
    }
    let want = missing.version_number.as_str();
    [v.version_number.as_str(), v.file_name.as_str()]
        .into_iter()
        .any(|s| same_version(want, s))
}

fn fits(kind: ContentKind, v: &ContentVersion, target: &Target) -> bool {
    let mc_ok = v.mc_versions.is_empty() || v.mc_versions.contains(&target.mc);
    let loader_ok = !matches!(kind, ContentKind::Mod | ContentKind::Modpack)
        || v.loaders.is_empty()
        || target.loader == LoaderKind::Vanilla
        || loader_compatible(&v.loaders, target.loader);
    mc_ok && loader_ok
}

/// Folder of a project class in `curseforge.com` URLs.
fn category(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Mod => "mc-mods",
        ContentKind::Modpack => "modpacks",
        ContentKind::ResourcePack => "texture-packs",
        ContentKind::Shader => "shaders",
        ContentKind::DataPack => "data-packs",
    }
}

fn url_safe_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 128
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn file_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// Direct link to one file of this project and game version
/// (`…/minecraft/mc-mods/<slug>/files/<file>`); `page` (the numeric project or
/// file URL) when the slug cannot be used in a URL.
pub(crate) fn file_url(kind: ContentKind, slug: &str, file_id: &str, page: &str) -> String {
    if url_safe_slug(slug) && file_id_ok(file_id) {
        format!(
            "https://www.curseforge.com/minecraft/{}/{slug}/files/{file_id}",
            category(kind)
        )
    } else {
        page.to_owned()
    }
}

impl Inner {
    /// The same mod version from Modrinth for a file that may not be
    /// redistributed. Best effort: `None` (with a log line) when nothing matches
    /// confidently or the network fails.
    pub(crate) async fn modrinth_stand_in(
        &self,
        missing: &Missing,
        target: &Target,
        sink: &dyn ProgressSink,
    ) -> Option<StandIn> {
        let mut seen: HashSet<String> = HashSet::new();
        for text in [missing.slug.as_str(), missing.title.as_str()] {
            let query = SearchQuery {
                source: ContentSourceKind::Modrinth,
                kind: missing.kind,
                text: text.to_owned(),
                mc: Some(target.mc.clone()),
                loader: Some(target.loader),
                categories: Vec::new(),
                sort: SortBy::Downloads,
                offset: 0,
                limit: MAX_HITS,
            };
            let Ok(page) = self.modrinth.search(&query).await else {
                continue;
            };
            for hit in page.items {
                if !seen.insert(hit.project_id.clone())
                    || !same_project(missing, &hit.slug, &hit.title)
                {
                    continue;
                }
                let Ok(list) = self
                    .modrinth
                    .versions_ranked(&hit.project_id, &target.filter(missing.kind))
                    .await
                else {
                    continue;
                };
                let Some(version) = list
                    .into_iter()
                    .map(|(v, _)| v)
                    .find(|v| version_matches(missing, v, target))
                else {
                    continue;
                };
                let Ok(remote) = self.modrinth.file(&version).await else {
                    continue;
                };
                let (Some(url), Some(sha1)) = (remote.url, remote.sha1) else {
                    continue;
                };
                sink.log(&format!(
                    "CurseForge file {} may not be redistributed; installing {} from Modrinth",
                    missing.file_name, version.file_name
                ));
                return Some(StandIn {
                    title: hit.title,
                    version,
                    url,
                    sha1,
                    sha512: remote.sha512,
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use lc_types::content::ContentSourceKind;

    use super::*;

    fn missing() -> Missing {
        Missing {
            slug: "sodium".into(),
            title: "Sodium".into(),
            kind: ContentKind::Mod,
            version_number: "sodium-fabric-mc1.20.1-0.5.8".into(),
            file_name: "sodium-fabric-0.5.8.jar".into(),
        }
    }

    fn version(version_number: &str, file_name: &str, loaders: &[LoaderKind]) -> ContentVersion {
        ContentVersion {
            source: ContentSourceKind::Modrinth,
            project_id: "AANobbMI".into(),
            version_id: "v1".into(),
            name: version_number.into(),
            version_number: version_number.into(),
            mc_versions: vec!["1.20.1".into()],
            loaders: loaders.to_vec(),
            published: String::new(),
            file_name: file_name.into(),
            size: 1,
            sha1: None,
            dependencies: Vec::new(),
            manual_download: false,
        }
    }

    fn target(loader: LoaderKind) -> Target {
        Target {
            mc: "1.20.1".into(),
            loader,
        }
    }

    #[test]
    fn numbers_are_compared_by_their_tail() {
        assert!(same_version(
            "mc1.20.1-0.5.8",
            "sodium-fabric-mc1.20.1-0.5.8"
        ));
        assert!(same_version("sodium-fabric-mc1.20.1-0.5.8", "0.5.8"));
        assert!(!same_version("0.5.7", "sodium-fabric-mc1.20.1-0.5.8"));
        assert!(!same_version("0.5", "sodium-fabric-mc1.20.1-0.5.8"));
        // A single component is not a version match.
        assert!(!same_version("8", "sodium-0.5.8"));
        assert_eq!(numbers("v2.1a3"), vec!["2", "1", "3"]);
        assert!(numbers("nonumbers").is_empty());
    }

    #[test]
    fn project_name_match_ignores_case_and_punctuation() {
        assert!(same_project(&missing(), "sodium", "Sodium"));
        assert!(same_project(&missing(), "Sodium", ""));
        assert!(!same_project(&missing(), "sodium-extra", "Sodium Extra"));
        // Too short to be a name.
        let mut m = missing();
        m.slug = "a".into();
        m.title = "b".into();
        assert!(!same_project(&m, "a", "b"));
    }

    #[test]
    fn version_match_needs_name_and_version() {
        let m = missing();
        let t = target(LoaderKind::Forge);
        // Same jar published under another name.
        assert!(version_matches(
            &m,
            &version(
                "mc1.20.1-0.5.8",
                "sodium-fabric-0.5.8.jar",
                &[LoaderKind::Forge]
            ),
            &t
        ));
        // Same version numbers in another file name.
        assert!(version_matches(
            &m,
            &version("0.5.8", "sodium-forge.jar", &[LoaderKind::Forge]),
            &t
        ));
        // Another build of the same mod.
        assert!(!version_matches(
            &m,
            &version("0.5.7", "sodium-forge-0.5.7.jar", &[LoaderKind::Forge]),
            &t
        ));
        // Another Minecraft version or loader.
        let mut other = version("0.5.8", "sodium.jar", &[LoaderKind::Forge]);
        other.mc_versions = vec!["1.21.1".into()];
        assert!(!version_matches(&m, &other, &t));
        assert!(!version_matches(
            &m,
            &version("0.5.8", "sodium.jar", &[LoaderKind::Fabric]),
            &t
        ));
        // Resource packs carry no mod loader.
        let pack = Missing {
            kind: ContentKind::ResourcePack,
            ..missing()
        };
        assert!(version_matches(
            &pack,
            &version("0.5.8", "sodium.jar", &[]),
            &t
        ));
    }

    #[test]
    fn file_links_point_at_the_exact_file() {
        assert_eq!(
            file_url(ContentKind::Mod, "sodium", "3946121", "https://x/y"),
            "https://www.curseforge.com/minecraft/mc-mods/sodium/files/3946121"
        );
        assert_eq!(
            file_url(ContentKind::Shader, "complementary", "12", "page"),
            "https://www.curseforge.com/minecraft/shaders/complementary/files/12"
        );
        // Unusable slug or id: the caller's page is kept as is.
        for (slug, id) in [
            ("Sodium", "12"),
            ("a/b", "12"),
            ("sodium", "12a"),
            ("", "12"),
            ("sodium", ""),
        ] {
            assert_eq!(file_url(ContentKind::Mod, slug, id, "page"), "page");
        }
    }
}
