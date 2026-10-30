//! CurseForge catalogue through the launcher hub broker; credentials stay on the server.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::content::{
    ContentFilter, ContentKind, ContentSourceKind, ContentVersion, Dependency, DependencyKind,
    ProjectSummary, SearchPage, SearchQuery, SortBy,
};
use lc_types::version::LoaderKind;
use lc_types::{AppError, AppResult, ErrorCode};
use serde::Deserialize;

use crate::api::{Api, TTL_SEARCH, TTL_VERSIONS};
use crate::contract::ContentSource;
use crate::source::{Catalog, PackFile, ProjectInfo, RemoteFile, Target, UpdateItem};
use crate::util::{check_file_name, loader_from_str, parse_url};

/// CurseForge metadata adapter backed by a restricted hub broker.
pub struct CurseForgeSource {
    api: Arc<Api>,
    base: String,
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
    #[serde(default)]
    pagination: Pagination,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    total_count: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: u32,
    game_id: u32,
    name: String,
    slug: String,
    class_id: Option<u32>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    download_count: f64,
    #[serde(default)]
    authors: Vec<Author>,
    #[serde(default)]
    categories: Vec<Category>,
    logo: Option<Logo>,
    allow_mod_distribution: Option<bool>,
}
#[derive(Deserialize)]
struct Author {
    name: String,
}
#[derive(Deserialize)]
struct Category {
    id: u32,
}
#[derive(Deserialize)]
struct Logo {
    url: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    id: u32,
    game_id: u32,
    mod_id: u32,
    is_available: bool,
    display_name: String,
    file_name: String,
    file_date: String,
    file_length: u64,
    release_type: u32,
    download_url: Option<String>,
    #[serde(default)]
    hashes: Vec<Hash>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    dependencies: Vec<Dep>,
}
#[derive(Clone, Deserialize)]
struct Hash {
    algo: u32,
    value: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Dep {
    mod_id: u32,
    relation_type: u32,
}

fn bad(message: &str) -> AppError {
    AppError::new(ErrorCode::InvalidInput, message)
}
fn id(value: &str) -> AppResult<u32> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("invalid CurseForge ID"));
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0 && *n <= i32::MAX as u32)
        .ok_or_else(|| bad("invalid CurseForge ID"))
}
fn class(kind: ContentKind) -> u32 {
    match kind {
        ContentKind::Mod => 6,
        ContentKind::Modpack => 4471,
        ContentKind::ResourcePack => 12,
        ContentKind::Shader => 6552,
        ContentKind::DataPack => 6945,
    }
}
fn project_kind(p: &Project) -> AppResult<ContentKind> {
    match p.class_id {
        Some(6) => Ok(ContentKind::Mod),
        Some(4471) => Ok(ContentKind::Modpack),
        Some(12) => Ok(ContentKind::ResourcePack),
        Some(6552) => Ok(ContentKind::Shader),
        Some(6945) => Ok(ContentKind::DataPack),
        _ => Err(bad("unsupported CurseForge content class")),
    }
}
fn loader_id(loader: LoaderKind) -> &'static str {
    match loader {
        LoaderKind::Vanilla => "0",
        LoaderKind::Forge => "1",
        LoaderKind::Fabric => "4",
        LoaderKind::Quilt => "5",
        LoaderKind::NeoForge => "6",
    }
}
fn page(p: &Project) -> String {
    // Numeric project URLs avoid trusting an upstream link or slug.
    format!("https://www.curseforge.com/projects/{}", p.id)
}
pub(crate) fn allowed_cdn(value: &str) -> bool {
    parse_url(value).is_ok_and(|u| {
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port_or_known_default() == Some(443)
            && matches!(
                u.host_str(),
                Some("edge.forgecdn.net" | "mediafilez.forgecdn.net")
            )
    })
}
impl File {
    fn sha1(&self) -> Option<String> {
        self.hashes
            .iter()
            .find(|h| {
                h.algo == 1 && h.value.len() == 40 && h.value.bytes().all(|b| b.is_ascii_hexdigit())
            })
            .map(|h| h.value.to_ascii_lowercase())
    }
    fn version(&self, p: &Project) -> AppResult<ContentVersion> {
        if self.game_id != 432 || self.mod_id != p.id || self.id == 0 {
            return Err(bad("CurseForge file ownership mismatch"));
        }
        check_file_name(&self.file_name)?;
        Ok(ContentVersion {
            source: ContentSourceKind::CurseForge,
            project_id: self.mod_id.to_string(),
            version_id: self.id.to_string(),
            name: self.display_name.clone(),
            version_number: self.display_name.clone(),
            published: self.file_date.clone(),
            file_name: self.file_name.clone(),
            size: self.file_length,
            sha1: self.sha1(),
            mc_versions: self
                .game_versions
                .iter()
                .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
                .cloned()
                .collect(),
            loaders: self
                .game_versions
                .iter()
                .filter_map(|v| loader_from_str(&v.to_ascii_lowercase()))
                .collect(),
            manual_download: p.allow_mod_distribution == Some(false)
                || !self.is_available
                || self.download_url.is_none()
                || self.sha1().is_none(),
            dependencies: self
                .dependencies
                .iter()
                .filter_map(|d| {
                    let kind = match d.relation_type {
                        1 | 6 => DependencyKind::Embedded,
                        2 => DependencyKind::Optional,
                        3 => DependencyKind::Required,
                        5 => DependencyKind::Incompatible,
                        _ => return None,
                    };
                    Some(Dependency {
                        project_id: d.mod_id.to_string(),
                        version_id: None,
                        kind,
                    })
                })
                .collect(),
        })
    }
}
fn remote_file(f: &File, p: &Project) -> AppResult<RemoteFile> {
    let cv = f.version(p)?;
    let url = if cv.manual_download {
        None
    } else {
        Some(
            f.download_url
                .clone()
                .filter(|u| allowed_cdn(u))
                .ok_or_else(|| bad("CurseForge download URL is not permitted"))?,
        )
    };
    Ok(RemoteFile {
        url,
        sha1: cv.sha1,
        sha512: None,
        manual_url: format!("{}/files/{}", page(p), f.id),
    })
}

