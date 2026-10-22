//! CONTRACT (lead). The implementation also provides a `LaunchContributor`
//! (no-op in `launcherOnly`, JVM proxy properties in `launcherAndGame`) and
//! feeds the proxy URL into `lc-http` (`Downloader::set_vpn_proxy`).
//! mihomo runs as a sidecar without admin rights or TUN, on its own free ports
//! (never the defaults of a Clash/mihomo the user already runs).

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::vpn::{VpnMode, VpnNode, VpnState};
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
}
