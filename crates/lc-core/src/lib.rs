//! Game core. Owner: core. `contract.rs` is owned by the lead.

pub mod contract;
mod stub;

pub use contract::{
    GameLauncher, GameObserver, InstanceService, JavaManager, SystemProbe, VersionService,
};
pub use stub::StubCore;
