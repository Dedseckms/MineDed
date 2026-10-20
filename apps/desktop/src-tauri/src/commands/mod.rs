//! Tauri commands, one file per domain. Owner of each file is noted at its top.
//! Rules: thin adapters only (no business logic); errors are `AppError`;
//! anything that may take >100 ms takes `Channel<TaskEvent>` and returns `TaskId`.

pub mod accounts;
pub mod app;
pub mod content;
pub mod instances;
pub mod java;
pub mod loaders;
pub mod rooms;
pub mod settings;
pub mod skins;
pub mod system;
pub mod tasks;
pub mod versions;
pub mod voice;
pub mod vpn;
