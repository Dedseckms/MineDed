//! CONTRACT (lead): consumed by core, mods, accounts, vpn.

use std::path::PathBuf;

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::AppResult;
use url::Url;

/// How a request reaches the internet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    Direct,
    /// Through mihomo's local mixed-port (VPN mode `launcherOnly` or higher).
    Proxied,
    /// Direct first, switch to proxy for this host after `Blocked`-like errors.
    Auto,
}

/// Used for mirror selection and per-kind concurrency limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    VersionMeta,
    Library,
    Asset,
    ClientJar,
    JavaRuntime,
    Loader,
    Content,
    Skin,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadReq {
    /// Candidates in order of preference. Usually one; `Downloader::mirrors_for`
    /// expands official URLs into mirrors automatically.
    pub urls: Vec<Url>,
    /// Hex SHA-1. When set, mirrors are untrusted and the file is verified.
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub dest: PathBuf,
    pub kind: ResourceKind,
}

#[async_trait]
pub trait Downloader: Send + Sync {
    /// Downloads all requests with bounded concurrency, skipping files that
    /// already exist with the right hash. Atomic: write to `*.part`, then rename.
    /// Reports aggregated progress (bytes) to `sink` and honours cancellation.
    async fn fetch_many(&self, reqs: Vec<DownloadReq>, sink: &dyn ProgressSink) -> AppResult<()>;

    /// Small JSON/text GET with mirrors, retries and route policy.
    async fn get_bytes(&self, url: &Url, kind: ResourceKind) -> AppResult<Vec<u8>>;

    /// Preconfigured client (user agent, timeouts, proxy) for API calls.
    fn client(&self, route: Route) -> reqwest::Client;

    /// Official URL → ordered candidate list (mirrors + original).
    fn mirrors_for(&self, original: &Url, kind: ResourceKind) -> Vec<Url>;
}
