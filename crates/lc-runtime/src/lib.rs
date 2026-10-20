//! Cross-cutting runtime plumbing shared by all backend crates.
//!
//! CONTRACT: owned by the lead.

pub mod launch;
pub mod paths;
pub mod progress;
pub mod settings;

pub use launch::{LaunchContributor, LaunchPlan, QuickPlay};
pub use paths::Paths;
pub use progress::{CancelToken, NullSink, ProgressSink};
pub use settings::{MemorySettingsStore, SettingsStore};
