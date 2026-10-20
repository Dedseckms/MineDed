//! Built-in VPN via mihomo sidecar. Owner: net. `contract.rs` is owned by the lead.

pub mod contract;
mod stub;

pub use contract::VpnController;
pub use stub::StubVpn;
