//! HTTP layer: the only place that talks to the network over HTTP.
//! Owner: net. `contract.rs` is owned by the lead.

mod config;
pub mod contract;
mod downloader;
mod errors;
pub mod mirrors;
mod progress;
mod route;
mod stub;
mod sysproxy;

pub use config::{default_user_agent, DownloaderConfig, SystemProxyMode};
pub use contract::{DownloadReq, Downloader, ResourceKind, Route};
pub use downloader::HttpDownloader;
pub use stub::StubDownloader;
