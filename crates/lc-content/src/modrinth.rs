//! Modrinth API v2 (no key; the downloader sets the required User-Agent).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use lc_types::content::{
    ContentFilter, ContentKind, ContentSourceKind, ContentVersion, Dependency, DependencyKind,
    ProjectSummary, SearchPage, SearchQuery, SortBy,
};
use lc_types::version::LoaderKind;
use lc_types::{AppError, AppResult, ErrorCode};
use serde::Deserialize;

use crate::api::{Api, TTL_IMMUTABLE, TTL_META, TTL_SEARCH, TTL_VERSIONS};
use crate::contract::ContentSource;
use crate::source::{Catalog, PackFile, ProjectInfo, RemoteFile, Target, UpdateItem};
use crate::util::{loader_from_str, parse_url};

pub(crate) const MODRINTH_API: &str = "https://api.modrinth.com/v2";

/// Modrinth catalogue.
pub struct ModrinthSource {
    api: Arc<Api>,
    base: String,
}

#[derive(Debug, Deserialize)]
struct SearchResp {
    hits: Vec<Hit>,
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    total_hits: u32,
}

#[derive(Debug, Deserialize)]
struct Hit {
    project_id: String,
    #[serde(default)]
    slug: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    project_type: String,
    #[serde(default)]
    categories: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Project {
    id: String,
    slug: String,
    title: String,
    project_type: String,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    icon_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MrVersion {
    id: String,
    project_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version_number: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    date_published: String,
    #[serde(default)]
    version_type: String,
    #[serde(default)]
    featured: bool,
    #[serde(default)]
    files: Vec<MrFile>,
    #[serde(default)]
    dependencies: Vec<MrDep>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MrFile {
    #[serde(default)]
    hashes: HashMap<String, String>,
    url: String,
    filename: String,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    size: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct MrDep {
    #[serde(default)]
    version_id: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    dependency_type: String,
}

fn kind_of(project_type: &str, loaders: &[String]) -> ContentKind {
    match project_type {
        "modpack" => ContentKind::Modpack,
        "resourcepack" => ContentKind::ResourcePack,
        "shader" => ContentKind::Shader,
        "datapack" => ContentKind::DataPack,
        _ if !loaders.is_empty() && loaders.iter().all(|l| l == "datapack") => {
            ContentKind::DataPack
        }
        _ => ContentKind::Mod,
    }
}

fn loader_names(l: LoaderKind) -> Vec<&'static str> {
    match l {
        LoaderKind::Fabric => vec!["fabric"],
        LoaderKind::Quilt => vec!["quilt", "fabric"],
        LoaderKind::Forge => vec!["forge"],
        LoaderKind::NeoForge => vec!["neoforge"],
        LoaderKind::Vanilla => vec![],
    }
}

fn kind_loaders(kind: ContentKind, loader: LoaderKind) -> Vec<&'static str> {
    match kind {
        ContentKind::Mod | ContentKind::Modpack => loader_names(loader),
        ContentKind::ResourcePack => vec!["minecraft"],
        ContentKind::Shader => vec!["iris", "optifine", "canvas", "vanilla"],
        ContentKind::DataPack => vec!["datapack"],
    }
}

fn primary(v: &MrVersion) -> Option<&MrFile> {
    v.files
        .iter()
        .find(|f| f.primary)
        .or_else(|| v.files.first())
}

pub(crate) fn to_content_version(v: &MrVersion) -> AppResult<ContentVersion> {
    let f = primary(v).ok_or_else(|| {
        AppError::new(ErrorCode::NotFound, "version has no files").detail("version", &v.id)
    })?;
    Ok(ContentVersion {
        source: ContentSourceKind::Modrinth,
        project_id: v.project_id.clone(),
        version_id: v.id.clone(),
        name: v.name.clone(),
        version_number: v.version_number.clone(),
        mc_versions: v.game_versions.clone(),
        loaders: v
            .loaders
            .iter()
            .filter_map(|l| loader_from_str(l))
            .collect(),
        published: v.date_published.clone(),
        file_name: f.filename.clone(),
        size: f.size,
        sha1: f.hashes.get("sha1").cloned(),
        dependencies: v
            .dependencies
            .iter()
            .filter_map(|d| {
                let kind = match d.dependency_type.as_str() {
                    "required" => DependencyKind::Required,
                    "optional" => DependencyKind::Optional,
                    "incompatible" => DependencyKind::Incompatible,
                    "embedded" => DependencyKind::Embedded,
                    _ => return None,
                };
                if d.project_id.is_none() && d.version_id.is_none() {
                    return None;
                }
                Some(Dependency {
                    project_id: d.project_id.clone().unwrap_or_default(),
                    version_id: d.version_id.clone(),
                    kind,
                })
            })
            .collect(),
        manual_download: false,
    })
}

fn sort_index(s: SortBy) -> &'static str {
    match s {
        SortBy::Relevance => "relevance",
        SortBy::Downloads => "downloads",
        SortBy::Updated => "updated",
        SortBy::Newest => "newest",
    }
}

fn json_list(items: &[&str]) -> String {
    serde_json::to_string(items).unwrap_or_else(|_| "[]".into())
}

impl ModrinthSource {
    pub(crate) fn new(api: Arc<Api>) -> Self {
        Self::with_base(api, MODRINTH_API)
    }

