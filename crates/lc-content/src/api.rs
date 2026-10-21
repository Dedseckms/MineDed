//! HTTP access for catalogue APIs (through `Downloader` only) + on-disk TTL cache
//! under `Paths::cache()/content`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use lc_http::{Downloader, ResourceKind, Route};
use lc_types::{AppError, AppResult, ErrorCode};
use serde::de::DeserializeOwned;

use crate::util::{parse_url, sha1_hex};

pub(crate) const TTL_SEARCH: Duration = Duration::from_secs(10 * 60);
pub(crate) const TTL_VERSIONS: Duration = Duration::from_secs(30 * 60);
pub(crate) const TTL_META: Duration = Duration::from_secs(6 * 60 * 60);
/// Immutable objects (a specific version/file id).
pub(crate) const TTL_IMMUTABLE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Raw transport. Production: `DlHttp`; tests: fixtures.
#[async_trait]
pub(crate) trait Http: Send + Sync {
    async fn get(&self, url: &str, headers: &[(&'static str, String)]) -> AppResult<Vec<u8>>;
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&'static str, String)],
        body: &serde_json::Value,
    ) -> AppResult<Vec<u8>>;
}

/// `Downloader`-backed transport. Header-less GETs go through `get_bytes`
/// (retries, route policy); requests with headers / POST use `client(Route::Auto)`.
pub(crate) struct DlHttp(pub Arc<dyn Downloader>);

fn status_err(status: reqwest::StatusCode, url: &str) -> AppError {
    let code = match status.as_u16() {
        404 => ErrorCode::NotFound,
        401 => ErrorCode::AuthRequired,
        403 => ErrorCode::PermissionDenied,
        400 | 422 => ErrorCode::InvalidInput,
        _ => ErrorCode::Network,
    };
    // Never put request headers (API key) into the error.
    AppError::new(code, format!("http status {status}")).detail("url", url)
}

async fn send(req: reqwest::RequestBuilder, url: &str) -> AppResult<Vec<u8>> {
    let resp = req
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| {
            AppError::new(
                ErrorCode::Network,
                format!("request failed: {}", e.without_url()),
            )
            .detail("url", url)
        })?;
    if !resp.status().is_success() {
        return Err(status_err(resp.status(), url));
    }
    let bytes = resp.bytes().await.map_err(|e| {
        AppError::new(
            ErrorCode::Network,
            format!("read failed: {}", e.without_url()),
        )
        .detail("url", url)
    })?;
    Ok(bytes.to_vec())
}

#[async_trait]
impl Http for DlHttp {
    async fn get(&self, url: &str, headers: &[(&'static str, String)]) -> AppResult<Vec<u8>> {
        if headers.is_empty() {
            return self
                .0
                .get_bytes(&parse_url(url)?, ResourceKind::Other)
                .await;
        }
        let mut req = self.0.client(Route::Auto).get(parse_url(url)?);
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        send(req, url).await
    }

    async fn post_json(
        &self,
        url: &str,
        headers: &[(&'static str, String)],
        body: &serde_json::Value,
    ) -> AppResult<Vec<u8>> {
        let mut req = self.0.client(Route::Auto).post(parse_url(url)?).json(body);
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        send(req, url).await
    }
}

/// JSON API client with a file cache (`<cache>/content/<sha1(key)>.json`).
/// Stale entries are used as a fallback when the network fails.
pub(crate) struct Api {
    http: Arc<dyn Http>,
    dir: PathBuf,
}

impl Api {
    pub fn new(http: Arc<dyn Http>, cache_dir: PathBuf) -> Self {
        Self {
            http,
            dir: cache_dir,
        }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.json", sha1_hex(key.as_bytes())))
    }

    async fn cached(&self, key: &str, ttl: Duration) -> Option<Vec<u8>> {
        let p = self.path(key);
        let meta = tokio::fs::metadata(&p).await.ok()?;
        let age = SystemTime::now()
            .duration_since(meta.modified().ok()?)
            .unwrap_or(Duration::MAX);
        if age > ttl {
            return None;
        }
        tokio::fs::read(&p).await.ok()
    }

    async fn store(&self, key: &str, bytes: &[u8]) {
        if tokio::fs::create_dir_all(&self.dir).await.is_ok() {
            let p = self.path(key);
            let tmp = p.with_extension("tmp");
            if tokio::fs::write(&tmp, bytes).await.is_ok() {
                let _ = tokio::fs::rename(&tmp, &p).await;
            }
        }
    }

    async fn through_cache<F, Fut>(&self, key: &str, ttl: Duration, fetch: F) -> AppResult<Vec<u8>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = AppResult<Vec<u8>>>,
    {
        if !ttl.is_zero() {
            if let Some(b) = self.cached(key, ttl).await {
                return Ok(b);
            }
        }
        match fetch().await {
            Ok(b) => {
                self.store(key, &b).await;
                Ok(b)
            }
            Err(e) if e.code != ErrorCode::NotFound => {
                match tokio::fs::read(self.path(key)).await {
                    Ok(b) => Ok(b),
                    Err(_) => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    /// `ttl = 0` always refetches (still falls back to the stale copy offline).
    pub async fn get<T: DeserializeOwned>(
        &self,
        url: &str,
        headers: &[(&'static str, String)],
        ttl: Duration,
    ) -> AppResult<T> {
        let bytes = self
            .through_cache(url, ttl, || self.http.get(url, headers))
            .await?;
        decode(&bytes, url)
    }

    pub async fn post<T: DeserializeOwned>(
        &self,
        url: &str,
        headers: &[(&'static str, String)],
        body: &serde_json::Value,
        ttl: Duration,
    ) -> AppResult<T> {
        let key = format!("POST {url} {body}");
        let bytes = self
            .through_cache(&key, ttl, || self.http.post_json(url, headers, body))
            .await?;
        decode(&bytes, url)
    }
}

fn decode<T: DeserializeOwned>(bytes: &[u8], url: &str) -> AppResult<T> {
    serde_json::from_slice(bytes).map_err(|e| {
        // An HTML block page instead of JSON looks like this too.
        AppError::new(ErrorCode::Network, format!("unexpected API response: {e}"))
            .detail("url", url)
    })
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// URL (or `POST <url> <body>`) → response bytes.
    #[derive(Default)]
    pub struct FakeHttp {
        pub routes: Mutex<HashMap<String, Vec<u8>>>,
        pub calls: Mutex<Vec<String>>,
    }

    impl FakeHttp {
        pub fn route(&self, key: &str, body: impl Into<Vec<u8>>) {
            self.routes
                .lock()
                .unwrap()
                .insert(key.to_owned(), body.into());
        }

        fn answer(&self, key: &str) -> AppResult<Vec<u8>> {
            self.calls.lock().unwrap().push(key.to_owned());
            let routes = self.routes.lock().unwrap();
            // Exact match first, then the URL without its query string.
            routes
                .get(key)
                .or_else(|| key.split_once('?').and_then(|(k, _)| routes.get(k)))
                .cloned()
                .ok_or_else(|| AppError::new(ErrorCode::NotFound, "no fixture").detail("url", key))
        }
    }

    #[async_trait]
    impl Http for FakeHttp {
        async fn get(&self, url: &str, _h: &[(&'static str, String)]) -> AppResult<Vec<u8>> {
            self.answer(url)
        }

        async fn post_json(
            &self,
            url: &str,
            _h: &[(&'static str, String)],
            body: &serde_json::Value,
        ) -> AppResult<Vec<u8>> {
            self.answer(&format!("POST {url} {body}"))
                .or_else(|_| self.answer(&format!("POST {url}")))
        }
    }

    #[tokio::test]
    async fn cache_hits_and_stale_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(FakeHttp::default());
        http.route("https://x/a", r#"{"v":1}"#);
        let api = Api::new(http.clone(), dir.path().to_path_buf());
        let v: serde_json::Value = api.get("https://x/a", &[], TTL_META).await.unwrap();
        assert_eq!(v["v"], 1);
        let _: serde_json::Value = api.get("https://x/a", &[], TTL_META).await.unwrap();
        assert_eq!(http.calls.lock().unwrap().len(), 1);
        // Network failure (not 404) → stale copy.
        http.routes.lock().unwrap().clear();
        let failing = Arc::new(Failing);
        let api2 = Api::new(failing, dir.path().to_path_buf());
        let v: serde_json::Value = api2.get("https://x/a", &[], Duration::ZERO).await.unwrap();
        assert_eq!(v["v"], 1);
    }

    struct Failing;

    #[async_trait]
    impl Http for Failing {
        async fn get(&self, _u: &str, _h: &[(&'static str, String)]) -> AppResult<Vec<u8>> {
            Err(AppError::new(ErrorCode::Network, "down"))
        }
        async fn post_json(
            &self,
            _u: &str,
            _h: &[(&'static str, String)],
            _b: &serde_json::Value,
        ) -> AppResult<Vec<u8>> {
            Err(AppError::new(ErrorCode::Network, "down"))
        }
    }
}
