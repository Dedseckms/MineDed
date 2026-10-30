//! `ContentServices`: `ContentService` + `PackIo` + `PresetService` on top of
//! Modrinth and core's `InstanceService`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use lc_core::{InstanceService, SystemProbe};
use lc_http::{DownloadReq, Downloader, ResourceKind};
use lc_runtime::{Paths, ProgressSink};
use lc_types::content::{
    ContentFilter, ContentKind, ContentSourceKind, ContentVersion, ContentVersionRef,
    InstalledContent, LocalDepIssue, ResolvePlan, SearchPage, SearchQuery,
};
use lc_types::instance::Instance;
use lc_types::version::LoaderKind;
use lc_types::{AppError, AppResult, ErrorCode, InstanceId};

use crate::api::{Api, DlHttp, Http};
use crate::contract::ContentService;
use crate::curseforge::CurseForgeSource;
use crate::index::{base_name, key, ContentIndex, IndexEntry};
use crate::modmeta::{read_jar_meta_async, ModMeta};
use crate::modrinth::ModrinthSource;
use crate::resolver::{resolve, DiskMeta, InstalledRef, Lookup};
use crate::shaders::{has_shader_loader, optifine_manual, shader_loader_groups};
use crate::source::{Catalog, Target, UpdateItem};
use crate::util::{
    check_file_name, ensure_no_links, file_hashes, io_err, is_link, kind_dir, loader_compatible,
    parse_url, safe_join, sha1_hex, CONTENT_DIRS,
};

/// Jars larger than this are not fetched just to read their metadata.
const MAX_META_JAR: u64 = 64 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct Inner {
    api: Arc<Api>,
    pub paths: Paths,
    pub dl: Arc<dyn Downloader>,
    pub instances: Arc<dyn InstanceService>,
    pub system: Arc<dyn SystemProbe>,
    pub modrinth: Arc<ModrinthSource>,
    pub curseforge: Arc<CurseForgeSource>,
}

/// Real content services. Cheap to clone; implements `ContentService`,
/// `PackIo` and `PresetService`.
#[derive(Clone)]
pub struct ContentServices {
    pub(crate) inner: Arc<Inner>,
}

/// A file found in one of the content folders.
#[derive(Debug, Clone)]
pub(crate) struct DiskFile {
    pub dir: &'static str,
    pub kind: ContentKind,
    /// Name on disk (may end with `.disabled`).
    pub name: String,
    pub enabled: bool,
    /// Unpacked pack folder (not a mod).
    pub is_dir: bool,
}

impl DiskFile {
    pub fn base(&self) -> &str {
        base_name(&self.name)
    }

    pub fn key(&self) -> String {
        key(self.dir, &self.name)
    }
}

impl ContentServices {
    /// Uses Modrinth and the default hub CurseForge broker.
    pub fn new(
        paths: Paths,
        downloader: Arc<dyn Downloader>,
        instances: Arc<dyn InstanceService>,
        system: Arc<dyn SystemProbe>,
    ) -> Self {
        let http: Arc<dyn Http> = Arc::new(DlHttp::new(downloader.clone()));
        Self::with_parts(paths, downloader, instances, system, http)
    }

    /// Configure the hub API root used for the CurseForge catalogue.
    pub fn with_hub_url(mut self, hub: &str) -> Self {
        let inner = Arc::make_mut(&mut self.inner);
        inner.curseforge = Arc::new(CurseForgeSource::new(inner.api.clone(), hub));
        self
    }

    pub(crate) fn with_parts(
        paths: Paths,
        downloader: Arc<dyn Downloader>,
        instances: Arc<dyn InstanceService>,
        system: Arc<dyn SystemProbe>,
        http: Arc<dyn Http>,
    ) -> Self {
        let api = Arc::new(Api::new(http, paths.cache().join("content")));
        Self {
            inner: Arc::new(Inner {
                api: api.clone(),
                modrinth: Arc::new(ModrinthSource::new(api.clone())),
                curseforge: Arc::new(CurseForgeSource::new(
                    api,
                    lc_types::settings::DEFAULT_HUB_URL,
                )),
                paths,
                dl: downloader,
                instances,
                system,
            }),
        }
    }
}

