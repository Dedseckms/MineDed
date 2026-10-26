//! CONTRACT (lead). The implementation also provides a `LaunchContributor`
//! (no-op in `launcherOnly`, JVM proxy properties in `launcherAndGame`) and
//! feeds the proxy URL into `lc-http` (`Downloader::set_vpn_proxy`).
//! mihomo runs as a sidecar without admin rights or TUN (except `VpnMode::Apps`), on its own free ports
//! (never the defaults of a Clash/mihomo the user already runs).

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::vpn::{VpnAccess, VpnApp, VpnMode, VpnNode, VpnState};
use lc_types::AppResult;
use url::Url;

#[async_trait]
pub trait VpnController: Send + Sync {
    async fn state(&self) -> AppResult<VpnState>;
    /// Several URLs of the same subscription (mirrors); tried in order.
    async fn set_subscription(&self, urls: Vec<String>) -> AppResult<()>;
    /// Imports a subscription from a local file (mihomo YAML, base64 or plain
    /// list of `vless://…` links). Replaces the URL subscription.
    async fn import_subscription_file(&self, path: &str) -> AppResult<()>;
    async fn refresh(&self, sink: &dyn ProgressSink) -> AppResult<()>;
    async fn set_mode(&self, mode: VpnMode) -> AppResult<VpnState>;
    async fn nodes(&self) -> AppResult<Vec<VpnNode>>;
    async fn select(&self, node: &str) -> AppResult<()>;
    async fn test_latency(&self) -> AppResult<Vec<VpnNode>>;
    /// Local mixed-port proxy (`http://127.0.0.1:<port>`) when running.
    fn proxy_url(&self) -> Option<Url>;
    /// MineDed VPN through hub `/me/vpn`. `bearer` is the Yggdrasil access
    /// token of an own-server account (never logged). `provision` creates the
    /// key when missing; `Ok(None)` when there is none and `provision` is false.
    /// When access is returned, its personal subscription replaces the current
    /// one (the URL itself is never exposed to the UI).
    async fn mineded_access(&self, bearer: &str, provision: bool) -> AppResult<Option<VpnAccess>>;
    /// Programs routed in `VpnMode::Apps` (persisted).
    async fn apps(&self) -> AppResult<Vec<VpnApp>>;
    async fn set_apps(&self, apps: Vec<VpnApp>) -> AppResult<()>;
    /// Currently running user programs (deduplicated by executable), for the picker.
    async fn running_apps(&self) -> AppResult<Vec<VpnApp>>;
}