    pub(crate) fn with_base(api: Arc<Api>, base: &str) -> Self {
        Self {
            api,
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    fn id_segment(id: &str) -> AppResult<&str> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(AppError::new(ErrorCode::InvalidInput, "bad modrinth id").detail("id", id));
        }
        Ok(id)
    }

    pub(crate) fn search_url(&self, q: &SearchQuery) -> AppResult<String> {
        let mut facets: Vec<Vec<String>> = Vec::new();
        let ptype = match q.kind {
            ContentKind::Mod | ContentKind::DataPack => "mod",
            ContentKind::ResourcePack => "resourcepack",
            ContentKind::Shader => "shader",
            ContentKind::Modpack => "modpack",
        };
        facets.push(vec![format!("project_type:{ptype}")]);
        if q.kind == ContentKind::DataPack {
            facets.push(vec!["categories:datapack".into()]);
        }
        if let Some(mc) = q.mc.as_deref().filter(|s| !s.is_empty()) {
            facets.push(vec![format!("versions:{mc}")]);
        }
        if matches!(q.kind, ContentKind::Mod | ContentKind::Modpack) {
            if let Some(l) = q.loader {
                let names = loader_names(l);
                if !names.is_empty() {
                    facets.push(names.iter().map(|n| format!("categories:{n}")).collect());
                }
            }
        }
        for c in &q.categories {
            facets.push(vec![format!("categories:{c}")]);
        }
        let mut url = parse_url(&format!("{}/search", self.base))?;
        url.query_pairs_mut()
            .append_pair("query", q.text.trim())
            .append_pair(
                "facets",
                &serde_json::to_string(&facets).unwrap_or_else(|_| "[]".into()),
            )
            .append_pair("index", sort_index(q.sort))
            .append_pair("offset", &q.offset.to_string())
            .append_pair("limit", &q.limit.clamp(1, 100).to_string());
        Ok(url.to_string())
    }

    async fn raw_version(&self, id: &str) -> AppResult<MrVersion> {
        let id = Self::id_segment(id)?;
        self.api
            .get(&format!("{}/version/{id}", self.base), &[], TTL_IMMUTABLE)
            .await
    }