pub(crate) fn target_of(inst: &Instance) -> Target {
    Target {
        mc: inst.mc.clone(),
        loader: inst.loader.kind,
    }
}

impl Inner {
    pub fn catalog(&self, source: ContentSourceKind) -> AppResult<Arc<dyn Catalog>> {
        match source {
            ContentSourceKind::Modrinth => Ok(self.modrinth.clone()),
            ContentSourceKind::CurseForge => Ok(self.curseforge.clone()),
            ContentSourceKind::Local => Err(AppError::new(
                ErrorCode::InvalidInput,
                "local files have no remote catalogue",
            )),
        }
    }

    pub fn instance_dir(&self, id: InstanceId) -> PathBuf {
        self.paths.instance(id)
    }

    /// Cache of jars fetched during resolution to read `fabric.mod.json`
    /// (keyed by SHA-1, reused by the install).
    fn jar_cache(&self, sha1: &str) -> PathBuf {
        self.paths
            .cache()
            .join("content")
            .join("jars")
            .join(format!(
                "{}.jar",
                sha1_hex(sha1.to_ascii_lowercase().as_bytes())
            ))
    }

    /// `fabric.mod.json` of the enabled mod jars in `game_dir` (Fabric/Quilt only).
    pub async fn disk_metas(
        &self,
        inst: &Instance,
        game_dir: &Path,
        idx: &ContentIndex,
    ) -> AppResult<Vec<DiskMeta>> {
        if !matches!(inst.loader.kind, LoaderKind::Fabric | LoaderKind::Quilt) {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for f in self.scan(game_dir).await? {
            if f.kind != ContentKind::Mod || !f.enabled || !f.name.ends_with(".jar") {
                continue;
            }
            let Some(meta) = read_jar_meta_async(game_dir.join(f.dir).join(&f.name)).await else {
                continue;
            };
            out.push(DiskMeta {
                owner: idx
                    .files
                    .get(&f.key())
                    .map(|e| (e.source, e.project_id.clone())),
                meta,
            });
        }
        Ok(out)
    }

    /// Files in `mods/`, `resourcepacks/`, `shaderpacks/`, `datapacks/`.
    pub async fn scan(&self, game_dir: &Path) -> AppResult<Vec<DiskFile>> {
        let mut out = Vec::new();
        for (dir, kind) in CONTENT_DIRS {
            let path = game_dir.join(dir);
            ensure_no_links(&path)?;
            let mut rd = match tokio::fs::read_dir(&path).await {
                Ok(rd) => rd,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_err(&path, e)),
            };
            while let Some(e) = rd.next_entry().await.map_err(|e| io_err(&path, e))? {
                let Ok(meta) = tokio::fs::symlink_metadata(e.path()).await else {
                    continue;
                };
                if is_link(&meta) {
                    continue;
                }
                let ft = meta.file_type();
                let name = e.file_name().to_string_lossy().into_owned();
                // Unpacked resource/shader/data packs are folders; hidden
                // names are temp files of `add_local`.
                let is_dir = ft.is_dir() && kind != ContentKind::Mod;
                if !(ft.is_file() || is_dir) || name.starts_with('.') {
                    continue;
                }
                let lower = name.to_ascii_lowercase();
                let base = base_name(&lower);
                if !is_dir && !(base.ends_with(".jar") || base.ends_with(".zip")) {
                    continue;
                }
                out.push(DiskFile {
                    dir,
                    kind,
                    enabled: !name.ends_with(".disabled"),
                    name,
                    is_dir,
                });
            }
        }
        out.sort_by(|a, b| a.dir.cmp(b.dir).then(a.name.cmp(&b.name)));
        Ok(out)
    }

