//! Content (mods, packs, shaders) and modpacks. Owner: mods.
//! `contract.rs` is owned by the lead.

mod api;
pub mod contract;
mod curseforge;
mod index;
mod local;
mod localdeps;
mod modmeta;
mod modrinth;
mod packs;
mod presets;
mod resolver;
mod service;
mod shaders;
mod source;
mod stub;
#[cfg(test)]
mod tests;
mod util;

pub use contract::{ContentService, ContentSource, PackIo, PresetService};
pub use curseforge::CurseForgeSource;
pub use modrinth::ModrinthSource;
pub use service::ContentServices;
pub use stub::StubContent;