    async fn raw_versions(
        &self,
        project: &str,
        filter: &ContentFilter,
    ) -> AppResult<Vec<MrVersion>> {
        let project = Self::id_segment(project)?;
        let mut url = parse_url(&format!("{}/project/{project}/version", self.base))?;
        {
            let mut q = url.query_pairs_mut();
            if let Some(l) = filter.loader {
                q.append_pair("loaders", &json_list(&loader_names(l)));
            }
            if let Some(mc) = filter.mc.as_deref().filter(|s| !s.is_empty()) {
                q.append_pair("game_versions", &json_list(&[mc]));
            }
        }
        self.api.get(url.as_str(), &[], TTL_VERSIONS).await
    }

    /// `.mrpack` of a modpack version (`None` = latest, see [`pick_pack_version`]).
    pub(crate) async fn pack_file(
        &self,
        project_id: &str,
        version_id: Option<&str>,
    ) -> AppResult<PackFile> {
        let id = Self::id_segment(project_id)?;
        let p: Project = self
            .api
            .get(&format!("{}/project/{id}", self.base), &[], TTL_META)
            .await?;
        if kind_of(&p.project_type, &p.loaders) != ContentKind::Modpack {
            return Err(
                AppError::new(ErrorCode::InvalidInput, "project is not a modpack")
                    .detail("project", project_id),
            );
        }
        let v = match version_id {
            Some(vid) => {
                let v = self.raw_version(vid).await?;
                if v.project_id != p.id {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        "version belongs to another project",
                    )
                    .detail("version", vid));
                }
                v
            }
            None => {
                let list = self.raw_versions(&p.id, &ContentFilter::default()).await?;
                pick_pack_version(&list).cloned().ok_or_else(|| {
                    AppError::new(ErrorCode::NotFound, "modpack has no .mrpack versions")
                        .detail("project", project_id)
                })?
            }
        };
        let f = mrpack_file(&v).ok_or_else(|| {
            AppError::new(ErrorCode::NotFound, "version has no .mrpack file")
                .detail("version", &v.id)
        })?;
        let sha1 = f.hashes.get("sha1").cloned().ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput, "modpack file has no sha1")
                .detail("version", &v.id)
        })?;
        Ok(PackFile {
            version_id: v.id.clone(),
            file_name: f.filename.clone(),
            url: f.url.clone(),
            sha1,
            sha512: f.hashes.get("sha512").cloned(),
            size: f.size,
            title: p.title,
            icon_url: p.icon_url.filter(|s| !s.is_empty()),
        })
    }
}

/// The `.mrpack` of a version: the primary one, else the first.
fn mrpack_file(v: &MrVersion) -> Option<&MrFile> {
    let packs = || v.files.iter().filter(|f| f.filename.ends_with(".mrpack"));
    packs().find(|f| f.primary).or_else(|| packs().next())
}

/// Latest installable pack version from a newest-first list: featured release,
/// then any release, then the newest version with a `.mrpack`.
fn pick_pack_version(list: &[MrVersion]) -> Option<&MrVersion> {
    let ok = |v: &&MrVersion| mrpack_file(v).is_some();
    let release = |v: &&MrVersion| v.version_type == "release";
    list.iter()
        .filter(ok)
        .find(|v| v.featured && release(v))
        .or_else(|| list.iter().filter(ok).find(release))
        .or_else(|| list.iter().find(ok))
}

#[async_trait]
impl ContentSource for ModrinthSource {
    fn kind(&self) -> ContentSourceKind {
        ContentSourceKind::Modrinth
    }

    async fn search(&self, query: &SearchQuery) -> AppResult<SearchPage> {
        let url = self.search_url(query)?;
        let r: SearchResp = self.api.get(&url, &[], TTL_SEARCH).await?;
        Ok(SearchPage {
            items: r
                .hits
                .into_iter()
                .map(|h| {
                    let mut kind = kind_of(&h.project_type, &[]);
                    if query.kind == ContentKind::DataPack {
                        kind = ContentKind::DataPack;
                    }
                    ProjectSummary {
                        source: ContentSourceKind::Modrinth,
                        slug: h.slug.unwrap_or_else(|| h.project_id.clone()),
                        project_id: h.project_id,
                        title: h.title,
                        description: h.description,
                        author: h.author,
                        icon_url: h.icon_url.filter(|s| !s.is_empty()),
                        downloads: h.downloads,
                        kind,
                        categories: h.categories,
                    }
                })
                .collect(),
            total: r.total_hits,
            offset: r.offset,
        })
    }

