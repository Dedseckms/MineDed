//! In-memory `Downloader` for tests: URL → bytes, verifies `sha1`.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use lc_http::{DownloadReq, Downloader, ResourceKind, Route};
use lc_runtime::ProgressSink;
use lc_types::settings::ProxyCfg;
use lc_types::{AppError, AppResult, ErrorCode};
use url::Url;

use crate::util::sha1_hex;

#[derive(Default)]
pub(crate) struct MockDl {
    pub files: Mutex<HashMap<String, Vec<u8>>>,
    pub fetched: Mutex<Vec<String>>,
}

impl MockDl {
    pub fn put(&self, url: &str, bytes: impl Into<Vec<u8>>) {
        self.files
            .lock()
            .unwrap()
            .insert(url.to_owned(), bytes.into());
    }

    fn get(&self, url: &str) -> AppResult<Vec<u8>> {
        self.files
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "404").detail("url", url))
    }
}

#[async_trait]
impl Downloader for MockDl {
    async fn fetch_many(&self, reqs: Vec<DownloadReq>, _sink: &dyn ProgressSink) -> AppResult<()> {
        for r in reqs {
            let url = r.urls[0].to_string();
            let bytes = self.get(&url)?;
            if let Some(want) = &r.sha1 {
                if &sha1_hex(&bytes) != want {
                    return Err(AppError::new(ErrorCode::HashMismatch, "mock hash"));
                }
            }
            std::fs::create_dir_all(r.dest.parent().unwrap()).unwrap();
            std::fs::write(&r.dest, bytes).unwrap();
            self.fetched.lock().unwrap().push(url);
        }
        Ok(())
    }

    async fn get_bytes(&self, url: &Url, _kind: ResourceKind) -> AppResult<Vec<u8>> {
        self.get(url.as_str())
    }

    fn client(&self, route: Route) -> reqwest::Client {
        lc_http::StubDownloader.client(route)
    }

    fn mirrors_for(&self, original: &Url, _kind: ResourceKind) -> Vec<Url> {
        vec![original.clone()]
    }

    fn set_manual_proxy(&self, _proxy: Option<ProxyCfg>) {}
}
