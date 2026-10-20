use std::path::PathBuf;

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::instance::{Instance, InstancePatch, InstanceSpec, LaunchOpts, RunState};
use lc_types::java::JavaRuntime;
use lc_types::system::{MemoryRecommendation, SystemInfo};
use lc_types::version::{VersionEntry, VersionFilter};
use lc_types::{AppError, AppResult, InstanceId};

use crate::contract::{GameLauncher, InstanceService, JavaManager, SystemProbe, VersionService};

/// Placeholder for all core services until core implements them.
#[derive(Debug, Default)]
pub struct StubCore;

#[async_trait]
impl VersionService for StubCore {
    async fn list(&self, _filter: VersionFilter) -> AppResult<Vec<VersionEntry>> {
        Err(AppError::not_implemented("VersionService::list"))
    }

    async fn java_major(&self, _mc: &str) -> AppResult<u32> {
        Err(AppError::not_implemented("VersionService::java_major"))
    }
}

#[async_trait]
impl JavaManager for StubCore {
    async fn list(&self) -> AppResult<Vec<JavaRuntime>> {
        Ok(Vec::new())
    }

    async fn detect(&self) -> AppResult<Vec<JavaRuntime>> {
        Err(AppError::not_implemented("JavaManager::detect"))
    }

    async fn install(&self, _major: u32, _sink: &dyn ProgressSink) -> AppResult<JavaRuntime> {
        Err(AppError::not_implemented("JavaManager::install"))
    }

    async fn ensure(&self, _major: u32, _sink: &dyn ProgressSink) -> AppResult<JavaRuntime> {
        Err(AppError::not_implemented("JavaManager::ensure"))
    }
}

#[async_trait]
impl InstanceService for StubCore {
    async fn list(&self) -> AppResult<Vec<Instance>> {
        Ok(Vec::new())
    }

    async fn get(&self, _id: InstanceId) -> AppResult<Instance> {
        Err(AppError::not_implemented("InstanceService::get"))
    }

    async fn create(&self, _spec: InstanceSpec, _sink: &dyn ProgressSink) -> AppResult<Instance> {
        Err(AppError::not_implemented("InstanceService::create"))
    }

    async fn clone_instance(
        &self,
        _id: InstanceId,
        _name: &str,
        _sink: &dyn ProgressSink,
    ) -> AppResult<Instance> {
        Err(AppError::not_implemented("InstanceService::clone_instance"))
    }

    async fn update(&self, _id: InstanceId, _patch: InstancePatch) -> AppResult<Instance> {
        Err(AppError::not_implemented("InstanceService::update"))
    }

    async fn delete(&self, _id: InstanceId) -> AppResult<()> {
        Err(AppError::not_implemented("InstanceService::delete"))
    }

    async fn repair(&self, _id: InstanceId, _sink: &dyn ProgressSink) -> AppResult<()> {
        Err(AppError::not_implemented("InstanceService::repair"))
    }

    fn game_dir(&self, id: InstanceId) -> PathBuf {
        PathBuf::from("instances").join(id.to_string())
    }
}

#[async_trait]
impl GameLauncher for StubCore {
    async fn launch(
        &self,
        _id: InstanceId,
        _opts: LaunchOpts,
        _sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        Err(AppError::not_implemented("GameLauncher::launch"))
    }

    async fn kill(&self, _id: InstanceId) -> AppResult<()> {
        Err(AppError::not_implemented("GameLauncher::kill"))
    }

    fn state(&self, _id: InstanceId) -> RunState {
        RunState::Stopped
    }
}

#[async_trait]
impl SystemProbe for StubCore {
    async fn info(&self) -> AppResult<SystemInfo> {
        Err(AppError::not_implemented("SystemProbe::info"))
    }

    async fn recommend_memory(&self, _id: InstanceId) -> AppResult<MemoryRecommendation> {
        Err(AppError::not_implemented("SystemProbe::recommend_memory"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_runtime::NullSink;
    use lc_types::ErrorCode;

    #[tokio::test]
    async fn stub_launch_not_implemented() {
        let err = StubCore
            .launch(
                InstanceId::new(),
                LaunchOpts::default(),
                &NullSink::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert_eq!(StubCore.state(InstanceId::new()), RunState::Stopped);
    }
}
