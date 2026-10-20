//! Content (mods, packs, shaders) and modpacks. Owner: mods.
//! `contract.rs` is owned by the lead.

pub mod contract;
mod stub;

pub use contract::{ContentService, ContentSource, PackIo, PresetService};
pub use stub::StubContent;
