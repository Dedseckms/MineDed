//! Real `Downloader`: parallel, resumable, hash-checked, with mirrors.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt};
use lc_runtime::{CancelToken, ProgressSink};
use lc_types::settings::{ProxyCfg, ProxyKind};
use lc_types::{AppError, AppResult, ErrorCode};
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Semaphore;
use url::Url;

use crate::config::{redacted_proxy, DownloaderConfig};
use crate::contract::{DownloadReq, Downloader, ResourceKind, Route};
use crate::errors::{self, AttemptError};
use crate::mirrors;
use crate::progress::Progress;

const MAX_BACKOFF: Duration = Duration::from_secs(8);
const HASH_BUF: usize = 64 * 1024;

/// Production implementation of [`Downloader`].
///
/// Cheap to share behind `Arc`; all methods take `&self`.
pub struct HttpDownloader {
    cfg: DownloaderConfig,
    /// Current client for `Route::Direct` (carries the manual proxy, if any).
    direct: RwLock<reqwest::Client>,
    prefer_mirrors: AtomicBool,
    global: Arc<Semaphore>,
    per_host: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl std::fmt::Debug for HttpDownloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpDownloader")
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

impl HttpDownloader {
    /// Builds the downloader. Fails with `InvalidInput` when the initial
    /// proxy is malformed, or `Internal` if the HTTP stack cannot start.
    pub fn new(mut cfg: DownloaderConfig) -> AppResult<Self> {
        cfg.max_parallel = cfg.max_parallel.max(1);
        cfg.max_per_host = cfg.max_per_host.clamp(1, cfg.max_parallel);
        cfg.attempts_per_url = cfg.attempts_per_url.max(1);
        let proxy = cfg.proxy.take();
        let client = build_client(&cfg, proxy.as_ref())?;
        Ok(Self {
            prefer_mirrors: AtomicBool::new(cfg.prefer_mirrors),
            global: Arc::new(Semaphore::new(cfg.max_parallel)),
            per_host: Mutex::new(HashMap::new()),
            direct: RwLock::new(client),
            cfg,
        })
    }

    /// Changes mirror priority for requests started after the call.
    pub fn set_prefer_mirrors(&self, prefer: bool) {
        self.prefer_mirrors.store(prefer, Ordering::Relaxed);
    }

    fn current_client(&self) -> reqwest::Client {
        self.direct
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn host_semaphore(&self, url: &Url) -> Arc<Semaphore> {
        let key = format!(
            "{}:{}",
            url.host_str().unwrap_or_default(),
            url.port_or_known_default().unwrap_or_default()
        );
        let mut map = self.per_host.lock().unwrap_or_else(PoisonError::into_inner);
        map.entry(key)
            .or_insert_with(|| Arc::new(Semaphore::new(self.cfg.max_per_host)))
            .clone()
    }

    fn candidates(&self, urls: &[Url]) -> Vec<Url> {
        mirrors::expand_all(
            urls,
            self.cfg.use_bmclapi,
            self.prefer_mirrors.load(Ordering::Relaxed),
        )
    }

    fn backoff(&self, attempt: u32, hint: Option<Duration>) -> Duration {
        if let Some(h) = hint {
            return h;
        }
        let exp = self
            .cfg
            .backoff_base
            .saturating_mul(1u32 << attempt.min(16))
            .min(MAX_BACKOFF);
        // Cheap jitter (0..base/2) without pulling in an RNG.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let half = self.cfg.backoff_base.as_millis().max(1) / 2 + 1;
        let jitter = u64::try_from(u128::from(nanos) % half).unwrap_or(0);
        exp + Duration::from_millis(jitter)
    }

    /// Downloads one request trying every candidate with retries.
    async fn fetch_one(
        &self,
        req: DownloadReq,
        prog: &Progress<'_>,
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        sink.check_cancelled()?;
        let cancel = sink.cancel_token();

        if let Some(parent) = req.dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| errors::io_err(e, parent))?;
        }

        if let Some(len) = verified_len(&req.dest, req.sha1.as_deref(), req.size).await? {
            prog.reused(len);
            return Ok(());
        }

        let candidates = self.candidates(&req.urls);
        if candidates.is_empty() {
            return Err(
                AppError::new(ErrorCode::InvalidInput, "download without URLs")
                    .detail("path", req.dest.display().to_string()),
            );
        }

        let _global = self
            .global
            .acquire()
            .await
            .map_err(|_| AppError::internal("download semaphore closed"))?;

        let part = part_path(&req.dest);
        // Bytes of this request currently counted in `prog`.
        let mut counted = 0u64;
        let mut last_err: Option<AppError> = None;

