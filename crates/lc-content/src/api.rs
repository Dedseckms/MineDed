//! HTTP access for catalogue APIs (through `Downloader` only) + on-disk TTL cache
//! under `Paths::cache()/content`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

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

/// How long a host stays "proxy first" after the direct route failed.
const PROXY_FIRST_TTL: Duration = Duration::from_secs(10 * 60);

/// `Downloader`-backed transport. Header-less GETs go through `get_bytes`
/// (mirrors, retries, `Route::Auto` fallback inside the downloader). Requests
/// with headers / POST bodies get the same policy here: `client(Route::Auto)`
/// first, and on a network error, 5xx/451 or a non-JSON (block page) answer
/// they are retried through `client(Route::Proxied)`; the host is then tried
/// via the proxy first for a while.
pub(crate) struct DlHttp {
    dl: Arc<dyn Downloader>,
    proxy_first: Mutex<HashMap<String, Instant>>,
}

impl DlHttp {
    pub fn new(dl: Arc<dyn Downloader>) -> Self {
        Self {
            dl,
            proxy_first: Mutex::new(HashMap::new()),
        }
    }

    fn host(url: &str) -> String {
        url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default()
    }

    fn is_proxy_first(&self, host: &str) -> bool {
        let map = self
            .proxy_first
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        map.get(host).is_some_and(|t| t.elapsed() < PROXY_FIRST_TTL)
    }

    fn set_proxy_first(&self, host: &str, on: bool) {
        let mut map = self
            .proxy_first
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if on {
            map.insert(host.to_owned(), Instant::now());
        } else {
            map.remove(host);
        }
    }

    /// Sends `build(client)` over Auto, then Proxied (or the reverse for
    /// remembered hosts). Returns the first error when both fail.
    async fn routed<F>(&self, url: &str, build: F) -> AppResult<Vec<u8>>
    where
        F: Fn(reqwest::Client) -> reqwest::RequestBuilder + Send + Sync,
    {
        let host = Self::host(url);
        let order = if self.is_proxy_first(&host) {
            [Route::Proxied, Route::Auto]
        } else {
            [Route::Auto, Route::Proxied]
        };
        let mut first: Option<AppError> = None;
        for (i, route) in order.into_iter().enumerate() {
            match send(build(self.dl.client(route)), url)
                .await
                .and_then(|b| looks_like_json(b, url))
            {
                Ok(b) => {
                    if i > 0 {
                        self.set_proxy_first(&host, route == Route::Proxied);
                    }
                    return Ok(b);
                }
                Err(e) if e.code == ErrorCode::Network => {
                    first.get_or_insert(e.detail("route", format!("{route:?}")));
                }
                Err(e) => return Err(e),
            }
        }
        Err(first.unwrap_or_else(|| AppError::new(ErrorCode::Network, "request failed")))
    }
}