impl CurseForgeSource {
    pub(crate) fn new(api: Arc<Api>, hub: &str) -> Self {
        let hub = if hub.trim().is_empty() {
            lc_types::settings::DEFAULT_HUB_URL
        } else {
            hub.trim()
        };
        Self {
            api,
            base: format!("{}/catalog/curseforge", hub.trim_end_matches('/')),
        }
    }
    async fn raw_project(&self, value: &str) -> AppResult<Project> {
        let n = id(value)?;
        let r: Envelope<Project> = self
            .api
            .get_uncached(&format!("{}/mods/{n}", self.base))
            .await?;
        if r.data.id != n || r.data.game_id != 432 {
            return Err(bad("CurseForge project ownership mismatch"));
        }
        Ok(r.data)
    }
    async fn raw_file(&self, value: &str) -> AppResult<File> {
        let n = id(value)?;
        let r: Envelope<File> = self
            .api
            .get_uncached(&format!("{}/files/{n}", self.base))
            .await?;
        if r.data.id != n || r.data.game_id != 432 {
            return Err(bad("CurseForge file ownership mismatch"));
        }
        Ok(r.data)
    }
    async fn ranked(
        &self,
        project: &str,
        filter: &ContentFilter,
        fresh: bool,
    ) -> AppResult<Vec<(ContentVersion, bool)>> {
        let p = self.raw_project(project).await?;
        let kind = project_kind(&p)?;
        let mut list = Vec::new();
        for index in (0..10_000).step_by(50) {
            let mut u = parse_url(&format!("{}/mods/{}/files", self.base, p.id))?;
            {
                let mut q = u.query_pairs_mut();
                q.append_pair("pageSize", "50")
                    .append_pair("index", &index.to_string());
                if let Some(mc) = &filter.mc {
                    q.append_pair("gameVersion", mc);
                }
                if matches!(kind, ContentKind::Mod | ContentKind::Modpack) {
                    if let Some(l) = filter.loader {
                        q.append_pair("modLoaderType", loader_id(l));
                    }
                }
            }
            let r: Envelope<Vec<File>> = self
                .api
                .get(
                    u.as_str(),
                    &[],
                    if fresh { Duration::ZERO } else { TTL_VERSIONS },
                )
                .await?;
            let count = r.data.len();
            for f in r.data {
                let v = f.version(&p)?;
                if filter
                    .mc
                    .as_ref()
                    .is_some_and(|mc| !v.mc_versions.contains(mc))
                {
                    continue;
                }
                if matches!(kind, ContentKind::Mod | ContentKind::Modpack)
                    && filter.loader.is_some_and(|l| {
                        l != LoaderKind::Vanilla && !crate::util::loader_compatible(&v.loaders, l)
                    })
                {
                    continue;
                }
                list.push((v, f.release_type == 1));
            }
            if count < 50 || index + count >= r.pagination.total_count as usize {
                break;
            }
        }
        list.sort_by(|(a, _), (b, _)| {
            b.published.cmp(&a.published).then_with(|| {
                b.version_id
                    .parse::<u32>()
                    .unwrap_or(0)
                    .cmp(&a.version_id.parse::<u32>().unwrap_or(0))
            })
        });
        Ok(list)
    }
    /// Resolve manifest entries in paired batches of at most fifty; no stale permissions.
    pub(crate) async fn pack_entries(
        &self,
        entries: &[(u64, u64)],
        sink: &dyn ProgressSink,
    ) -> AppResult<Vec<(ContentVersion, ProjectInfo, RemoteFile)>> {
        let mut resolved = Vec::new();
        for chunk in entries.chunks(50) {
            sink.check_cancelled()?;
            let mut projects = Vec::new();
            let mut files = Vec::new();
            for (project, file) in chunk {
                let project = id(&project.to_string())?;
                let file = id(&file.to_string())?;
                if !projects.contains(&project) {
                    projects.push(project);
                }
                if !files.contains(&file) {
                    files.push(file);
                }
            }
            let projects_response: Envelope<Vec<Project>> = self
                .api
                .post_uncached(
                    &format!("{}/mods", self.base),
                    &serde_json::json!({"modIds":projects}),
                    sink,
                )
                .await?;
            let files_response: Envelope<Vec<File>> = self
                .api
                .post_uncached(
                    &format!("{}/files", self.base),
                    &serde_json::json!({"fileIds":files}),
                    sink,
                )
                .await?;
            let mut project_map = HashMap::new();
            for p in projects_response.data {
                if p.game_id != 432
                    || !projects.contains(&p.id)
                    || project_map.insert(p.id, p).is_some()
                {
                    return Err(bad("unexpected or duplicate CurseForge batch project"));
                }
            }
            let mut file_map = HashMap::new();
            for f in files_response.data {
                if f.game_id != 432 || !files.contains(&f.id) || file_map.insert(f.id, f).is_some()
                {
                    return Err(bad("unexpected or duplicate CurseForge batch file"));
                }
            }
            for (project, file) in chunk {
                sink.check_cancelled()?;
                let p = project_map.get(&(*project as u32)).ok_or_else(|| {
                    AppError::new(ErrorCode::NotFound, "CurseForge batch project missing")
                })?;
                let f = file_map.get(&(*file as u32)).ok_or_else(|| {
                    AppError::new(ErrorCode::NotFound, "CurseForge batch file missing")
                })?;
                let version = f.version(p)?;
                let remote = remote_file(f, p)?;
                let info = ProjectInfo {
                    kind: project_kind(p)?,
                    page_url: page(p),
                    id: p.id.to_string(),
                    slug: p.slug.clone(),
                    title: p.name.clone(),
                };
                resolved.push((version, info, remote));
            }
        }
        Ok(resolved)
    }

