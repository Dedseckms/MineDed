use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::version::{LoaderKind, LoaderVersion};
use lc_types::{AppError, AppResult};

use crate::contract::{InstallCtx, LoaderInstaller, VersionProfileId};

/// Placeholder installer for any loader kind.
#[derive(Debug)]
pub struct StubLoader(pub LoaderKind);

#[async_trait]
impl LoaderInstaller for StubLoader {
    fn kind(&self) -> LoaderKind {
        self.0
    }

    async fn versions(&self, _mc: &str) -> AppResult<Vec<LoaderVersion>> {
        Err(AppError::not_implemented("LoaderInstaller::versions"))
    }

    async fn install(
        &self,
        _ctx: &InstallCtx,
        _mc: &str,
        _loader_version: &str,
        _sink: &dyn ProgressSink,
    ) -> AppResult<VersionProfileId> {
        Err(AppError::not_implemented("LoaderInstaller::install"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::contract::LoaderRegistry;

    #[test]
    fn registry_finds_by_kind() {
        let mut r = LoaderRegistry::default();
        r.register(Arc::new(StubLoader(LoaderKind::Fabric)));
        assert!(r.get(LoaderKind::Fabric).is_ok());
        assert!(r.get(LoaderKind::Forge).is_err());
    }
}