    async fn find_file(&self, game_dir: &Path, file_name: &str) -> AppResult<DiskFile> {
        check_file_name(file_name)?;
        let want = base_name(file_name);
        self.scan(game_dir)
            .await?
            .into_iter()
            .find(|f| f.base() == want)
            .ok_or_else(|| {
                AppError::new(ErrorCode::NotFound, "content file not found")
                    .detail("file", file_name)
            })
    }

    /// Downloads `versions` into the instance, replacing older files of the same
    /// projects (keeping their disabled state) and updating the index.
    pub async fn install_versions(
        &self,
        id: InstanceId,
        versions: &[ContentVersion],
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        let game = self.instances.game_dir(id);
        let inst_dir = self.instance_dir(id);
        let mut idx = ContentIndex::load(&inst_dir).await?;
        let disk = self.scan(&game).await?;

        struct Job {
            dest: PathBuf,
            new_key: String,
            old_key: Option<String>,
            disabled: bool,
            sha512: Option<String>,
            entry: IndexEntry,
        }
        let mut reqs = Vec::new();
        let mut jobs = Vec::new();
        for v in versions {
            sink.check_cancelled()?;
            if v.manual_download {
                sink.log(&format!("manual download required: {}", v.file_name));
                continue;
            }
            let cat = self.catalog(v.source)?;
            let info = cat.project(&v.project_id).await?;
            let dir = kind_dir(info.kind)?;
            check_file_name(&v.file_name)?;
            let rf = cat.file(v).await?;
            let Some(url) = rf.url.clone() else {
                sink.log(&format!(
                    "manual download required: {} ({})",
                    v.file_name, rf.manual_url
                ));
                continue;
            };
            let new_key = key(dir, &v.file_name);
            let old_key = idx.find_project(v.source, &v.project_id);
            let old_disk = old_key
                .as_ref()
                .and_then(|k| disk.iter().find(|f| &f.key() == k));
            let disabled = old_disk.map(|f| !f.enabled).unwrap_or(false);
            let dest = safe_join(&game, &format!("{dir}/{}", v.file_name))?;
            let sha1 = rf.sha1.clone().or_else(|| v.sha1.clone());
            if let Some(h) = &sha1 {
                // Fetched while resolving: the downloader re-verifies and skips it.
                let cached = self.jar_cache(h);
                if tokio::fs::try_exists(&cached).await.unwrap_or(false)
                    && !tokio::fs::try_exists(&dest).await.unwrap_or(false)
                {
                    if let Some(parent) = dest.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    let _ = tokio::fs::copy(&cached, &dest).await;
                }
            }
            reqs.push(DownloadReq {
                urls: vec![parse_url(&url)?],
                sha1: sha1.clone(),
                size: (v.size > 0).then_some(v.size),
                dest: dest.clone(),
                kind: ResourceKind::Content,
            });
            jobs.push(Job {
                dest,
                old_key,
                disabled,
                sha512: rf.sha512.clone(),
                entry: IndexEntry {
                    kind: info.kind,
                    source: v.source,
                    project_id: info.id.clone(),
                    version_id: v.version_id.clone(),
                    title: Some(info.title.clone()),
                    version_number: Some(v.version_number.clone()),
                    sha1,
                    sha512: rf.sha512.clone(),
                    size: Some(v.size),
                    url: Some(url),
                },
                new_key,
            });
        }
        if reqs.is_empty() {
            return Ok(());
        }
        self.dl.fetch_many(reqs, sink).await?;

        for job in jobs {
            let (sha1, sha512) = file_hashes(&job.dest).await?;
            if let Some(want) = &job.sha512 {
                if !want.eq_ignore_ascii_case(&sha512) {
                    let _ = tokio::fs::remove_file(&job.dest).await;
                    return Err(AppError::new(ErrorCode::HashMismatch, "sha512 mismatch")
                        .detail("path", job.dest.to_string_lossy())
                        .detail("expected", want.as_str())
                        .detail("actual", sha512));
                }
            }
            if let Some(old) = job.old_key.filter(|k| *k != job.new_key) {
                if let Some(f) = disk.iter().find(|f| f.key() == old) {
                    let p = game.join(f.dir).join(&f.name);
                    let _ = tokio::fs::remove_file(&p).await;
                }
                idx.files.remove(&old);
            }
            let mut disabled_path = job.dest.as_os_str().to_owned();
            disabled_path.push(".disabled");
            let disabled_path = PathBuf::from(disabled_path);
            if job.disabled {
                let _ = tokio::fs::remove_file(&disabled_path).await;
                tokio::fs::rename(&job.dest, &disabled_path)
                    .await
                    .map_err(|e| io_err(&disabled_path, e))?;
            } else if tokio::fs::try_exists(&disabled_path).await.unwrap_or(false) {
                // A stale disabled copy of the same file would shadow nothing; drop it.
                let _ = tokio::fs::remove_file(&disabled_path).await;
            }
            let mut entry = job.entry;
            entry.sha1 = Some(sha1);
            entry.sha512 = Some(sha512);
            idx.files.insert(job.new_key, entry);
        }
        idx.save(&inst_dir).await
    }

