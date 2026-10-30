//! CONTRACT (lead): consumed by core, mods, accounts, vpn.

use std::path::PathBuf;

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::settings::ProxyCfg;
use lc_types::AppResult;
use url::Url;

/// How a request reaches the internet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    Direct,
    /// Through the proxy: the built-in VPN if running, else the manual proxy.
    /// Without a proxy requests fail.
    Proxied,
    /// Direct first; on timeout / network error / `Blocked` / hash mismatch /
    /// spoofed response (HTML block page instead of expected data) retry via the
    /// proxy (built-in VPN, else manual proxy, else the OS system proxy). The host is
    /// then remembered as "proxy first" for a while.
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
    /// already exist with the right hash. Atomic: write to `*.part`, then rename;
    /// an existing `*.part` is resumed with an HTTP Range request.
    /// Reports aggregated progress (bytes) to `sink` and honours cancellation.
    async fn fetch_many(&self, reqs: Vec<DownloadReq>, sink: &dyn ProgressSink) -> AppResult<()>;

    /// Small JSON/text GET with mirrors, retries and route policy.
    async fn get_bytes(&self, url: &Url, kind: ResourceKind) -> AppResult<Vec<u8>>;

    /// Preconfigured client (user agent, timeouts, proxy) for API calls.
    fn client(&self, route: Route) -> reqwest::Client;

    /// Client for protocol upgrades (WebSocket). HTTP/2 has no
    /// `Connection`/`Upgrade` headers, so the upgrade handshake needs HTTP/1.1.
    fn upgrade_client(&self, route: Route) -> reqwest::Client {
        self.client(route)
    }

    /// Official URL → ordered candidate list (mirrors + original).
    fn mirrors_for(&self, original: &Url, kind: ResourceKind) -> Vec<Url>;

    /// Applies the manual proxy from settings. It is never used for
    /// `Route::Direct`; it is the fallback route of `Route::Auto` and the route
    /// of `Route::Proxied` (until the built-in VPN provides its own proxy).
    /// Takes effect for requests started after the call.
    fn set_manual_proxy(&self, proxy: Option<ProxyCfg>);

    /// Local proxy of the built-in VPN (mihomo mixed-port), `None` when off.
    /// When set it is the preferred proxy for `Auto` fallback and `Proxied`,
    /// ahead of the manual proxy. Default: ignored (test doubles).
    fn set_vpn_proxy(&self, _proxy: Option<Url>) {}
}
