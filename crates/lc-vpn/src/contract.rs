//! CONTRACT (lead). The implementation also provides a `LaunchContributor`
//! (no-op in `launcherOnly`, process rules in `launcherAndGame`) and feeds the
//! proxy URL into `lc-http`.

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
    async fn refresh(&self, sink: &dyn ProgressSink) -> AppResult<()>;
    /// `launcherAndGame` may trigger an elevation prompt (UAC / pkexec).
    async fn set_mode(&self, mode: VpnMode) -> AppResult<VpnState>;
    async fn nodes(&self) -> AppResult<Vec<VpnNode>>;
    async fn select(&self, node: &str) -> AppResult<()>;
    async fn test_latency(&self) -> AppResult<Vec<VpnNode>>;
    /// Local mixed-port proxy (`http://127.0.0.1:<port>`) when running.
    fn proxy_url(&self) -> Option<Url>;
}