/// A JSON API never answers with HTML: treat that as a spoofed (blocked) reply.
fn looks_like_json(bytes: Vec<u8>, url: &str) -> AppResult<Vec<u8>> {
    match bytes.iter().find(|b| !b.is_ascii_whitespace()) {
        Some(b'{') | Some(b'[') | None => Ok(bytes),
        Some(_) => {
            Err(AppError::new(ErrorCode::Network, "spoofed response (not JSON)").detail("url", url))
        }
    }
}

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
        let parsed = parse_url(url)?;
        if headers.is_empty() {
            return self.dl.get_bytes(&parsed, ResourceKind::Other).await;
        }
        self.routed(url, |client| {
            let mut req = client.get(parsed.clone());
            for (k, v) in headers {
                req = req.header(*k, v);
            }
            req
        })
        .await
    }

    async fn post_json(
        &self,
        url: &str,
        headers: &[(&'static str, String)],
        body: &serde_json::Value,
    ) -> AppResult<Vec<u8>> {
        let parsed = parse_url(url)?;
        self.routed(url, |client| {
            let mut req = client.post(parsed.clone()).json(body);
            for (k, v) in headers {
                req = req.header(*k, v);
            }
            req
        })
        .await
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

#[cfg(test)]
mod route_tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use lc_http::DownloadReq;
    use lc_runtime::ProgressSink;
    use lc_types::settings::ProxyCfg;
    use url::Url;

    use super::*;

    /// Serves `body` as JSON to every connection (stand-in for the API via the proxy).
    fn serve(body: &'static str) -> (String, Arc<AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                h.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        (format!("http://{addr}"), hits)
    }

    /// A port nothing listens on: the "blackholed" direct route.
    fn dead_proxy() -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        format!("http://{addr}")
    }

    /// Direct (`Auto`) goes through an unreachable proxy; `Proxied` reaches the server.
    struct Dl {
        dead: String,
        gets: AtomicUsize,
        routes: Mutex<Vec<Route>>,
    }

    #[async_trait]
    impl Downloader for Dl {
        async fn fetch_many(&self, _r: Vec<DownloadReq>, _s: &dyn ProgressSink) -> AppResult<()> {
            Ok(())
        }
        async fn get_bytes(&self, _url: &Url, _k: ResourceKind) -> AppResult<Vec<u8>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            Ok(b"{}".to_vec())
        }
        fn client(&self, route: Route) -> reqwest::Client {
            self.routes.lock().unwrap().push(route);
            let b = reqwest::Client::builder().no_proxy();
            match route {
                Route::Proxied => b.build().unwrap(),
                _ => b
                    .proxy(reqwest::Proxy::all(&self.dead).unwrap())
                    .build()
                    .unwrap(),
            }
        }
        fn mirrors_for(&self, original: &Url, _k: ResourceKind) -> Vec<Url> {
            vec![original.clone()]
        }
        fn set_manual_proxy(&self, _p: Option<ProxyCfg>) {}
    }

    fn dl() -> Arc<Dl> {
        Arc::new(Dl {
            dead: dead_proxy(),
            gets: AtomicUsize::new(0),
            routes: Mutex::new(Vec::new()),
        })
    }

    #[tokio::test]
    async fn api_calls_fall_back_to_proxy_when_direct_is_blackholed() {
        let (base, hits) = serve(r#"{"ok":true}"#);
        let d = dl();
        let http = DlHttp::new(d.clone());
        // Header-less GET (Modrinth search/versions): the downloader's Auto policy.
        http.get(&format!("{base}/v2/search"), &[]).await.unwrap();
        assert_eq!(d.gets.load(Ordering::SeqCst), 1);
        assert_eq!(hits.load(Ordering::SeqCst), 0);

        // GET with headers (CurseForge) and POST (Modrinth updates): Auto, then Proxied.
        let h = [("x-api-key", "k".to_owned())];
        let b = http
            .get(&format!("{base}/v1/mods/search"), &h)
            .await
            .unwrap();
        assert_eq!(b, br#"{"ok":true}"#);
        assert_eq!(*d.routes.lock().unwrap(), vec![Route::Auto, Route::Proxied]);
        // Remembered: the next call goes via the proxy first.
        let body = serde_json::json!({"hashes": []});
        http.post_json(&format!("{base}/v2/version_files/update"), &[], &body)
            .await
            .unwrap();
        assert_eq!(d.routes.lock().unwrap()[2], Route::Proxied);
        assert_eq!(d.routes.lock().unwrap().len(), 3);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn block_page_is_retried_and_errors_surface() {
        let (base, _) = serve("<html>blocked</html>");
        let d = dl();
        let http = DlHttp::new(d.clone());
        let h = [("x-api-key", "k".to_owned())];
        let e = http.get(&format!("{base}/v1/x"), &h).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Network);
        // Both routes were tried.
        assert_eq!(*d.routes.lock().unwrap(), vec![Route::Auto, Route::Proxied]);
    }
}
