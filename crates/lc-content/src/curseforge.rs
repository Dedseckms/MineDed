//! CurseForge API v1. The key comes only from `CURSEFORGE_API_KEY` (environment
//! or a `.env` file); it is never stored in the repository or logged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
use crate::source::{Catalog, ProjectInfo, RemoteFile, Target, UpdateItem};
use crate::util::{loader_from_str, parse_url};

pub(crate) const CF_API: &str = "https://api.curseforge.com";
const GAME_MINECRAFT: u32 = 432;
pub(crate) const ENV_KEY: &str = "CURSEFORGE_API_KEY";

/// Error returned for every CurseForge call when no key is configured.
pub(crate) fn no_key_error() -> AppError {
    AppError::new(
        ErrorCode::AuthRequired,
        "CurseForge is disabled: set CURSEFORGE_API_KEY in the environment or .env",
    )
    .detail("source", "curseForge")
}

/// `CURSEFORGE_API_KEY` from the environment, else from the first `.env` found in `dirs`.
pub(crate) fn find_api_key(dirs: &[PathBuf]) -> Option<String> {
    if let Ok(v) = std::env::var(ENV_KEY) {
        let v = v.trim().to_owned();
        if !v.is_empty() {
            return Some(v);
        }
    }
    dirs.iter()
        .find_map(|d| std::fs::read_to_string(d.join(".env")).ok())
        .and_then(|text| parse_dotenv(&text, ENV_KEY))
}

pub(crate) fn parse_dotenv(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (k, v) = line.split_once('=')?;
        if k.trim() != key {
            return None;
        }
        let v = v.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(v);
        (!v.is_empty()).then(|| v.to_owned())
    })
}

/// Default `.env` locations: current dir, executable dir, launcher data root.
pub(crate) fn default_env_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(d) = std::env::current_dir() {
        dirs.push(d);
    }
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        dirs.push(d);
    }
    dirs.push(root.to_path_buf());
    dirs
}

/// CurseForge catalogue.
pub struct CurseForgeSource {
    api: Arc<Api>,
    base: String,
    key: String,
}

impl std::fmt::Debug for CurseForgeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurseForgeSource")
            .field("base", &self.base)
            .field("key", &"***")
            .finish()
    }
}

