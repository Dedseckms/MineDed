use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::vpn::{VpnAccess, VpnApp, VpnMode, VpnNode, VpnState};
use lc_types::{AppError, AppResult};
use url::Url;

use crate::contract::VpnController;

/// Placeholder: VPN is always off.
#[derive(Debug, Default)]
pub struct StubVpn;

#[async_trait]
impl VpnController for StubVpn {
    async fn state(&self) -> AppResult<VpnState> {
        Ok(VpnState {
            mode: VpnMode::Off,
            running: false,
            node: None,
            latency_ms: None,
            subscription_updated: None,
            error: None,
        })
    }

    async fn set_subscription(&self, _urls: Vec<String>) -> AppResult<()> {
        Err(AppError::not_implemented("VpnController::set_subscription"))
    }

    async fn import_subscription_file(&self, _path: &str) -> AppResult<()> {
        Err(AppError::not_implemented(
            "VpnController::import_subscription_file",
        ))
    }

    async fn refresh(&self, _sink: &dyn ProgressSink) -> AppResult<()> {
        Err(AppError::not_implemented("VpnController::refresh"))
    }

    async fn set_mode(&self, _mode: VpnMode) -> AppResult<VpnState> {
        Err(AppError::not_implemented("VpnController::set_mode"))
    }

    async fn nodes(&self) -> AppResult<Vec<VpnNode>> {
        Ok(Vec::new())
    }

    async fn select(&self, _node: &str) -> AppResult<()> {
        Err(AppError::not_implemented("VpnController::select"))
    }

    async fn test_latency(&self) -> AppResult<Vec<VpnNode>> {
        Err(AppError::not_implemented("VpnController::test_latency"))
    }

    fn proxy_url(&self) -> Option<Url> {
        None
    }

    async fn mineded_access(
        &self,
        _bearer: &str,
        _provision: bool,
    ) -> AppResult<Option<VpnAccess>> {
        Err(AppError::not_implemented("VpnController::mineded_access"))
    }

    async fn apps(&self) -> AppResult<Vec<VpnApp>> {
        Ok(Vec::new())
    }

    async fn set_apps(&self, _apps: Vec<VpnApp>) -> AppResult<()> {
        Err(AppError::not_implemented("VpnController::set_apps"))
    }

    async fn running_apps(&self) -> AppResult<Vec<VpnApp>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_is_off() {
        assert_eq!(StubVpn.state().await.unwrap().mode, VpnMode::Off);
        assert!(StubVpn.proxy_url().is_none());
    }
}
