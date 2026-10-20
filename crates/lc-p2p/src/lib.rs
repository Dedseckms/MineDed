//! P2P rooms. Owner: net. `contract.rs` and `proto.rs` are owned by the lead.

pub mod contract;
pub mod proto;
mod stub;

pub use contract::{DatagramTransport, RoomObserver, RoomService};
pub use stub::StubRooms;