    async fn versions(
        &self,
        project_id: &str,
        filter: &ContentFilter,
    ) -> AppResult<Vec<ContentVersion>> {
        Ok(self
            .versions_ranked(project_id, filter)
            .await?
            .into_iter()
            .map(|(v, _)| v)
            .collect())
    }

    async fn version(&self, version_id: &str) -> AppResult<ContentVersion> {
        to_content_version(&self.raw_version(version_id).await?)
    }
}

#[async_trait]
impl Catalog for ModrinthSource {
    async fn project(&self, id: &str) -> AppResult<ProjectInfo> {
        let id = Self::id_segment(id)?;
        let p: Project = self
            .api
            .get(&format!("{}/project/{id}", self.base), &[], TTL_META)
            .await?;
        Ok(ProjectInfo {
            kind: kind_of(&p.project_type, &p.loaders),
            page_url: format!("https://modrinth.com/{}/{}", p.project_type, p.slug),
            id: p.id,
            slug: p.slug,
            title: p.title,
        })
    }

    async fn versions_ranked(
        &self,
        project_id: &str,
        filter: &ContentFilter,
    ) -> AppResult<Vec<(ContentVersion, bool)>> {
        // Non-mod content is not filtered by mod loader.
        let mut filter = filter.clone();
        if filter.loader.is_some() {
            let kind = self.project(project_id).await?.kind;
            if !matches!(kind, ContentKind::Mod | ContentKind::Modpack) {
                filter.loader = None;
            }
        }
        let raw = self.raw_versions(project_id, &filter).await?;
        Ok(raw
            .iter()
            .filter_map(|v| {
                to_content_version(v)
                    .ok()
                    .map(|c| (c, v.version_type == "release"))
            })
            .collect())
    }

    async fn file(&self, v: &ContentVersion) -> AppResult<RemoteFile> {
        let raw = self.raw_version(&v.version_id).await?;
        let f = primary(&raw).ok_or_else(|| {
            AppError::new(ErrorCode::NotFound, "version has no files")
                .detail("version", &v.version_id)
        })?;
        Ok(RemoteFile {
            url: Some(f.url.clone()),
            sha1: f.hashes.get("sha1").cloned(),
            sha512: f.hashes.get("sha512").cloned(),
            manual_url: format!(
                "https://modrinth.com/project/{}/version/{}",
                raw.project_id, raw.id
            ),
        })
    }