        for url in &candidates {
            for attempt in 0..self.cfg.attempts_per_url {
                sink.check_cancelled()?;
                let host = self.host_semaphore(url);
                let res = {
                    let _host = host
                        .acquire()
                        .await
                        .map_err(|_| AppError::internal("host semaphore closed"))?;
                    self.attempt(url, &req, &part, prog, &mut counted, cancel)
                        .await
                };
                let fail = match res {
                    Ok(()) => return Ok(()),
                    Err(fail) => fail,
                };
                if fail.is_terminal() {
                    return Err(fail.err);
                }
                tracing::debug!(url = %url, attempt, error = %fail.err, "download attempt failed");
                let retry = fail.retry && attempt + 1 < self.cfg.attempts_per_url;
                let delay = self.backoff(attempt, fail.retry_after);
                last_err = Some(fail.err);
                if !retry {
                    break;
                }
                sleep_or_cancel(delay, cancel).await?;
            }
        }
        prog.rollback(counted);
        let err = last_err.unwrap_or_else(|| AppError::internal("no download attempt made"));
        tracing::warn!(dest = %req.dest.display(), error = %err, "download failed");
        Err(err.detail("path", req.dest.display().to_string()))
    }

    /// Single HTTP attempt: optional resume, stream to `.part`, verify, rename.
    async fn attempt(
        &self,
        url: &Url,
        req: &DownloadReq,
        part: &Path,
        prog: &Progress<'_>,
        counted: &mut u64,
        cancel: &CancelToken,
    ) -> Result<(), AttemptError> {
        // Every attempt re-derives progress from what is on disk.
        prog.rollback(std::mem::take(counted));
        let fatal_io = |e: std::io::Error| AttemptError::fatal(errors::io_err(e, part));

        let mut part_len = match tokio::fs::metadata(part).await {
            Ok(m) if m.is_file() => m.len(),
            _ => 0,
        };
        if let Some(size) = req.size {
            if part_len > size {
                remove_quiet(part).await;
                part_len = 0;
            } else if part_len == size && size > 0 {
                // A complete part left over from an interrupted rename.
                if hash_matches(part, req.sha1.as_deref()).await? {
                    finish(part, &req.dest).await?;
                    *counted = size;
                    prog.reused(size);
                    return Ok(());
                }
                remove_quiet(part).await;
                part_len = 0;
            }
        }

        let mut rb = self.current_client().get(url.clone());
        if part_len > 0 {
            rb = rb.header(RANGE, format!("bytes={part_len}-"));
        }
        let resp = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(AttemptError::fatal(errors::cancelled())),
            r = rb.send() => r.map_err(|e| AttemptError::retryable(errors::from_reqwest(&e, url)))?,
        };

        let status = resp.status();
        let (mut hasher, file) = if status == StatusCode::PARTIAL_CONTENT && part_len > 0 {
            let expected = format!("bytes {part_len}-");
            let range_ok = resp
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with(&expected));
            if !range_ok {
                remove_quiet(part).await;
                return Err(AttemptError::retryable(
                    AppError::new(ErrorCode::Network, "unexpected Content-Range on resume")
                        .detail("url", url.as_str()),
                ));
            }
            let hasher = hash_prefix(part).await?;
            let file = tokio::fs::OpenOptions::new()
                .append(true)
                .open(part)
                .await
                .map_err(fatal_io)?;
            *counted = part_len;
            prog.reused(part_len);
            (hasher, file)
        } else if status.is_success() {
            // 200 (server ignored Range or no part): start from scratch.
            part_len = 0;
            let file = tokio::fs::File::create(part).await.map_err(fatal_io)?;
            (Sha1::new(), file)
        } else if status == StatusCode::RANGE_NOT_SATISFIABLE {
            remove_quiet(part).await;
            return Err(AttemptError::retryable(
                AppError::new(ErrorCode::Network, "range not satisfiable; restarting")
                    .detail("url", url.as_str()),
            ));
        } else {
            let hint = errors::retry_after(resp.headers());
            return Err(errors::from_status(status, url, hint));
        };

        let mut out = BufWriter::with_capacity(HASH_BUF, file);
        let mut written = part_len;
        let mut stream = resp.bytes_stream();
        loop {
            let next = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    // Keep what we have for a later resume.
                    let _ = out.flush().await;
                    return Err(AttemptError::fatal(errors::cancelled()));
                }
                n = stream.next() => n,
            };
            let chunk = match next {
                None => break,
                Some(Ok(c)) => c,
                Some(Err(e)) => {
                    let _ = out.flush().await;
                    return Err(AttemptError::retryable(errors::from_reqwest(&e, url)));
                }
            };
            out.write_all(&chunk).await.map_err(fatal_io)?;
            hasher.update(&chunk);
            let n = chunk.len() as u64;
            written += n;
            *counted += n;
            prog.received(n);
            if req.size.is_some_and(|s| written > s) {
                break;
            }
        }
        out.flush().await.map_err(fatal_io)?;
        drop(out);

        let actual_sha1 = hex::encode(hasher.finalize());
        let size_bad = req.size.is_some_and(|s| s != written);
        let hash_bad = req
            .sha1
            .as_deref()
            .is_some_and(|want| !want.eq_ignore_ascii_case(&actual_sha1));
        if size_bad || hash_bad {
            remove_quiet(part).await;
            let mut err = AppError::new(
                ErrorCode::HashMismatch,
                "downloaded file failed verification",
            )
            .detail("url", url.as_str())
            .detail("actualSize", written.to_string())
            .detail("actualSha1", actual_sha1);
            if let Some(s) = req.size {
                err = err.detail("expectedSize", s.to_string());
            }
            if let Some(h) = &req.sha1 {
                err = err.detail("expectedSha1", h.clone());
            }
            return Err(AttemptError::retryable(err));
        }
        finish(part, &req.dest).await
    }

    async fn get_once(&self, url: &Url) -> Result<Vec<u8>, AttemptError> {
        let resp = self
            .current_client()
            .get(url.clone())
            .timeout(self.cfg.request_timeout)
            .send()
            .await
            .map_err(|e| AttemptError::retryable(errors::from_reqwest(&e, url)))?;
        let status = resp.status();
        if !status.is_success() {
            let hint = errors::retry_after(resp.headers());
            return Err(errors::from_status(status, url, hint));
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| AttemptError::retryable(errors::from_reqwest(&e, url)))?;
        Ok(body.to_vec())
    }
}

