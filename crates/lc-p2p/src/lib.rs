//! P2P rooms. Owner: net. `contract.rs` and `proto.rs` are owned by the lead.

pub mod code;
pub mod contract;
mod env;
mod hub;
pub mod lan;
mod launch;
mod nbt;
pub mod proto;
mod rooms;
mod servers;
mod stub;
mod wire;

pub use code::{join_link, parse_code};
pub use contract::{DatagramTransport, RoomObserver, RoomService};
pub use env::{mods_hash, CoreRoomEnv, Identity, RoomEnv};
pub use launch::{RoomLaunchContributor, RoomLogTap};
pub use rooms::{reasons, EndpointTweak, P2pConfig, P2pRooms};
pub use stub::StubRooms;
pub use wire::make_peer_token;

/// Re-export so callers (tests, `src-tauri`) can build `EndpointTweak`s
/// against the exact pinned iroh version.
pub use iroh;
