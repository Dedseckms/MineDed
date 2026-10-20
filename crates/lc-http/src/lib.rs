//! HTTP layer: the only place that talks to the network over HTTP.
//! Owner: net. `contract.rs` is owned by the lead.

pub mod contract;
mod stub;

pub use contract::{DownloadReq, Downloader, ResourceKind, Route};
pub use stub::StubDownloader;