    async fn updates(
        &self,
        items: &[UpdateItem],
        target: &Target,
        fresh: bool,
    ) -> AppResult<HashMap<String, ContentVersion>> {
        let mut out = HashMap::new();
        let kinds = [
            ContentKind::Mod,
            ContentKind::ResourcePack,
            ContentKind::Shader,
            ContentKind::DataPack,
        ];
        for kind in kinds {
            let group: Vec<&UpdateItem> = items
                .iter()
                .filter(|i| i.kind == kind && i.sha1.is_some())
                .collect();
            if group.is_empty() {
                continue;
            }
            let loaders = kind_loaders(kind, target.loader);
            if loaders.is_empty() {
                continue;
            }
            let hashes: Vec<&str> = group.iter().filter_map(|i| i.sha1.as_deref()).collect();
            let body = serde_json::json!({
                "hashes": hashes,
                "algorithm": "sha1",
                "loaders": loaders,
                "game_versions": [target.mc],
            });
            let ttl = if fresh {
                std::time::Duration::ZERO
            } else {
                TTL_VERSIONS
            };
            let map: HashMap<String, MrVersion> = self
                .api
                .post(
                    &format!("{}/version_files/update", self.base),
                    &[],
                    &body,
                    ttl,
                )
                .await?;
            for item in group {
                let Some(sha) = &item.sha1 else { continue };
                if let Some(v) = map.get(sha) {
                    if v.id != item.version_id {
                        if let Ok(cv) = to_content_version(v) {
                            out.insert(item.project_id.clone(), cv);
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_types::content::SortBy;

    #[test]
    fn parses_version_fixture() {
        let v: MrVersion =
            serde_json::from_str(include_str!("../tests/fixtures/modrinth/version_iris.json"))
                .unwrap();
        let c = to_content_version(&v).unwrap();
        assert_eq!(c.project_id, "YL57xq9U");
        assert_eq!(c.loaders, vec![LoaderKind::Fabric, LoaderKind::NeoForge]);
        assert_eq!(c.file_name, "iris-fabric-1.8.8+mc1.21.1.jar");
        assert!(c.sha1.is_some());
        assert_eq!(c.dependencies.len(), 2);
        assert_eq!(c.dependencies[0].kind, DependencyKind::Required);
        assert_eq!(c.dependencies[0].project_id, "AANobbMI");
    }

    #[test]
    fn search_url_has_facets() {
        let api = Arc::new(Api::new(
            Arc::new(crate::api::fake::FakeHttp::default()),
            std::env::temp_dir(),
        ));
        let s = ModrinthSource::new(api);
        let url = s
            .search_url(&SearchQuery {
                source: ContentSourceKind::Modrinth,
                kind: ContentKind::Mod,
                text: "sodium".into(),
                mc: Some("1.21.1".into()),
                loader: Some(LoaderKind::Quilt),
                categories: vec![],
                sort: SortBy::Downloads,
                offset: 0,
                limit: 20,
            })
            .unwrap();
        let u = url::Url::parse(&url).unwrap();
        let facets = u
            .query_pairs()
            .find(|(k, _)| k == "facets")
            .unwrap()
            .1
            .to_string();
        assert_eq!(
            facets,
            r#"[["project_type:mod"],["versions:1.21.1"],["categories:quilt","categories:fabric"]]"#
        );
        assert!(url.contains("index=downloads"));

        let url = s
            .search_url(&SearchQuery {
                source: ContentSourceKind::Modrinth,
                kind: ContentKind::Modpack,
                text: String::new(),
                mc: None,
                loader: Some(LoaderKind::Fabric),
                categories: vec![],
                sort: SortBy::Relevance,
                offset: 0,
                limit: 20,
            })
            .unwrap();
        let u = url::Url::parse(&url).unwrap();
        let facets = u.query_pairs().find(|(k, _)| k == "facets").unwrap().1;
        assert_eq!(
            facets,
            r#"[["project_type:modpack"],["categories:fabric"]]"#
        );
    }

    fn pv(id: &str, vtype: &str, featured: bool, file: &str) -> MrVersion {
        serde_json::from_value(serde_json::json!({
            "id": id, "project_id": "P", "version_type": vtype, "featured": featured,
            "files": [{"url": "https://cdn.modrinth.com/x", "filename": file, "primary": true}],
        }))
        .unwrap()
    }

    #[test]
    fn pack_version_preference() {
        let list = vec![
            pv("beta", "beta", true, "a.mrpack"),
            pv("rel-new", "release", false, "b.mrpack"),
            pv("rel-feat", "release", true, "c.mrpack"),
        ];
        assert_eq!(pick_pack_version(&list).unwrap().id, "rel-feat");
        assert_eq!(pick_pack_version(&list[..2]).unwrap().id, "rel-new");
        assert_eq!(pick_pack_version(&list[..1]).unwrap().id, "beta");
        let no_pack = vec![pv("x", "release", true, "x.zip")];
        assert!(pick_pack_version(&no_pack).is_none());
    }
}
