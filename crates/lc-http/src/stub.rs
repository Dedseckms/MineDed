use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::settings::ProxyCfg;
use lc_types::{AppError, AppResult};
use url::Url;

use crate::contract::{DownloadReq, Downloader, ResourceKind, Route};

/// Placeholder until net implements the real downloader.
#[derive(Debug, Default)]
pub struct StubDownloader;

#[async_trait]
impl Downloader for StubDownloader {
    async fn fetch_many(&self, _reqs: Vec<DownloadReq>, _sink: &dyn ProgressSink) -> AppResult<()> {
        Err(AppError::not_implemented("Downloader::fetch_many"))
    }

    async fn get_bytes(&self, _url: &Url, _kind: ResourceKind) -> AppResult<Vec<u8>> {
        Err(AppError::not_implemented("Downloader::get_bytes"))
    }

    fn client(&self, _route: Route) -> reqwest::Client {
        reqwest::Client::new()
    }

    fn mirrors_for(&self, original: &Url, _kind: ResourceKind) -> Vec<Url> {
        vec![original.clone()]
    }

    fn set_manual_proxy(&self, _proxy: Option<ProxyCfg>) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_runtime::NullSink;
    use lc_types::ErrorCode;

    #[tokio::test]
    async fn stub_reports_not_implemented() {
        let d = StubDownloader;
        let err = d
            .fetch_many(vec![], &NullSink::default())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
    }
}
