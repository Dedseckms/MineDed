//! Built-in VPN via mihomo sidecar. Owner: net. `contract.rs` is owned by the lead.

mod apps;
mod binary;
mod config;
pub mod contract;
mod controller;
mod launch;
mod mineded;
mod ports;
mod response;
mod service;
mod store;
mod stub;
mod subscription;
mod tun;

pub use binary::MIHOMO_VERSION;
pub use contract::VpnController;
pub use service::MihomoVpn;
pub use stub::StubVpn;
pub use tun::{run_tun_helper_from_args, HELPER_ARG};
