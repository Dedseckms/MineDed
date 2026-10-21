//! Mod loader installers. Owner: mods. `contract.rs` is owned by the lead.

pub mod contract;
mod fabric;
mod forge;
mod libs;
#[cfg(test)]
mod mock;
mod profile;
mod stage;
mod stub;
mod util;

use std::sync::Arc;

use lc_http::Downloader;

pub use contract::{InstallCtx, LoaderInstaller, LoaderRegistry, VersionProfileId};
pub use fabric::FabricLikeInstaller;
pub use forge::ForgeLikeInstaller;
pub use stub::StubLoader;

/// Registry with the real Fabric, Quilt, Forge and NeoForge installers.
/// `dl` serves `versions()`; `install()` uses `InstallCtx::downloader`.
pub fn default_registry(dl: Arc<dyn Downloader>) -> LoaderRegistry {
    let mut r = LoaderRegistry::default();
    r.register(Arc::new(FabricLikeInstaller::fabric(dl.clone())));
    r.register(Arc::new(FabricLikeInstaller::quilt(dl.clone())));
    r.register(Arc::new(ForgeLikeInstaller::forge(dl.clone())));
    r.register(Arc::new(ForgeLikeInstaller::neoforge(dl)));
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_types::version::LoaderKind;

    #[test]
    fn registry_has_all_loaders() {
        let r = default_registry(Arc::new(lc_http::StubDownloader));
        for k in [
            LoaderKind::Fabric,
            LoaderKind::Quilt,
            LoaderKind::Forge,
            LoaderKind::NeoForge,
        ] {
            assert_eq!(r.get(k).unwrap().kind(), k);
        }
        assert!(r.get(LoaderKind::Vanilla).is_err());
    }
}