#[async_trait]
impl Downloader for HttpDownloader {
    async fn fetch_many(&self, reqs: Vec<DownloadReq>, sink: &dyn ProgressSink) -> AppResult<()> {
        sink.check_cancelled()?;
        let total = reqs
            .iter()
            .map(|r| r.size)
            .try_fold(0u64, |acc, s| s.map(|s| acc + s));
        let prog = Progress::new(sink, total);
        prog.emit(true);
        let limit = self.cfg.max_parallel;
        futures_util::stream::iter(reqs.into_iter().map(Ok::<_, AppError>))
            .try_for_each_concurrent(limit, |req| self.fetch_one(req, &prog, sink))
            .await?;
        prog.emit(true);
        Ok(())
    }

    async fn get_bytes(&self, url: &Url, _kind: ResourceKind) -> AppResult<Vec<u8>> {
        let mut last_err = None;
        for candidate in self.candidates(std::slice::from_ref(url)) {
            for attempt in 0..self.cfg.attempts_per_url {
                match self.get_once(&candidate).await {
                    Ok(body) => return Ok(body),
                    Err(fail) => {
                        let retry = fail.retry && attempt + 1 < self.cfg.attempts_per_url;
                        let delay = self.backoff(attempt, fail.retry_after);
                        last_err = Some(fail.err);
                        if !retry {
                            break;
                        }
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            AppError::new(ErrorCode::InvalidInput, "no URL candidates").detail("url", url.as_str())
        }))
    }

    fn client(&self, route: Route) -> reqwest::Client {
        match route {
            Route::Direct => self.current_client(),
            // TODO(net): Proxied → mihomo mixed-port client from lc-vpn;
            // Auto → direct with per-host switch to proxy after `Blocked`.
            Route::Proxied | Route::Auto => self.current_client(),
        }
    }

    fn mirrors_for(&self, original: &Url, _kind: ResourceKind) -> Vec<Url> {
        mirrors::candidates(
            original,
            self.cfg.use_bmclapi,
            self.prefer_mirrors.load(Ordering::Relaxed),
        )
    }

    fn set_manual_proxy(&self, proxy: Option<ProxyCfg>) {
        match build_client(&self.cfg, proxy.as_ref()) {
            Ok(client) => {
                *self.direct.write().unwrap_or_else(PoisonError::into_inner) = client;
                match &proxy {
                    Some(p) => tracing::info!(proxy = %redacted_proxy(p), "manual proxy applied"),
                    None => tracing::info!("manual proxy cleared"),
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "manual proxy rejected; keeping previous client");
            }
        }
    }
}

fn build_client(cfg: &DownloaderConfig, proxy: Option<&ProxyCfg>) -> AppResult<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .user_agent(cfg.user_agent.clone())
        .connect_timeout(cfg.connect_timeout)
        .read_timeout(cfg.read_timeout);
    if let Some(p) = proxy {
        b = b.proxy(build_proxy(p)?);
    }
    b.build()
        .map_err(|e| AppError::internal(format!("cannot build HTTP client: {e}")))
}

