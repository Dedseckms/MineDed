use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::content::{
    ContentFilter, ContentSourceKind, ContentVersion, ContentVersionRef, InstalledContent,
    ResolvePlan, SearchPage, SearchQuery,
};
use lc_types::instance::{Instance, PackFormat, PerfPreset};
use lc_types::{AppError, AppResult, InstanceId};

use crate::contract::{ContentService, PackIo, PresetService};

/// Placeholder until mods implements content services.
#[derive(Debug, Default)]
pub struct StubContent;

#[async_trait]
impl ContentService for StubContent {
    async fn search(&self, _query: SearchQuery) -> AppResult<SearchPage> {
        Err(AppError::not_implemented("ContentService::search"))
    }

    async fn versions(
        &self,
        _source: ContentSourceKind,
        _project_id: &str,
        _filter: ContentFilter,
    ) -> AppResult<Vec<ContentVersion>> {
        Err(AppError::not_implemented("ContentService::versions"))
    }

    async fn resolve(
        &self,
        _instance: InstanceId,
        _picks: Vec<ContentVersionRef>,
    ) -> AppResult<ResolvePlan> {
        Err(AppError::not_implemented("ContentService::resolve"))
    }

    async fn install(
        &self,
        _instance: InstanceId,
        _plan: ResolvePlan,
        _sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        Err(AppError::not_implemented("ContentService::install"))
    }

    async fn list(&self, _instance: InstanceId) -> AppResult<Vec<InstalledContent>> {
        Ok(Vec::new())
    }

    async fn set_enabled(
        &self,
        _instance: InstanceId,
        _file_name: &str,
        _enabled: bool,
    ) -> AppResult<()> {
        Err(AppError::not_implemented("ContentService::set_enabled"))
    }

    async fn remove(&self, _instance: InstanceId, _file_name: &str) -> AppResult<()> {
        Err(AppError::not_implemented("ContentService::remove"))
    }

    async fn update_all(&self, _instance: InstanceId, _sink: &dyn ProgressSink) -> AppResult<()> {
        Err(AppError::not_implemented("ContentService::update_all"))
    }
}

#[async_trait]
impl PackIo for StubContent {
    async fn import(&self, _path: &str, _sink: &dyn ProgressSink) -> AppResult<Instance> {
        Err(AppError::not_implemented("PackIo::import"))
    }

    async fn export(
        &self,
        _instance: InstanceId,
        _format: PackFormat,
        _path: &str,
        _sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        Err(AppError::not_implemented("PackIo::export"))
    }
}

#[async_trait]
impl PresetService for StubContent {
    async fn apply(
        &self,
        _instance: InstanceId,
        _preset: PerfPreset,
        _sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        Err(AppError::not_implemented("PresetService::apply"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_list_is_empty() {
        assert!(StubContent
            .list(InstanceId::new())
            .await
            .unwrap()
            .is_empty());
    }
}
