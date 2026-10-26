//! CONTRACT (lead).

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::content::{
    ContentFilter, ContentKind, ContentSourceKind, ContentVersion, ContentVersionRef,
    InstalledContent, LocalDepIssue, ResolvePlan, SearchPage, SearchQuery,
};
use lc_types::instance::{Instance, PackFormat, PerfPreset};
use lc_types::{AppResult, InstanceId};

/// One remote catalogue (Modrinth, CurseForge).
#[async_trait]
pub trait ContentSource: Send + Sync {
    fn kind(&self) -> ContentSourceKind;
    async fn search(&self, query: &SearchQuery) -> AppResult<SearchPage>;
    async fn versions(
        &self,
        project_id: &str,
        filter: &ContentFilter,
    ) -> AppResult<Vec<ContentVersion>>;
    async fn version(&self, version_id: &str) -> AppResult<ContentVersion>;
}

/// Content of one instance; dispatches to the right `ContentSource`.
#[async_trait]
pub trait ContentService: Send + Sync {
    async fn search(&self, query: SearchQuery) -> AppResult<SearchPage>;
    async fn versions(
        &self,
        source: ContentSourceKind,
        project_id: &str,
        filter: ContentFilter,
    ) -> AppResult<Vec<ContentVersion>>;
    /// Walks required dependencies, detects incompatibilities and manual downloads.
    async fn resolve(
        &self,
        instance: InstanceId,
        picks: Vec<ContentVersionRef>,
    ) -> AppResult<ResolvePlan>;
    async fn install(
        &self,
        instance: InstanceId,
        plan: ResolvePlan,
        sink: &dyn ProgressSink,
    ) -> AppResult<()>;
    async fn list(&self, instance: InstanceId) -> AppResult<Vec<InstalledContent>>;
    /// Renames `x.jar` ↔ `x.jar.disabled`.
    async fn set_enabled(
        &self,
        instance: InstanceId,
        file_name: &str,
        enabled: bool,
    ) -> AppResult<()>;
    async fn remove(&self, instance: InstanceId, file_name: &str) -> AppResult<()>;
    /// Copies local files into the instance folder for `kind` (mods: `.jar`;
    /// resource packs / shaders: `.zip` or a folder). Existing files with the
    /// same name are replaced. Returns the added entries (source `local`).
    async fn add_local(
        &self,
        instance: InstanceId,
        kind: ContentKind,
        paths: Vec<String>,
    ) -> AppResult<Vec<InstalledContent>>;
    /// Reads the metadata of local mod jars (`file_names`; empty = every local
    /// mod of the instance) and reports unmet or conflicting dependencies and
    /// jars built for another loader / Minecraft version. Optional dependencies
    /// are ignored. Looks up Modrinth suggestions; offline → issues without them.
    async fn check_local(
        &self,
        instance: InstanceId,
        file_names: Vec<String>,
    ) -> AppResult<Vec<LocalDepIssue>>;
    async fn update_all(&self, instance: InstanceId, sink: &dyn ProgressSink) -> AppResult<()>;
}

/// Modpack import/export.
#[async_trait]
pub trait PackIo: Send + Sync {
    /// Detects format by content; creates a new instance.
    async fn import(&self, path: &str, sink: &dyn ProgressSink) -> AppResult<Instance>;
    /// Downloads a modpack version (`version_id`; `None` = latest) from Modrinth
    /// (.mrpack) or CurseForge (zip), verifies it and imports it as a new instance.
    async fn install_remote(
        &self,
        source: ContentSourceKind,
        project_id: &str,
        version_id: Option<&str>,
        sink: &dyn ProgressSink,
    ) -> AppResult<Instance>;
    async fn export(
        &self,
        instance: InstanceId,
        format: PackFormat,
        path: &str,
        sink: &dyn ProgressSink,
    ) -> AppResult<()>;
}

/// Performance presets: optimization mods + JVM flags per loader/version.
#[async_trait]
pub trait PresetService: Send + Sync {
    async fn apply(
        &self,
        instance: InstanceId,
        preset: PerfPreset,
        sink: &dyn ProgressSink,
    ) -> AppResult<()>;
}