    /// Available updates per source, keyed by project id.
    pub async fn updates(
        &self,
        inst: &Instance,
        idx: &ContentIndex,
        fresh: bool,
    ) -> AppResult<HashMap<(ContentSourceKind, String), ContentVersion>> {
        let target = target_of(inst);
        let mut out = HashMap::new();
        for source in [ContentSourceKind::Modrinth, ContentSourceKind::CurseForge] {
            let items: Vec<UpdateItem> = idx
                .files
                .values()
                .filter(|e| e.source == source)
                .map(|e| UpdateItem {
                    project_id: e.project_id.clone(),
                    version_id: e.version_id.clone(),
                    sha1: e.sha1.clone(),
                    kind: e.kind,
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            let Ok(cat) = self.catalog(source) else {
                continue;
            };
            for (p, v) in cat.updates(&items, &target, fresh).await? {
                out.insert((source, p), v);
            }
        }
        Ok(out)
    }

    pub async fn resolve_for(
        &self,
        inst: &Instance,
        picks: Vec<ContentVersionRef>,
    ) -> AppResult<ResolvePlan> {
        let target = target_of(inst);
        let game = self.instances.game_dir(inst.id);
        let idx = ContentIndex::load(&self.instance_dir(inst.id)).await?;
        let installed: Vec<InstalledRef> = idx
            .files
            .values()
            .map(|e| InstalledRef {
                source: e.source,
                project_id: e.project_id.clone(),
                version_id: Some(e.version_id.clone()),
            })
            .collect();

        // Shader packs pull in a shader loader.
        let mut wants_shader = false;
        for p in &picks {
            let cat = self.catalog(p.source)?;
            if cat.project(&p.project_id).await?.kind == ContentKind::Shader {
                wants_shader = true;
                break;
            }
        }
        let mut extra = Vec::new();
        let mut manual = Vec::new();
        if wants_shader {
            let names: Vec<String> = self
                .scan(&game)
                .await?
                .into_iter()
                .filter(|f| f.kind == ContentKind::Mod)
                .map(|f| f.name)
                .collect();
            if !has_shader_loader(&names) {
                let (e, m) = self.shader_loader(&target).await;
                extra = e;
                manual = m;
            }
        }
        let disk = self.disk_metas(inst, &game, &idx).await?;
        let mut plan = resolve(self, &target, &installed, &disk, picks, extra).await?;
        plan.manual.extend(manual);
        Ok(plan)
    }

    /// Modrinth refs of the shader loader (and its renderer on Forge), or an
    /// OptiFine manual download when no compatible loader exists.
    pub async fn shader_loader(
        &self,
        target: &Target,
    ) -> (
        Vec<ContentVersionRef>,
        Vec<lc_types::content::ManualDownload>,
    ) {
        let mut extra = Vec::new();
        let groups = shader_loader_groups(target.loader);
        if groups.is_empty() {
            return (extra, vec![optifine_manual()]);
        }
        for (gi, group) in groups.iter().enumerate() {
            let mut found = None;
            for slug in group {
                if let Ok(Some(v)) = self.modrinth.latest(slug, target).await {
                    found = Some(v);
                    break;
                }
            }
            match found {
                Some(v) => extra.push(ContentVersionRef {
                    source: ContentSourceKind::Modrinth,
                    project_id: v.project_id,
                    version_id: Some(v.version_id),
                }),
                None if gi == 0 => return (Vec::new(), vec![optifine_manual()]),
                None => {}
            }
        }
        (extra, Vec::new())
    }
}

#[async_trait]
impl Lookup for Inner {
    async fn version(&self, source: ContentSourceKind, id: &str) -> AppResult<ContentVersion> {
        self.catalog(source)?.version(id).await
    }

    async fn latest(
        &self,
        source: ContentSourceKind,
        project_id: &str,
        target: &Target,
    ) -> AppResult<Option<ContentVersion>> {
        self.catalog(source)?.latest(project_id, target).await
    }

    async fn manual_url(&self, v: &ContentVersion) -> AppResult<String> {
        Ok(self.catalog(v.source)?.file(v).await?.manual_url)
    }

    async fn candidates(
        &self,
        source: ContentSourceKind,
        project_id: &str,
        target: &Target,
    ) -> AppResult<Vec<ContentVersion>> {
        let cat = self.catalog(source)?;
        let kind = cat.project(project_id).await?.kind;
        let mut list: Vec<(ContentVersion, bool)> = cat
            .versions_ranked(project_id, &target.filter(kind))
            .await?
            .into_iter()
            .filter(|(v, _)| {
                kind != ContentKind::Mod || loader_compatible(&v.loaders, target.loader)
            })
            .collect();
        // Releases first, otherwise newest first (stable).
        list.sort_by_key(|(_, release)| !*release);
        Ok(list.into_iter().map(|(v, _)| v).collect())
    }

    async fn mod_meta(&self, v: &ContentVersion) -> Option<ModMeta> {
        if v.manual_download || v.size > MAX_META_JAR || !v.file_name.ends_with(".jar") {
            return None;
        }
        let cat = self.catalog(v.source).ok()?;
        let rf = cat.file(v).await.ok()?;
        // Only hash-verified downloads.
        let sha1 = rf.sha1.clone().or_else(|| v.sha1.clone())?;
        let url = parse_url(rf.url.as_deref()?).ok()?;
        let dest = self.jar_cache(&sha1);
        let req = DownloadReq {
            urls: vec![url],
            sha1: Some(sha1),
            size: (v.size > 0).then_some(v.size),
            dest: dest.clone(),
            kind: ResourceKind::Content,
        };
        self.dl
            .fetch_many(vec![req], &lc_runtime::NullSink::default())
            .await
            .ok()?;
        read_jar_meta_async(dest).await
    }
}

#[async_trait]
impl ContentService for ContentServices {
    async fn search(&self, query: SearchQuery) -> AppResult<SearchPage> {
        self.inner.catalog(query.source)?.search(&query).await
    }

    async fn versions(
        &self,
        source: ContentSourceKind,
        project_id: &str,
        filter: ContentFilter,
    ) -> AppResult<Vec<ContentVersion>> {
        self.inner
            .catalog(source)?
            .versions(project_id, &filter)
            .await
    }

    async fn resolve(
        &self,
        instance: InstanceId,
        picks: Vec<ContentVersionRef>,
    ) -> AppResult<ResolvePlan> {
        let inst = self.inner.instances.get(instance).await?;
        self.inner.resolve_for(&inst, picks).await
    }

    async fn install(
        &self,
        instance: InstanceId,
        plan: ResolvePlan,
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        self.inner.instances.get(instance).await?;
        for m in &plan.manual {
            sink.log(&format!("manual download: {} → {}", m.file_name, m.url));
        }
        self.inner
            .install_versions(instance, &plan.install, sink)
            .await
    }

    async fn list(&self, instance: InstanceId) -> AppResult<Vec<InstalledContent>> {
        let inst = self.inner.instances.get(instance).await?;
        let game = self.inner.instances.game_dir(instance);
        let idx = ContentIndex::load(&self.inner.instance_dir(instance)).await?;
        let files = self.inner.scan(&game).await?;
        // Update check is best effort (offline → no badges).
        let updates = self
            .inner
            .updates(&inst, &idx, false)
            .await
            .unwrap_or_default();
        Ok(files
            .into_iter()
            .map(|f| {
                // Local entries only carry hashes; they have no catalogue ids.
                let e = idx
                    .files
                    .get(&f.key())
                    .filter(|e| e.source != ContentSourceKind::Local);
                InstalledContent {
                    file_name: f.base().to_owned(),
                    kind: f.kind,
                    enabled: f.enabled,
                    source: e.map(|e| e.source).unwrap_or(ContentSourceKind::Local),
                    project_id: e.map(|e| e.project_id.clone()),
                    version_id: e.map(|e| e.version_id.clone()),
                    title: e.and_then(|e| e.title.clone()),
                    version_number: e.and_then(|e| e.version_number.clone()),
                    update_available: e
                        .map(|e| updates.contains_key(&(e.source, e.project_id.clone())))
                        .unwrap_or(false),
                }
            })
            .collect())
    }

    async fn set_enabled(
        &self,
        instance: InstanceId,
        file_name: &str,
        enabled: bool,
    ) -> AppResult<()> {
        let game = self.inner.instances.game_dir(instance);
        let f = self.inner.find_file(&game, file_name).await?;
        if f.enabled == enabled {
            return Ok(());
        }
        let dir = game.join(f.dir);
        let from = safe_join(&dir, &f.name)?;
        let to = if enabled {
            dir.join(f.base())
        } else {
            dir.join(format!("{}.disabled", f.base()))
        };
        ensure_no_links(&to)?;
        tokio::fs::rename(&from, &to)
            .await
            .map_err(|e| io_err(&from, e))
    }

    async fn remove(&self, instance: InstanceId, file_name: &str) -> AppResult<()> {
        let game = self.inner.instances.game_dir(instance);
        let f = self.inner.find_file(&game, file_name).await?;
        let p = safe_join(&game, &format!("{}/{}", f.dir, f.name))?;
        if f.is_dir {
            tokio::fs::remove_dir_all(&p).await
        } else {
            tokio::fs::remove_file(&p).await
        }
        .map_err(|e| io_err(&p, e))?;
        let inst_dir = self.inner.instance_dir(instance);
        let mut idx = ContentIndex::load(&inst_dir).await?;
        if idx.files.remove(&f.key()).is_some() {
            idx.save(&inst_dir).await?;
        }
        Ok(())
    }

    async fn add_local(
        &self,
        instance: InstanceId,
        kind: ContentKind,
        paths: Vec<String>,
    ) -> AppResult<Vec<InstalledContent>> {
        self.inner.add_local(instance, kind, paths).await
    }

    async fn check_local(
        &self,
        instance: InstanceId,
        file_names: Vec<String>,
    ) -> AppResult<Vec<LocalDepIssue>> {
        self.inner.check_local(instance, file_names).await
    }

    async fn update_all(&self, instance: InstanceId, sink: &dyn ProgressSink) -> AppResult<()> {
        let inst = self.inner.instances.get(instance).await?;
        let idx = ContentIndex::load(&self.inner.instance_dir(instance)).await?;
        sink.progress(0, None, "stages.contentCheckUpdates");
        let updates = self.inner.updates(&inst, &idx, true).await?;
        let versions: Vec<ContentVersion> = updates.into_values().collect();
        sink.log(&format!("{} update(s) found", versions.len()));
        self.inner.install_versions(instance, &versions, sink).await
    }
}