#[derive(Debug, Deserialize)]
struct Data<T> {
    data: T,
    #[serde(default)]
    pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    total_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfMod {
    id: u64,
    name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    download_count: f64,
    #[serde(default)]
    logo: Option<CfLogo>,
    #[serde(default)]
    authors: Vec<CfAuthor>,
    #[serde(default)]
    categories: Vec<CfCategory>,
    #[serde(default)]
    class_id: Option<u32>,
    #[serde(default)]
    links: Option<CfLinks>,
    #[serde(default)]
    allow_mod_distribution: Option<bool>,
    #[serde(default)]
    latest_files_indexes: Vec<CfFileIndex>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfLogo {
    #[serde(default)]
    thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CfAuthor {
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CfCategory {
    #[serde(default)]
    slug: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfLinks {
    #[serde(default)]
    website_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfFileIndex {
    game_version: String,
    file_id: u64,
    #[serde(default)]
    release_type: u32,
    #[serde(default)]
    mod_loader: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CfFile {
    id: u64,
    mod_id: u64,
    #[serde(default)]
    display_name: String,
    file_name: String,
    #[serde(default)]
    release_type: u32,
    #[serde(default)]
    file_date: String,
    #[serde(default)]
    file_length: u64,
    #[serde(default)]
    hashes: Vec<CfHash>,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    dependencies: Vec<CfDep>,
}

#[derive(Debug, Clone, Deserialize)]
struct CfHash {
    value: String,
    algo: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfDep {
    mod_id: u64,
    relation_type: u32,
}

pub(crate) fn kind_of_class(class_id: Option<u32>) -> ContentKind {
    match class_id {
        Some(12) => ContentKind::ResourcePack,
        Some(6552) => ContentKind::Shader,
        Some(4471) => ContentKind::Modpack,
        Some(6945) => ContentKind::DataPack,
        _ => ContentKind::Mod,
    }
}

fn class_of_kind(kind: ContentKind) -> u32 {
    match kind {
        ContentKind::Mod => 6,
        ContentKind::ResourcePack => 12,
        ContentKind::Shader => 6552,
        ContentKind::Modpack => 4471,
        ContentKind::DataPack => 6945,
    }
}

pub(crate) fn loader_type(l: LoaderKind) -> Option<u32> {
    match l {
        LoaderKind::Forge => Some(1),
        LoaderKind::Fabric => Some(4),
        LoaderKind::Quilt => Some(5),
        LoaderKind::NeoForge => Some(6),
        LoaderKind::Vanilla => None,
    }
}

fn mod_allows(m: &CfMod) -> bool {
    m.allow_mod_distribution != Some(false)
}

fn mod_page(m: &CfMod) -> String {
    m.links
        .as_ref()
        .and_then(|l| l.website_url.clone())
        .filter(|u| u.starts_with("https://"))
        .unwrap_or_else(|| format!("https://www.curseforge.com/minecraft/mc-mods/{}", m.slug))
}

/// `allowModDistribution=false` (or no download URL) → manual download.
pub(crate) fn to_content_version(f: &CfFile, m: &CfMod) -> ContentVersion {
    ContentVersion {
        source: ContentSourceKind::CurseForge,
        project_id: f.mod_id.to_string(),
        version_id: f.id.to_string(),
        name: f.display_name.clone(),
        version_number: f.display_name.clone(),
        mc_versions: f
            .game_versions
            .iter()
            .filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .cloned()
            .collect(),
        loaders: f
            .game_versions
            .iter()
            .filter_map(|v| loader_from_str(v))
            .collect(),
        published: f.file_date.clone(),
        file_name: f.file_name.clone(),
        size: f.file_length,
        sha1: f
            .hashes
            .iter()
            .find(|h| h.algo == 1)
            .map(|h| h.value.to_ascii_lowercase()),
        dependencies: f
            .dependencies
            .iter()
            .filter_map(|d| {
                let kind = match d.relation_type {
                    3 => DependencyKind::Required,
                    2 => DependencyKind::Optional,
                    5 => DependencyKind::Incompatible,
                    1 => DependencyKind::Embedded,
                    _ => return None,
                };
                Some(Dependency {
                    project_id: d.mod_id.to_string(),
                    version_id: None,
                    kind,
                })
            })
            .collect(),
        manual_download: !mod_allows(m) || f.download_url.is_none(),
    }
}

/// Direct CDN URL when the API omits `downloadUrl` for a distributable file.
fn cdn_url(f: &CfFile) -> String {
    format!(
        "https://edge.forgecdn.net/files/{}/{}/{}",
        f.id / 1000,
        f.id % 1000,
        f.file_name
    )
}

fn numeric(id: &str) -> AppResult<u64> {
    id.parse::<u64>()
        .map_err(|_| AppError::new(ErrorCode::InvalidInput, "bad curseforge id").detail("id", id))
}

impl CurseForgeSource {
    pub(crate) fn new(api: Arc<Api>, key: String) -> Self {
        Self::with_base(api, key, CF_API)
    }

    pub(crate) fn with_base(api: Arc<Api>, key: String, base: &str) -> Self {
        Self {
            api,
            key,
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        vec![("x-api-key", self.key.clone())]
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str, ttl: Duration) -> AppResult<T> {
        self.api.get(url, &self.headers(), ttl).await
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
        ttl: Duration,
    ) -> AppResult<T> {
        self.api
            .post(&format!("{}{path}", self.base), &self.headers(), &body, ttl)
            .await
    }

    pub(crate) async fn raw_mod(&self, id: &str) -> AppResult<CfMod> {
        let id = numeric(id)?;
        let d: Data<CfMod> = self
            .get(&format!("{}/v1/mods/{id}", self.base), TTL_META)
            .await?;
        Ok(d.data)
    }

    pub(crate) async fn raw_mods(&self, ids: &[u64], ttl: Duration) -> AppResult<Vec<CfMod>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let d: Data<Vec<CfMod>> = self
            .post("/v1/mods", serde_json::json!({ "modIds": ids }), ttl)
            .await?;
        Ok(d.data)
    }

    pub(crate) async fn raw_files(&self, ids: &[u64]) -> AppResult<Vec<CfFile>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let d: Data<Vec<CfFile>> = self
            .post(
                "/v1/mods/files",
                serde_json::json!({ "fileIds": ids }),
                TTL_IMMUTABLE,
            )
            .await?;
        Ok(d.data)
    }

    /// Files by id with their mods (for packs): (version, kind, remote file).
    pub(crate) async fn files_with_mods(
        &self,
        file_ids: &[u64],
    ) -> AppResult<Vec<(ContentVersion, ContentKind, RemoteFile)>> {
        let files = self.raw_files(file_ids).await?;
        let mut mod_ids: Vec<u64> = files.iter().map(|f| f.mod_id).collect();
        mod_ids.sort_unstable();
        mod_ids.dedup();
        let mods: HashMap<u64, CfMod> = self
            .raw_mods(&mod_ids, TTL_META)
            .await?
            .into_iter()
            .map(|m| (m.id, m))
            .collect();
        let mut out = Vec::new();
        for f in &files {
            let Some(m) = mods.get(&f.mod_id) else {
                continue;
            };
            out.push((
                to_content_version(f, m),
                kind_of_class(m.class_id),
                remote_file(f, m),
            ));
        }
        Ok(out)
    }

    pub(crate) fn search_url(&self, q: &SearchQuery) -> AppResult<String> {
        let mut url = parse_url(&format!("{}/v1/mods/search", self.base))?;
        {
            let mut p = url.query_pairs_mut();
            p.append_pair("gameId", &GAME_MINECRAFT.to_string())
                .append_pair("classId", &class_of_kind(q.kind).to_string());
            if !q.text.trim().is_empty() {
                p.append_pair("searchFilter", q.text.trim());
            }
            if let Some(mc) = q.mc.as_deref().filter(|s| !s.is_empty()) {
                p.append_pair("gameVersion", mc);
            }
            if matches!(q.kind, ContentKind::Mod | ContentKind::Modpack) {
                if let Some(t) = q.loader.and_then(loader_type) {
                    p.append_pair("modLoaderType", &t.to_string());
                }
            }
            let sort = match q.sort {
                SortBy::Relevance => None,
                SortBy::Downloads => Some(6),
                SortBy::Updated => Some(3),
                SortBy::Newest => Some(11),
            };
            if let Some(s) = sort {
                p.append_pair("sortField", &s.to_string())
                    .append_pair("sortOrder", "desc");
            }
            // CurseForge rejects index + pageSize > 10000.
            let size = q.limit.clamp(1, 50);
            let index = q.offset.min(10_000 - size);
            p.append_pair("index", &index.to_string())
                .append_pair("pageSize", &size.to_string());
        }
        Ok(url.to_string())
    }
}

fn remote_file(f: &CfFile, m: &CfMod) -> RemoteFile {
    let allowed = mod_allows(m);
    RemoteFile {
        url: if allowed {
            Some(f.download_url.clone().unwrap_or_else(|| cdn_url(f)))
        } else {
            None
        },
        sha1: f
            .hashes
            .iter()
            .find(|h| h.algo == 1)
            .map(|h| h.value.to_ascii_lowercase()),
        sha512: None,
        manual_url: format!("{}/download/{}", mod_page(m), f.id),
    }
}

#[async_trait]
impl ContentSource for CurseForgeSource {
    fn kind(&self) -> ContentSourceKind {
        ContentSourceKind::CurseForge
    }

    async fn search(&self, query: &SearchQuery) -> AppResult<SearchPage> {
        let url = self.search_url(query)?;
        let d: Data<Vec<CfMod>> = self.get(&url, TTL_SEARCH).await?;
        let pag = d.pagination.unwrap_or(Pagination {
            index: query.offset,
            total_count: 0,
        });
        Ok(SearchPage {
            items: d
                .data
                .into_iter()
                .map(|m| ProjectSummary {
                    source: ContentSourceKind::CurseForge,
                    project_id: m.id.to_string(),
                    slug: m.slug.clone(),
                    title: m.name.clone(),
                    description: m.summary.clone(),
                    author: m
                        .authors
                        .first()
                        .map(|a| a.name.clone())
                        .unwrap_or_default(),
                    icon_url: m.logo.as_ref().and_then(|l| l.thumbnail_url.clone()),
                    downloads: m.download_count.max(0.0) as u64,
                    kind: kind_of_class(m.class_id),
                    categories: m.categories.iter().map(|c| c.slug.clone()).collect(),
                })
                .collect(),
            total: pag.total_count.min(10_000),
            offset: pag.index,
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
        let id = numeric(version_id)?;
        let f = self
            .raw_files(&[id])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                AppError::new(ErrorCode::NotFound, "file not found").detail("id", version_id)
            })?;
        let m = self.raw_mod(&f.mod_id.to_string()).await?;
        Ok(to_content_version(&f, &m))
    }
}

#[async_trait]
impl Catalog for CurseForgeSource {
    async fn project(&self, id: &str) -> AppResult<ProjectInfo> {
        let m = self.raw_mod(id).await?;
        Ok(ProjectInfo {
            id: m.id.to_string(),
            slug: m.slug.clone(),
            title: m.name.clone(),
            kind: kind_of_class(m.class_id),
            page_url: mod_page(&m),
        })
    }

    async fn versions_ranked(
        &self,
        project_id: &str,
        filter: &ContentFilter,
    ) -> AppResult<Vec<(ContentVersion, bool)>> {
        let m = self.raw_mod(project_id).await?;
        let kind = kind_of_class(m.class_id);
        let mut url = parse_url(&format!("{}/v1/mods/{}/files", self.base, m.id))?;
        {
            let mut p = url.query_pairs_mut();
            if let Some(mc) = filter.mc.as_deref().filter(|s| !s.is_empty()) {
                p.append_pair("gameVersion", mc);
            }
            if matches!(kind, ContentKind::Mod | ContentKind::Modpack) {
                if let Some(t) = filter.loader.and_then(loader_type) {
                    p.append_pair("modLoaderType", &t.to_string());
                }
            }
            p.append_pair("pageSize", "50");
        }
        let d: Data<Vec<CfFile>> = self.get(url.as_str(), TTL_VERSIONS).await?;
        let mut files = d.data;
        // Newest first.
        files.sort_by(|a, b| b.file_date.cmp(&a.file_date).then(b.id.cmp(&a.id)));
        Ok(files
            .iter()
            .map(|f| (to_content_version(f, &m), f.release_type == 1))
            .collect())
    }

    async fn file(&self, v: &ContentVersion) -> AppResult<RemoteFile> {
        let id = numeric(&v.version_id)?;
        let f = self
            .raw_files(&[id])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                AppError::new(ErrorCode::NotFound, "file not found").detail("id", &v.version_id)
            })?;
        let m = self.raw_mod(&f.mod_id.to_string()).await?;
        Ok(remote_file(&f, &m))
    }

    async fn updates(
        &self,
        items: &[UpdateItem],
        target: &Target,
        fresh: bool,
    ) -> AppResult<HashMap<String, ContentVersion>> {
        let ids: Vec<u64> = items
            .iter()
            .filter_map(|i| i.project_id.parse().ok())
            .collect();
        let ttl = if fresh { Duration::ZERO } else { TTL_VERSIONS };
        let mods = self.raw_mods(&ids, ttl).await?;
        let mut newest: HashMap<u64, u64> = HashMap::new();
        for m in &mods {
            let kind = kind_of_class(m.class_id);
            let want_loader = loader_type(target.loader);
            let best = m
                .latest_files_indexes
                .iter()
                .filter(|x| x.game_version == target.mc && x.release_type == 1)
                .filter(|x| {
                    kind != ContentKind::Mod
                        || x.mod_loader.is_none()
                        || x.mod_loader == want_loader
                        || (target.loader == LoaderKind::Quilt && x.mod_loader == Some(4))
                })
                .map(|x| x.file_id)
                .max();
            if let Some(b) = best {
                newest.insert(m.id, b);
            }
        }
        let mut wanted = Vec::new();
        for i in items {
            let (Ok(pid), Ok(cur)) = (i.project_id.parse::<u64>(), i.version_id.parse::<u64>())
            else {
                continue;
            };
            if let Some(&n) = newest.get(&pid) {
                if n > cur {
                    wanted.push(n);
                }
            }
        }
        let mut out = HashMap::new();
        for (v, _, _) in self.files_with_mods(&wanted).await? {
            out.insert(v.project_id.clone(), v);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (CfFile, CfMod, CfMod) {
        let f: Data<Vec<CfFile>> =
            serde_json::from_str(include_str!("../tests/fixtures/curseforge/files.json")).unwrap();
        let m: Data<Vec<CfMod>> =
            serde_json::from_str(include_str!("../tests/fixtures/curseforge/mods.json")).unwrap();
        let mut mods = m.data.into_iter();
        (
            f.data.into_iter().next().unwrap(),
            mods.next().unwrap(),
            mods.next().unwrap(),
        )
    }

    #[test]
    fn opted_out_mod_is_manual() {
        let (file, allowed, denied) = fixture();
        assert_eq!(denied.allow_mod_distribution, Some(false));
        let v = to_content_version(&file, &denied);
        assert!(v.manual_download);
        let r = remote_file(&file, &denied);
        assert_eq!(r.url, None);
        assert_eq!(
            r.manual_url,
            "https://www.curseforge.com/minecraft/mc-mods/denied-mod/download/4711"
        );

        let v = to_content_version(&file, &allowed);
        assert!(!v.manual_download);
        assert_eq!(v.mc_versions, vec!["1.20.1"]);
        assert_eq!(v.loaders, vec![LoaderKind::Forge]);
        assert_eq!(
            v.sha1.as_deref(),
            Some("da39a3ee5e6b4b0d3255bfef95601890afd80709")
        );
        assert_eq!(v.dependencies.len(), 3);
        assert_eq!(v.dependencies[0].kind, DependencyKind::Required);
        assert_eq!(v.dependencies[1].kind, DependencyKind::Incompatible);
        assert_eq!(v.dependencies[2].kind, DependencyKind::Optional);
        assert!(remote_file(&file, &allowed).url.is_some());
    }

    #[test]
    fn dotenv_parsing() {
        let t = "# c\nOTHER=1\nexport CURSEFORGE_API_KEY=\"$2a$10$abc\"\n";
        assert_eq!(parse_dotenv(t, ENV_KEY).as_deref(), Some("$2a$10$abc"));
        assert_eq!(parse_dotenv("CURSEFORGE_API_KEY=", ENV_KEY), None);
    }

    #[test]
    fn debug_hides_key() {
        let api = Arc::new(Api::new(
            Arc::new(crate::api::fake::FakeHttp::default()),
            std::env::temp_dir(),
        ));
        let s = CurseForgeSource::new(api, "secret-key".into());
        assert!(!format!("{s:?}").contains("secret-key"));
    }
}
