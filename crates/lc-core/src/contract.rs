//! CONTRACT (lead): services exposed to `src-tauri` and other crates.

use std::path::PathBuf;

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::events::{InstanceLogPayload, InstanceStatePayload};
use lc_types::instance::{Instance, InstancePatch, InstanceSpec, LaunchOpts, RunState};
use lc_types::java::JavaRuntime;
use lc_types::system::{MemoryRecommendation, SystemInfo};
use lc_types::version::{VersionEntry, VersionFilter};
use lc_types::{AppResult, InstanceId};

#[async_trait]
pub trait VersionService: Send + Sync {
    /// Mojang version manifest (cached, refreshed in background).
    async fn list(&self, filter: VersionFilter) -> AppResult<Vec<VersionEntry>>;
    /// `javaVersion.majorVersion` of the version json.
    async fn java_major(&self, mc: &str) -> AppResult<u32>;
}

#[async_trait]
pub trait JavaManager: Send + Sync {
    /// Managed + detected runtimes.
    async fn list(&self) -> AppResult<Vec<JavaRuntime>>;
    /// Scans PATH, JAVA_HOME, registry (Windows), /usr/lib/jvm (Linux).
    async fn detect(&self) -> AppResult<Vec<JavaRuntime>>;
    /// Mojang runtime manifest first, Adoptium as fallback.
    async fn install(&self, major: u32, sink: &dyn ProgressSink) -> AppResult<JavaRuntime>;
    /// Best runtime for a major, installing it when missing.
    async fn ensure(&self, major: u32, sink: &dyn ProgressSink) -> AppResult<JavaRuntime>;
}

#[async_trait]
pub trait InstanceService: Send + Sync {
    async fn list(&self) -> AppResult<Vec<Instance>>;
    async fn get(&self, id: InstanceId) -> AppResult<Instance>;
    /// Downloads vanilla files, installs the loader via `LoaderRegistry`.
    async fn create(&self, spec: InstanceSpec, sink: &dyn ProgressSink) -> AppResult<Instance>;
    async fn clone_instance(
        &self,
        id: InstanceId,
        name: &str,
        sink: &dyn ProgressSink,
    ) -> AppResult<Instance>;
    async fn update(&self, id: InstanceId, patch: InstancePatch) -> AppResult<Instance>;
    async fn delete(&self, id: InstanceId) -> AppResult<()>;
    /// Re-verifies all files and reinstalls the loader.
    async fn repair(&self, id: InstanceId, sink: &dyn ProgressSink) -> AppResult<()>;
    /// Game directory of the instance (for "open folder", mods, packs).
    fn game_dir(&self, id: InstanceId) -> PathBuf;
}

/// Receives game process events; implemented in `src-tauri` (emits Tauri events).
pub trait GameObserver: Send + Sync {
    fn on_state(&self, payload: InstanceStatePayload);
    fn on_log(&self, payload: InstanceLogPayload);
}

#[async_trait]
pub trait GameLauncher: Send + Sync {
    /// Prepares files, asks `AuthProvider` for credentials, builds `LaunchPlan`,
    /// applies `LaunchContributor`s and spawns the JVM. Returns once the process
    /// is running; further state goes to `GameObserver`.
    async fn launch(
        &self,
        id: InstanceId,
        opts: LaunchOpts,
        sink: &dyn ProgressSink,
    ) -> AppResult<()>;
    async fn kill(&self, id: InstanceId) -> AppResult<()>;
    fn state(&self, id: InstanceId) -> RunState;
}

#[async_trait]
pub trait SystemProbe: Send + Sync {
    async fn info(&self) -> AppResult<SystemInfo>;
    /// Uses system RAM, mod count and preset.
    async fn recommend_memory(&self, id: InstanceId) -> AppResult<MemoryRecommendation>;
}