    pub(crate) async fn pack_file(
        &self,
        project: &str,
        version: Option<&str>,
    ) -> AppResult<PackFile> {
        let p = self.raw_project(project).await?;
        if project_kind(&p)? != ContentKind::Modpack {
            return Err(bad("project is not a modpack"));
        }
        let v = match version {
            Some(v) => self.version(v).await?,
            None => {
                let list = self
                    .ranked(project, &ContentFilter::default(), true)
                    .await?;
                list.iter()
                    .find(|(_, release)| *release)
                    .or_else(|| list.first())
                    .map(|(v, _)| v.clone())
                    .ok_or_else(|| AppError::new(ErrorCode::NotFound, "modpack has no files"))?
            }
        };
        if v.project_id != project {
            return Err(bad("modpack file ownership mismatch"));
        }
        let remote = self.file(&v).await?;
        let url = remote.url.ok_or_else(|| {
            AppError::new(
                ErrorCode::ManualDownloadRequired,
                "modpack requires manual download",
            )
            .detail("url", &remote.manual_url)
        })?;
        Ok(PackFile {
            version_id: v.version_id,
            file_name: v.file_name,
            url,
            sha1: remote.sha1.ok_or_else(|| bad("modpack has no SHA-1"))?,
            sha512: None,
            size: v.size,
            title: p.name,
            icon_url: p.logo.map(|l| l.url),
        })
    }
}
#[async_trait]
impl ContentSource for CurseForgeSource {
    fn kind(&self) -> ContentSourceKind {
        ContentSourceKind::CurseForge
    }
    async fn search(&self, q: &SearchQuery) -> AppResult<SearchPage> {
        let mut u = parse_url(&format!("{}/mods/search", self.base))?;
        {
            let mut pairs = u.query_pairs_mut();
            pairs
                .append_pair("gameId", "432")
                .append_pair("classId", &class(q.kind).to_string())
                .append_pair("searchFilter", q.text.trim())
                .append_pair("index", &q.offset.min(9950).to_string())
                .append_pair("pageSize", &q.limit.clamp(1, 50).to_string())
                .append_pair("sortOrder", "desc")
                .append_pair(
                    "sortField",
                    match q.sort {
                        SortBy::Relevance => "2",
                        SortBy::Downloads => "6",
                        SortBy::Updated => "3",
                        SortBy::Newest => "11",
                    },
                );
            if let Some(mc) = &q.mc {
                pairs.append_pair("gameVersion", mc);
            }
            if matches!(q.kind, ContentKind::Mod | ContentKind::Modpack) {
                if let Some(l) = q.loader {
                    pairs.append_pair("modLoaderType", loader_id(l));
                }
            }
            if q.categories.len() > 1 {
                return Err(bad("CurseForge supports one category filter"));
            }
            if let Some(c) = q.categories.first() {
                pairs.append_pair("categoryId", &id(c)?.to_string());
            }
        }
        let r: Envelope<Vec<Project>> = self.api.get(u.as_str(), &[], TTL_SEARCH).await?;
        let mut items = Vec::new();
        for p in r.data {
            if p.game_id != 432 || project_kind(&p)? != q.kind {
                return Err(bad("unexpected CurseForge search result"));
            }
            items.push(ProjectSummary {
                source: self.kind(),
                project_id: p.id.to_string(),
                slug: p.slug,
                title: p.name,
                description: p.summary,
                author: p
                    .authors
                    .into_iter()
                    .map(|a| a.name)
                    .collect::<Vec<_>>()
                    .join(", "),
                icon_url: p.logo.map(|l| l.url),
                downloads: p.download_count.max(0.0) as u64,
                kind: q.kind,
                categories: p.categories.into_iter().map(|c| c.id.to_string()).collect(),
            });
        }
        Ok(SearchPage {
            items,
            offset: r.pagination.index,
            total: r.pagination.total_count,
        })
    }
    async fn versions(
        &self,
        project_id: &str,
        filter: &ContentFilter,
    ) -> AppResult<Vec<ContentVersion>> {
        Ok(self
            .ranked(project_id, filter, false)
            .await?
            .into_iter()
            .map(|(v, _)| v)
            .collect())
    }
    async fn version(&self, version_id: &str) -> AppResult<ContentVersion> {
        let f = self.raw_file(version_id).await?;
        f.version(&self.raw_project(&f.mod_id.to_string()).await?)
    }
}
#[async_trait]
impl Catalog for CurseForgeSource {
    async fn project(&self, value: &str) -> AppResult<ProjectInfo> {
        let p = self.raw_project(value).await?;
        Ok(ProjectInfo {
            kind: project_kind(&p)?,
            page_url: page(&p),
            id: p.id.to_string(),
            slug: p.slug,
            title: p.name,
        })
    }
    async fn versions_ranked(
        &self,
        project: &str,
        filter: &ContentFilter,
    ) -> AppResult<Vec<(ContentVersion, bool)>> {
        self.ranked(project, filter, false).await
    }
    async fn file(&self, v: &ContentVersion) -> AppResult<RemoteFile> {
        let f = self.raw_file(&v.version_id).await?;
        let p = self.raw_project(&v.project_id).await?;
        remote_file(&f, &p)
    }

