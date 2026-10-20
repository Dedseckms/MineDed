//! CONTRACT (lead): consumed by core when creating/repairing instances.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use lc_http::Downloader;
use lc_runtime::{Paths, ProgressSink};
use lc_types::version::{LoaderKind, LoaderVersion};
use lc_types::{AppError, AppResult};

/// Id of a version json under `Paths::versions()`, e.g. `fabric-loader-0.16.9-1.21.1`.
/// The json may `inheritsFrom` the vanilla version.
pub type VersionProfileId = String;

pub struct InstallCtx {
    pub paths: Paths,
    pub downloader: Arc<dyn Downloader>,
    /// Java used to run Forge/NeoForge install processors.
    pub java: PathBuf,
}

#[async_trait]
pub trait LoaderInstaller: Send + Sync {
    fn kind(&self) -> LoaderKind;
    /// Loader builds available for a Minecraft version, newest first.
    async fn versions(&self, mc: &str) -> AppResult<Vec<LoaderVersion>>;
    /// Installs libraries + version json; for Forge/NeoForge also runs processors.
    /// Idempotent: re-running repairs a broken install.
    async fn install(
        &self,
        ctx: &InstallCtx,
        mc: &str,
        loader_version: &str,
        sink: &dyn ProgressSink,
    ) -> AppResult<VersionProfileId>;
}

/// Lookup by kind; filled in `src-tauri`.
#[derive(Default)]
pub struct LoaderRegistry {
    installers: Vec<Arc<dyn LoaderInstaller>>,
}

impl LoaderRegistry {
    pub fn register(&mut self, installer: Arc<dyn LoaderInstaller>) {
        self.installers.retain(|i| i.kind() != installer.kind());
        self.installers.push(installer);
    }

    pub fn get(&self, kind: LoaderKind) -> AppResult<Arc<dyn LoaderInstaller>> {
        self.installers
            .iter()
            .find(|i| i.kind() == kind)
            .cloned()
            .ok_or_else(|| AppError::not_implemented(&format!("loader {kind:?}")))
    }
}