/// HTTP or SOCKS5 (`socks5h`, DNS resolved by the proxy) with optional auth.
/// Errors never include the password.
pub(crate) fn build_proxy(p: &ProxyCfg) -> AppResult<reqwest::Proxy> {
    let invalid = || {
        AppError::new(ErrorCode::InvalidInput, "invalid proxy configuration")
            .detail("host", p.host.clone())
            .detail("port", p.port.to_string())
    };
    let host = if p.host.contains(':') && !p.host.starts_with('[') {
        format!("[{}]", p.host)
    } else {
        p.host.clone()
    };
    let scheme = match p.kind {
        ProxyKind::Http => "http",
        ProxyKind::Socks5 => "socks5h",
    };
    let mut url = Url::parse(&format!("{scheme}://{host}:{}", p.port)).map_err(|_| invalid())?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err(invalid());
    }
    let username = p.username.as_deref().filter(|u| !u.is_empty());
    if p.kind == ProxyKind::Socks5 {
        if let Some(user) = username {
            url.set_username(user).map_err(|()| invalid())?;
            url.set_password(p.password.as_deref())
                .map_err(|()| invalid())?;
        }
    }
    let mut proxy = reqwest::Proxy::all(url.as_str()).map_err(|_| invalid())?;
    if p.kind == ProxyKind::Http {
        if let Some(user) = username {
            proxy = proxy.basic_auth(user, p.password.as_deref().unwrap_or_default());
        }
    }
    Ok(proxy)
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

async fn remove_quiet(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

async fn finish(part: &Path, dest: &Path) -> Result<(), AttemptError> {
    tokio::fs::rename(part, dest)
        .await
        .map_err(|e| AttemptError::fatal(errors::io_err(e, dest)))
}

async fn sleep_or_cancel(d: Duration, cancel: &CancelToken) -> AppResult<()> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(errors::cancelled()),
        () = tokio::time::sleep(d) => Ok(()),
    }
}

/// SHA-1 state over the whole file (used to continue hashing on resume).
async fn hash_prefix(path: &Path) -> Result<Sha1, AttemptError> {
    let p = path.to_owned();
    tokio::task::spawn_blocking(move || -> std::io::Result<Sha1> {
        use std::io::Read;
        let mut f = std::fs::File::open(&p)?;
        let mut h = Sha1::new();
        let mut buf = vec![0u8; HASH_BUF];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                return Ok(h);
            }
            h.update(&buf[..n]);
        }
    })
    .await
    .map_err(|e| AttemptError::fatal(AppError::internal(format!("hash task failed: {e}"))))?
    .map_err(|e| AttemptError::fatal(errors::io_err(e, path)))
}

async fn hash_matches(path: &Path, sha1: Option<&str>) -> Result<bool, AttemptError> {
    match sha1 {
        None => Ok(true),
        Some(want) => {
            let got = hex::encode(hash_prefix(path).await?.finalize());
            Ok(want.eq_ignore_ascii_case(&got))
        }
    }
}

/// `Some(len)` when `dest` already exists and passes every provided check.
/// Without size and hash, mere existence counts as done.
async fn verified_len(
    dest: &Path,
    sha1: Option<&str>,
    size: Option<u64>,
) -> AppResult<Option<u64>> {
    let meta = match tokio::fs::metadata(dest).await {
        Ok(m) if m.is_file() => m,
        _ => return Ok(None),
    };
    if size.is_some_and(|s| s != meta.len()) {
        return Ok(None);
    }
    match hash_matches(dest, sha1).await {
        Ok(true) => Ok(Some(meta.len())),
        Ok(false) => Ok(None),
        Err(e) => Err(e.err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy(kind: ProxyKind, host: &str) -> ProxyCfg {
        ProxyCfg {
            kind,
            host: host.into(),
            port: 1080,
            username: Some("user".into()),
            password: Some("p@ss:word".into()),
        }
    }

    #[test]
    fn proxies_build() {
        assert!(build_proxy(&proxy(ProxyKind::Socks5, "127.0.0.1")).is_ok());
        assert!(build_proxy(&proxy(ProxyKind::Http, "proxy.local")).is_ok());
        assert!(build_proxy(&proxy(ProxyKind::Socks5, "::1")).is_ok());
        let err = build_proxy(&proxy(ProxyKind::Http, "bad host/")).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(!format!("{err:?}").contains("p@ss"));
    }

    #[test]
    fn part_path_appends_suffix() {
        assert_eq!(
            part_path(Path::new("a/b/c.jar")),
            PathBuf::from("a/b/c.jar.part")
        );
    }

    #[tokio::test]
    async fn set_manual_proxy_swaps_client() {
        let d = HttpDownloader::new(DownloaderConfig::default()).unwrap();
        d.set_manual_proxy(Some(proxy(ProxyKind::Socks5, "127.0.0.1")));
        d.set_manual_proxy(Some(proxy(ProxyKind::Http, "bad host/")));
        d.set_manual_proxy(None);
        let _ = d.client(Route::Auto);
    }
}
