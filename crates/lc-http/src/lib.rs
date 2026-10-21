//! HTTP layer: the only place that talks to the network over HTTP.
//! Owner: net. `contract.rs` is owned by the lead.

mod config;
pub mod contract;
mod downloader;
mod errors;
pub mod mirrors;
mod progress;
mod stub;

pub use config::{default_user_agent, DownloaderConfig};
pub use contract::{DownloadReq, Downloader, ResourceKind, Route};
pub use downloader::HttpDownloader;
pub use stub::StubDownloader;
