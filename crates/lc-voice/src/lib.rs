//! Voice chat. Owner: voice. `contract.rs` is owned by the lead.
//! Transport comes from `lc-p2p::DatagramTransport`; text chat lives in `RoomService`.

pub mod contract;
mod stub;

pub use contract::{VoiceEngine, VoiceObserver};
pub use stub::StubVoice;
