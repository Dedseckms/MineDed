//! Voice chat. Owner: voice. `contract.rs` is owned by the lead.
//! Transport comes from `lc-p2p::DatagramTransport`; text chat lives in `RoomService`.

mod audio;
mod codec;
pub mod contract;
mod dsp;
mod engine;
mod jitter;
mod mixer;
mod ptt;
mod settings;
mod speaking;
mod stub;
mod tx;
mod vad;

#[cfg(test)]
mod loopback_tests;

pub use contract::{VoiceEngine, VoiceObserver};
pub use engine::{CpalVoiceEngine, SELF_PEER_ID};
pub use stub::StubVoice;
