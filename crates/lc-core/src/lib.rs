//! Game core. Owner: core. `contract.rs` is owned by the lead.

pub mod contract;
mod instances;
mod java;
mod launch;
mod meta;
mod prepare;
mod services;
mod stub;
mod system;
mod util;
mod versions;

pub use contract::{
    GameLauncher, GameObserver, InstanceService, JavaManager, SystemProbe, VersionService,
};
pub use services::CoreServices;
pub use stub::StubCore;
