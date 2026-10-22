//! Built-in VPN via mihomo sidecar. Owner: net. `contract.rs` is owned by the lead.

mod binary;
mod config;
pub mod contract;
mod controller;
mod launch;
mod ports;
mod service;
mod store;
mod stub;
mod subscription;

pub use binary::MIHOMO_VERSION;
pub use contract::VpnController;
pub use service::MihomoVpn;
pub use stub::StubVpn;