    async fn updates(
        &self,
        items: &[UpdateItem],
        target: &Target,
        fresh: bool,
    ) -> AppResult<HashMap<String, ContentVersion>> {
        let mut out = HashMap::new();
        for item in items {
            let list = self
                .ranked(&item.project_id, &target.filter(item.kind), fresh)
                .await?;
            if let Some((v, _)) = list
                .iter()
                .find(|(_, release)| *release)
                .or_else(|| list.first())
            {
                if v.version_id.parse::<u32>().unwrap_or(0)
                    > item.version_id.parse::<u32>().unwrap_or(0)
                {
                    out.insert(item.project_id.clone(), v.clone());
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_maps_hashes_loaders_and_dependency_kinds() {
        let f: File =
            serde_json::from_str(include_str!("../tests/fixtures/curseforge/file.json")).unwrap();
        let p: Project = serde_json::from_value(
            serde_json::json!({"id":1,"gameId":432,"classId":6,"name":"Example","slug":"example"}),
        )
        .unwrap();
        let v = f.version(&p).unwrap();
        assert_eq!(
            v.sha1.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(v.mc_versions, ["1.20.1"]);
        assert_eq!(v.loaders, [LoaderKind::Forge]);
        assert_eq!(
            v.dependencies.iter().map(|d| d.kind).collect::<Vec<_>>(),
            vec![
                DependencyKind::Required,
                DependencyKind::Incompatible,
                DependencyKind::Optional,
                DependencyKind::Embedded
            ]
        );
        assert!(!v.manual_download);
        assert!(allowed_cdn("https://mediafilez.forgecdn.net/files/x.jar"));
        for url in [
            "http://edge.forgecdn.net/files/x.jar",
            "https://edge.forgecdn.net.evil.example/x.jar",
            "https://user@edge.forgecdn.net/x.jar",
            "https://edge.forgecdn.net:444/x.jar",
        ] {
            assert!(!allowed_cdn(url));
        }
    }
}
