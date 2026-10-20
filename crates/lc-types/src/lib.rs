//! Shared DTOs exchanged between crates and with the frontend.
//!
//! CONTRACT: this crate is owned by the lead. Every type here is exported to
//! TypeScript via specta (`apps/desktop/src/bindings.ts`). Field names are
//! camelCase on the wire. Subagents request changes, they do not edit.

pub mod account;
pub mod content;
pub mod error;
pub mod events;
pub mod ids;
pub mod instance;
pub mod java;
pub mod room;
pub mod settings;
pub mod skin;
pub mod system;
pub mod task;
pub mod version;
pub mod voice;
pub mod vpn;

pub use error::{AppError, AppResult, ErrorCode};
pub use ids::{AccountId, InstanceId, RoomId, SkinId, TaskId};
