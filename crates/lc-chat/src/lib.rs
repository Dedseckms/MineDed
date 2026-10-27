//! Global chat and direct messages client. Contract in `contract.rs` (lead);
//! the implementation is net's.

pub mod contract;
pub mod stub;

pub use contract::{default_privacy, ChatService, SocialObserver};
pub use stub::StubChat;
