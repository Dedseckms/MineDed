//! Mod loader installers. Owner: mods. `contract.rs` is owned by the lead.

pub mod contract;
mod stub;

pub use contract::{InstallCtx, LoaderInstaller, LoaderRegistry, VersionProfileId};
pub use stub::StubLoader;
