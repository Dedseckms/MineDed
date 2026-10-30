//! Real `Downloader`: parallel, resumable, hash-checked, with mirrors.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt};
use lc_runtime::{CancelToken, ProgressSink};
use lc_types::settings::{ProxyCfg, ProxyKind};
use lc_types::{AppError, AppResult, ErrorCode};
use reqwest::header::{CONTENT_RANGE, CONTENT_TYPE, RANGE};
use reqwest::StatusCode;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Semaphore;
use url::Url;

use crate::config::{redacted_proxy, DownloaderConfig, SystemProxyMode};
use crate::contract::{DownloadReq, Downloader, ResourceKind, Route};
use crate::errors::{self, AttemptError};
use crate::mirrors;
use crate::progress::Progress;
use crate::route::{self, HostMemory};
use crate::sysproxy::{self, SystemProxy};

/// Refused / reset / timed out / blocked, as opposed to an HTTP error status.
fn is_transport(err: &AppError) -> bool {
    matches!(err.code, ErrorCode::Network | ErrorCode::Blocked) && !errors::is_http_status(err)
}

const MAX_BACKOFF: Duration = Duration::from_secs(8);
/// How often the OS proxy settings are re-read.
const SYSTEM_RECHECK: Duration = Duration::from_secs(15);
const HASH_BUF: usize = 64 * 1024;
/// Target of the client handed out for `Route::Proxied` while no proxy is
/// configured: the discard port on loopback, so every request fails fast.
const UNCONFIGURED_PROXY: &str = "http://127.0.0.1:9";
/// How long a host that failed at the transport level (refused, reset,
/// timed out) stays demoted: one attempt per URL, mirrors first.
const UNREACHABLE_TTL: Duration = Duration::from_secs(10 * 60);

/// Production implementation of [`Downloader`].
///
/// Cheap to share behind `Arc`; all methods take `&self`.
pub struct HttpDownloader {
    cfg: DownloaderConfig,
    follow_redirects: bool,
    /// Never uses a proxy (system/env proxies ignored too).
    direct: reqwest::Client,
    /// Clients that depend on the effective proxy; rebuilt whenever a proxy
    /// source changes (`set_vpn_proxy`, `set_manual_proxy`, OS settings).
    routed: RwLock<Routed>,
    /// Inputs of `routed`. Lock order: `sources` before `routed`.
    sources: Mutex<Sources>,
    /// Hosts that `Route::Auto` currently sends to the proxy first.
    memory: Arc<HostMemory>,
    /// Hosts that recently failed at the transport level on every route.
    unreachable: HostMemory,
    /// `client(Route::Proxied)` without a proxy: every request fails.
    unconfigured: reqwest::Client,
    prefer_mirrors: AtomicBool,
    global: Arc<Semaphore>,
    per_host: Mutex<HashMap<String, Arc<Semaphore>>>,
    /// One writer per destination across concurrent `fetch_many` calls.
    path_locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    proxied_warned: AtomicBool,
}

#[derive(Clone)]
struct Routed {
    /// Every request through the effective proxy; `None` when none is set.
    proxy: Option<reqwest::Client>,
    /// Direct, except hosts remembered as "proxy first".
    auto: reqwest::Client,
    /// Present when the effective proxy is the OS one: its bypass rules.
    system: Option<Arc<SystemProxy>>,
}

impl Routed {
    /// The proxy client for `url`, unless the target bypasses it.
    fn proxy_for(&self, url: &Url) -> Option<reqwest::Client> {
        if self.system.as_ref().is_some_and(|s| s.bypasses(url)) {
            return None;
        }
        self.proxy.clone()
    }
}

/// Proxy sources in priority order: VPN, manual, system.
struct Sources {
    vpn: Option<Url>,
    manual: Option<ProxyCfg>,
    system: Option<SystemProxy>,
    /// Last OS proxy lookup (`SystemProxyMode::Detect` only).
    system_checked: Option<Instant>,
}

/// Which source the effective proxy came from (for logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProxySource {
    Vpn,
    Manual,
    System,
}

/// Which way one attempt goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leg {
    Direct,
    Proxy,
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
    pub fn new(cfg: DownloaderConfig) -> AppResult<Self> {
        Self::with_redirects(cfg, true)
    }

    /// Builds a downloader whose clients return redirects without following them.
    /// Use this for requests carrying credentials in custom headers.
    pub fn new_no_redirects(cfg: DownloaderConfig) -> AppResult<Self> {
        Self::with_redirects(cfg, false)
    }

    fn with_redirects(mut cfg: DownloaderConfig, follow_redirects: bool) -> AppResult<Self> {
        cfg.max_parallel = cfg.max_parallel.max(1);
        cfg.max_per_host = cfg.max_per_host.clamp(1, cfg.max_parallel);
        cfg.attempts_per_url = cfg.attempts_per_url.max(1);
        let (system, system_checked) = match &cfg.system_proxy {
            SystemProxyMode::Detect => (sysproxy::detect(), Some(Instant::now())),
            SystemProxyMode::Off => (None, None),
            SystemProxyMode::Fixed(url) => (
                Some(SystemProxy {
                    url: url.clone(),
                    bypass: Vec::new(),
                }),
                None,
            ),
        };
        let mut sources = Sources {
            vpn: None,
            manual: cfg.proxy.take(),
            system,
            system_checked,
        };
        let memory = Arc::new(HostMemory::new(cfg.proxy_first_ttl));
        let direct = build_client(&cfg, None, follow_redirects)?;
        let routed = match build_routed(&cfg, &sources, &memory, &direct, follow_redirects) {
            Ok(r) => r,
            // A broken OS proxy must not prevent startup.
            Err(_) if sources.manual.is_none() => {
                tracing::warn!("system proxy rejected; ignoring it");
                sources.system = None;
                build_routed(&cfg, &sources, &memory, &direct, follow_redirects)?
            }
            Err(e) => return Err(e),
        };
        let unconfigured = build_client(
            &cfg,
            Some(
                reqwest::Proxy::all(UNCONFIGURED_PROXY)
                    .map_err(|e| AppError::internal(format!("cannot build proxy: {e}")))?,
            ),
            follow_redirects,
        )?;
        Ok(Self {
            prefer_mirrors: AtomicBool::new(cfg.prefer_mirrors),
            global: Arc::new(Semaphore::new(cfg.max_parallel)),
            per_host: Mutex::new(HashMap::new()),
            path_locks: Mutex::new(HashMap::new()),
            proxied_warned: AtomicBool::new(false),
            direct,
            routed: RwLock::new(routed),
            sources: Mutex::new(sources),
            memory,
            unreachable: HostMemory::new(UNREACHABLE_TTL),
            unconfigured,
            cfg,
            follow_redirects,
        })
    }

    /// Changes mirror priority for requests started after the call.
    pub fn set_prefer_mirrors(&self, prefer: bool) {
        self.prefer_mirrors.store(prefer, Ordering::Relaxed);
    }

    /// `get_bytes` with an explicit route. The proxy is the built-in VPN,
    /// else the manual proxy, else the OS system proxy. `Route::Proxied` fails
    /// with `Network` when there is none; `Route::Auto` without a proxy
    /// behaves like `Route::Direct`.
    pub async fn get_bytes_via(
        &self,
        url: &Url,
        kind: ResourceKind,
        route: Route,
    ) -> AppResult<Vec<u8>> {
        // Fails early when the route is unavailable (no proxy for `Proxied`).
        self.legs(url, route)
            .map_err(|e| e.detail("url", url.as_str()))?;
        let mut last_err = None;
        // No hash to verify against: mirrors only after the original.
        for candidate in self.candidates(std::slice::from_ref(url), false) {
            let res = self
                .run_routed(&candidate, route, None, |client| {
                    let candidate = &candidate;
                    async move { self.get_once(&client, candidate, kind).await }
                })
                .await;
            match res {
                Ok(body) => return Ok(body),
                Err(fail) if fail.is_terminal() => return Err(fail.err),
                Err(fail) => last_err = Some(fail.err),
            }
        }
        Err(match last_err {
            Some(e) => self.with_hint(e),
            None => AppError::new(ErrorCode::InvalidInput, "no URL candidates")
                .detail("url", url.as_str()),
        })
    }

    /// Every candidate failed at the transport level and there is no proxy:
    /// tells the UI to suggest one (`hint=useProxy`).
    fn with_hint(&self, err: AppError) -> AppError {
        if is_transport(&err) && self.routed().proxy.is_none() {
            err.detail("hint", "useProxy")
        } else {
            err
        }
    }

    fn routed(&self) -> Routed {
        self.refresh_system_proxy();
        self.routed
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Re-reads the OS proxy at most every [`SYSTEM_RECHECK`] and rebuilds the
    /// clients when it changed.
    fn refresh_system_proxy(&self) {
        if self.cfg.system_proxy != SystemProxyMode::Detect {
            return;
        }
        let mut s = self.sources.lock().unwrap_or_else(PoisonError::into_inner);
        if s.system_checked
            .is_some_and(|t| t.elapsed() < SYSTEM_RECHECK)
        {
            return;
        }
        s.system_checked = Some(Instant::now());
        let now = sysproxy::detect().filter(|p| url_target(&p.url).is_ok());
        if now == s.system {
            return;
        }
        let prev = std::mem::replace(&mut s.system, now);
        if !self.apply(&s) {
            s.system = prev;
        }
    }

    /// Rebuilds the proxy-dependent clients from `s`. On failure (malformed
    /// proxy) logs and keeps the previous clients; returns whether it applied.
    fn apply(&self, s: &Sources) -> bool {
        match build_routed(
            &self.cfg,
            s,
            &self.memory,
            &self.direct,
            self.follow_redirects,
        ) {
            Ok(routed) => {
                *self.routed.write().unwrap_or_else(PoisonError::into_inner) = routed;
                match effective(s) {
                    Some((source, desc)) => {
                        tracing::info!(?source, proxy = %desc, "proxy route updated");
                    }
                    None => tracing::info!("no proxy configured"),
                }
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "proxy rejected; keeping previous route");
                false
            }
        }
    }

    /// Ordered `(leg, client)` pairs a request to `url` goes through.
    fn legs(&self, url: &Url, route: Route) -> AppResult<Vec<(Leg, reqwest::Client)>> {
        let routed = self.routed();
        let direct = (Leg::Direct, self.direct.clone());
        let proxy = routed.proxy_for(url);
        if proxy.is_none() && routed.proxy.is_some() {
            // Target bypasses the OS proxy (loopback/LAN/bypass list): direct
            // is the only route, even for `Proxied`.
            return Ok(vec![direct]);
        }
        Ok(match (route, proxy) {
            (Route::Direct, _) | (Route::Auto, None) => vec![direct],
            (Route::Proxied, Some(p)) => vec![(Leg::Proxy, p)],
            (Route::Proxied, None) => return Err(route::no_proxy_configured()),
            (Route::Auto, Some(p)) if self.memory.is_proxy_first(url) => {
                vec![(Leg::Proxy, p), direct]
            }
            (Route::Auto, Some(p)) => vec![direct, (Leg::Proxy, p)],
        })
    }

    /// Runs `op` against `url` over the legs of `route`, with retries per leg.
    /// A leg failing with a network error, `Blocked`, a hash mismatch or a
    /// spoofed response hands over to the next leg; transport-level failures
    /// skip the same-leg retries when another leg is available. Success via
    /// the proxy after a failed direct attempt marks the host "proxy first".
    async fn run_routed<T, F, Fut>(
        &self,
        url: &Url,
        route: Route,
        cancel: Option<&CancelToken>,
        mut op: F,
    ) -> Result<T, AttemptError>
    where
        F: FnMut(reqwest::Client) -> Fut,
        Fut: Future<Output = Result<T, AttemptError>>,
    {
        let legs = self.legs(url, route).map_err(AttemptError::fatal)?;
        let n = legs.len();
        let mut last: Option<AttemptError> = None;
        // A host that just failed at the transport level gets one try per
        // URL, so a blocked origin costs one timeout before the mirror.
        let attempts = if self.unreachable.contains(url) {
            1
        } else {
            self.cfg.attempts_per_url
        };
        let res = 'legs: {
            for (i, (leg, client)) in legs.into_iter().enumerate() {
                let has_next = i + 1 < n;
                for attempt in 0..attempts {
                    if cancel.is_some_and(CancelToken::is_cancelled) {
                        return Err(AttemptError::fatal(errors::cancelled()));
                    }
                    let fail = match op(client.clone()).await {
                        Ok(v) => {
                            if i > 0 {
                                match leg {
                                    Leg::Proxy => self.memory.remember(url),
                                    Leg::Direct => self.memory.forget(url),
                                }
                            }
                            break 'legs Ok(v);
                        }
                        Err(fail) => fail,
                    };
                    if fail.is_terminal() {
                        break 'legs Err(fail);
                    }
                    tracing::debug!(url = %url, ?leg, attempt, error = %fail.err, "attempt failed");
                    let escalate_now = has_next
                        && route::worth_proxy(&fail.err)
                        && !errors::is_http_status(&fail.err);
                    let retry = fail.retry && !escalate_now && attempt + 1 < attempts;
                    let delay = self.backoff(attempt, fail.retry_after);
                    last = Some(fail);
                    if !retry {
                        break;
                    }
                    match cancel {
                        Some(c) => {
                            if let Err(e) = sleep_or_cancel(delay, c).await {
                                break 'legs Err(AttemptError::fatal(e));
                            }
                        }
                        None => tokio::time::sleep(delay).await,
                    }
                }
                let Some(fail) = &last else { break };
                if !has_next || !route::worth_proxy(&fail.err) {
                    break;
                }
                tracing::info!(url = %url, from = ?leg, error = %fail.err, "route failed; trying the other route");
            }
            Err(last.unwrap_or_else(|| {
                AttemptError::fatal(AppError::internal("no download attempt made"))
            }))
        };
        match &res {
            Ok(_) => self.unreachable.forget(url),
            Err(fail) if is_transport(&fail.err) => {
                if self.unreachable.insert(url).is_some() {
                    tracing::info!(url = %url, "host unreachable; demoted for a while");
                }
            }
            Err(_) => {}
        }
        res
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

    /// Candidate order per the mirror trust policy (CONTRACTS §2): third-party
    /// mirrors may go first (`prefer_mirrors`) only when the result is
    /// verified by SHA-1; unverified requests always try the original first.
    /// Verified requests also move recently unreachable hosts to the end.
    fn candidates(&self, urls: &[Url], verified: bool) -> Vec<Url> {
        let prefer = verified && self.prefer_mirrors.load(Ordering::Relaxed);
        let mut out = mirrors::expand_all(urls, self.cfg.use_bmclapi, prefer);
        if verified {
            // Stable: the original order is kept within each group.
            out.sort_by_key(|u| self.unreachable.contains(u));
        }
        out
    }

    /// Serialises work on one destination path; entries are dropped when idle.
    async fn with_path_lock<T>(
        &self,
        dest: &Path,
        cancel: &CancelToken,
        work: impl std::future::Future<Output = AppResult<T>>,
    ) -> AppResult<T> {
        let lock = {
            let mut map = self
                .path_locks
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            map.entry(dest.to_path_buf()).or_default().clone()
        };
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(errors::cancelled()),
            guard = lock.lock() => {
                let res = work.await;
                drop(guard);
                res
            }
        };
        let mut map = self
            .path_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Clones are only made under the map lock, so this count is stable here:
        // 2 = the map's entry + ours, i.e. nobody else is waiting.
        if Arc::strong_count(&lock) == 2 {
            map.remove(dest);
        }
        result
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

    /// Downloads one request under the per-destination lock.
    async fn fetch_one(
        &self,
        req: DownloadReq,
        prog: &Progress<'_>,
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        sink.check_cancelled()?;
        let dest = req.dest.clone();
        self.with_path_lock(
            &dest,
            sink.cancel_token(),
            self.fetch_locked(req, prog, sink),
        )
        .await
    }

    /// Downloads one request trying every candidate with retries.
    async fn fetch_locked(
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

        let candidates = self.candidates(&req.urls, req.sha1.is_some());
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
        let counted = AtomicU64::new(0);
        let mut last_err: Option<AppError> = None;

        let (req_ref, part_ref, counted_ref) = (&req, part.as_path(), &counted);
        for url in &candidates {
            sink.check_cancelled()?;
            let res = self
                .run_routed(url, Route::Auto, Some(cancel), move |client| async move {
                    let host = self.host_semaphore(url);
                    let _host = host.acquire().await.map_err(|_| {
                        AttemptError::fatal(AppError::internal("host semaphore closed"))
                    })?;
                    self.attempt(&client, url, req_ref, part_ref, prog, counted_ref, cancel)
                        .await
                })
                .await;
            match res {
                Ok(()) => return Ok(()),
                Err(fail) if fail.is_terminal() => return Err(fail.err),
                Err(fail) => last_err = Some(fail.err),
            }
        }
        prog.rollback(counted.swap(0, Ordering::Relaxed));
        let err = self
            .with_hint(last_err.unwrap_or_else(|| AppError::internal("no download attempt made")));
        tracing::warn!(dest = %req.dest.display(), error = %err, "download failed");
        Err(err.detail("path", req.dest.display().to_string()))
    }

    /// Single HTTP attempt: optional resume, stream to `.part`, verify, rename.
    #[allow(clippy::too_many_arguments)]
    async fn attempt(
        &self,
        client: &reqwest::Client,
        url: &Url,
        req: &DownloadReq,
        part: &Path,
        prog: &Progress<'_>,
        counted: &AtomicU64,
        cancel: &CancelToken,
    ) -> Result<(), AttemptError> {
        // Every attempt re-derives progress from what is on disk.
        prog.rollback(counted.swap(0, Ordering::Relaxed));
        // Without a hash, an HTML block page would be saved as the file.
        let sniff = req.sha1.is_none() && route::expects_data(req.kind);
        let fatal_io = |e: std::io::Error| AttemptError::fatal(errors::io_err(e, part));

        let mut part_len = match tokio::fs::symlink_metadata(part).await {
            Ok(m) if is_regular_part(&m) => m.len(),
            Ok(_) => {
                return Err(fatal_io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "download staging path is not a regular file",
                )))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(fatal_io(e)),
        };
        if part_len > 0 && req.sha1.is_none() {
            // Without a hash the old bytes cannot be verified: never resume blindly.
            remove_quiet(part).await;
            part_len = 0;
        }
        if let Some(size) = req.size {
            if part_len > size {
                remove_quiet(part).await;
                part_len = 0;
            } else if part_len == size && size > 0 {
                // A complete part left over from an interrupted rename.
                if hash_matches(part, req.sha1.as_deref()).await? {
                    finish(part, &req.dest).await?;
                    counted.store(size, Ordering::Relaxed);
                    prog.reused(size);
                    return Ok(());
                }
                remove_quiet(part).await;
                part_len = 0;
            }
        }

        let mut rb = client.get(url.clone());
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
            // Resume into a private inode: an imported .part may be a hardlink.
            let file = resume_part(part, cancel).await.map_err(|e| {
                if cancel.is_cancelled() {
                    AttemptError::fatal(errors::cancelled())
                } else {
                    fatal_io(e)
                }
            })?;
            let hasher = hash_prefix(part).await?;
            counted.store(part_len, Ordering::Relaxed);
            prog.reused(part_len);
            (hasher, file)
        } else if status.is_success() {
            if sniff && route::looks_like_block_page(content_type(&resp), b"") {
                return Err(AttemptError::fatal(route::spoofed(url)));
            }
            // 200 (server ignored Range or no part): start from scratch.
            part_len = 0;
            // Unlink any prior file before creation; never truncate through a link.
            match tokio::fs::remove_file(part).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(fatal_io(e)),
            }
            let file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(part)
                .await
                .map_err(fatal_io)?;
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
            if sniff && written == 0 && route::looks_like_block_page(None, &chunk) {
                drop(out);
                remove_quiet(part).await;
                return Err(AttemptError::fatal(route::spoofed(url)));
            }
            out.write_all(&chunk).await.map_err(fatal_io)?;
            hasher.update(&chunk);
            let n = chunk.len() as u64;
            written += n;
            counted.fetch_add(n, Ordering::Relaxed);
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
            // A bad body from this URL is unlikely to change: try the next candidate.
            return Err(AttemptError::fatal(err));
        }
        finish(part, &req.dest).await
    }

    async fn get_once(
        &self,
        client: &reqwest::Client,
        url: &Url,
        kind: ResourceKind,
    ) -> Result<Vec<u8>, AttemptError> {
        let resp = client
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
        let limit = self.cfg.max_body_bytes;
        let too_big = || {
            AttemptError::fatal(
                AppError::new(ErrorCode::Network, "response body exceeds limit")
                    .detail("url", url.as_str())
                    .detail("limit", limit.to_string()),
            )
        };
        let sniff = route::expects_data(kind);
        if sniff && route::looks_like_block_page(content_type(&resp), b"") {
            return Err(AttemptError::fatal(route::spoofed(url)));
        }
        if resp.content_length().is_some_and(|len| len > limit) {
            return Err(too_big());
        }
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| AttemptError::retryable(errors::from_reqwest(&e, url)))?;
            if body.len() as u64 + chunk.len() as u64 > limit {
                return Err(too_big());
            }
            body.extend_from_slice(&chunk);
        }
        if sniff && route::looks_like_block_page(None, &body) {
            return Err(AttemptError::fatal(route::spoofed(url)));
        }
        Ok(body)
    }
}

fn content_type(resp: &reqwest::Response) -> Option<&str> {
    resp.headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
}

#[async_trait]
impl Downloader for HttpDownloader {
    async fn fetch_many(&self, reqs: Vec<DownloadReq>, sink: &dyn ProgressSink) -> AppResult<()> {
        sink.check_cancelled()?;
        let reqs = dedup_by_dest(reqs);
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

    /// Uses `Route::Auto` (see [`HttpDownloader::get_bytes_via`]). For kinds
    /// other than `Other`, an HTML response counts as a spoofed block page.
    async fn get_bytes(&self, url: &Url, kind: ResourceKind) -> AppResult<Vec<u8>> {
        self.get_bytes_via(url, kind, Route::Auto).await
    }

    /// `Direct` never uses a proxy. `Auto` is direct except for hosts
    /// currently remembered as "proxy first". `Proxied` goes through the
    /// effective proxy (VPN, else manual, else OS); without one, every request
    /// of the returned client fails.
    fn client(&self, route: Route) -> reqwest::Client {
        match route {
            Route::Direct => self.direct.clone(),
            Route::Auto => self.routed().auto,
            Route::Proxied => match self.routed().proxy {
                Some(p) => p,
                None => {
                    if !self.proxied_warned.swap(true, Ordering::Relaxed) {
                        tracing::warn!(
                            "proxied client requested but no proxy is configured; requests will fail"
                        );
                    }
                    self.unconfigured.clone()
                }
            },
        }
    }

    /// Ordering for SHA-1-verified downloads (honours `prefer_mirrors`).
    /// Callers without a hash must keep the original first (CONTRACTS §2);
    /// `fetch_many`/`get_bytes` apply that policy themselves.
    fn upgrade_client(&self, route: Route) -> reqwest::Client {
        let proxy = match route {
            Route::Direct => None,
            _ => {
                let target = {
                    let s = self.sources.lock().unwrap_or_else(PoisonError::into_inner);
                    if let Some(u) = &s.vpn {
                        Some(u.clone())
                    } else if let Some(p) = &s.manual {
                        proxy_target(p).ok().map(|t| t.url)
                    } else {
                        s.system.as_ref().map(|p| p.url.clone())
                    }
                };
                target.and_then(|u| reqwest::Proxy::all(u.as_str()).ok())
            }
        };
        build_client_with(&self.cfg, proxy, false, true)
            .unwrap_or_else(|_| self.unconfigured.clone())
    }

    fn mirrors_for(&self, original: &Url, _kind: ResourceKind) -> Vec<Url> {
        mirrors::candidates(
            original,
            self.cfg.use_bmclapi,
            self.prefer_mirrors.load(Ordering::Relaxed),
        )
    }

    fn set_manual_proxy(&self, proxy: Option<ProxyCfg>) {
        // Validate even when a higher-priority source hides it for now.
        if let Some(Err(e)) = proxy.as_ref().map(proxy_target) {
            tracing::warn!(error = %e, "manual proxy rejected; keeping previous one");
            return;
        }
        let mut s = self.sources.lock().unwrap_or_else(PoisonError::into_inner);
        let prev = std::mem::replace(&mut s.manual, proxy);
        if !self.apply(&s) {
            s.manual = prev;
        }
    }

    /// The VPN's local mixed-port proxy, preferred over the manual and OS
    /// proxies. Malformed URLs are rejected (logged, previous route kept).
    fn set_vpn_proxy(&self, proxy: Option<Url>) {
        if let Some(Err(e)) = proxy.as_ref().map(url_target) {
            tracing::warn!(error = %e, "VPN proxy rejected; keeping previous one");
            return;
        }
        let mut s = self.sources.lock().unwrap_or_else(PoisonError::into_inner);
        let prev = std::mem::replace(&mut s.vpn, proxy);
        if !self.apply(&s) {
            s.vpn = prev;
        }
    }
}

fn build_client(
    cfg: &DownloaderConfig,
    proxy: Option<reqwest::Proxy>,
    follow_redirects: bool,
) -> AppResult<reqwest::Client> {
    build_client_with(cfg, proxy, follow_redirects, false)
}

fn build_client_with(
    cfg: &DownloaderConfig,
    proxy: Option<reqwest::Proxy>,
    follow_redirects: bool,
    upgrade: bool,
) -> AppResult<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .user_agent(cfg.user_agent.clone())
        .connect_timeout(cfg.connect_timeout)
        .read_timeout(cfg.read_timeout);
    if !follow_redirects {
        b = b.redirect(reqwest::redirect::Policy::none());
    }
    if upgrade {
        // HTTP/2 drops `Connection`/`Upgrade`; the WebSocket handshake needs HTTP/1.1.
        b = b.http1_only();
    }
    b = match proxy {
        Some(p) => b.proxy(p),
        // `None` means direct: reqwest must not pick up HTTP(S)_PROXY or the
        // OS proxy on its own; those are explicit fallbacks (`Sources`).
        None => b.no_proxy(),
    };
    b.build()
        .map_err(|e| AppError::internal(format!("cannot build HTTP client: {e}")))
}

/// The effective proxy source and a log-safe description of it.
fn effective(s: &Sources) -> Option<(ProxySource, String)> {
    if let Some(u) = &s.vpn {
        Some((ProxySource::Vpn, sysproxy::redacted(u)))
    } else if let Some(p) = &s.manual {
        Some((ProxySource::Manual, redacted_proxy(p)))
    } else {
        s.system
            .as_ref()
            .map(|p| (ProxySource::System, sysproxy::redacted(&p.url)))
    }
}

/// Proxy-dependent clients for the effective proxy of `s`.
fn build_routed(
    cfg: &DownloaderConfig,
    s: &Sources,
    memory: &Arc<HostMemory>,
    direct: &reqwest::Client,
    follow_redirects: bool,
) -> AppResult<Routed> {
    let (target, system) = if let Some(u) = &s.vpn {
        (url_target(u)?, None)
    } else if let Some(p) = &s.manual {
        (proxy_target(p)?, None)
    } else if let Some(sp) = &s.system {
        (url_target(&sp.url)?, Some(Arc::new(sp.clone())))
    } else {
        return Ok(Routed {
            proxy: None,
            auto: direct.clone(),
            system: None,
        });
    };
    let url = target.url.clone();
    let proxy = match &system {
        None => reqwest::Proxy::all(url.as_str())
            .map_err(|e| AppError::internal(format!("cannot build proxy: {e}")))?,
        Some(sp) => {
            let (sp, url) = (sp.clone(), url.clone());
            reqwest::Proxy::custom(move |dest| (!sp.bypasses(dest)).then(|| url.clone()))
        }
    };
    let (memory, bypass) = (memory.clone(), system.clone());
    let auto = reqwest::Proxy::custom(move |dest| {
        let use_proxy =
            memory.is_proxy_first(dest) && !bypass.as_ref().is_some_and(|b| b.bypasses(dest));
        use_proxy.then(|| url.clone())
    });
    Ok(Routed {
        proxy: Some(build_client(
            cfg,
            Some(target.apply(proxy)),
            follow_redirects,
        )?),
        auto: build_client(cfg, Some(target.apply(auto)), follow_redirects)?,
        system,
    })
}

/// Target for a ready proxy URL (VPN or OS proxy). Credentials, if any, stay
/// in the URL (reqwest applies them) and never reach logs or errors.
fn url_target(url: &Url) -> AppResult<ProxyTarget> {
    let invalid = || {
        AppError::new(ErrorCode::InvalidInput, "invalid proxy URL")
            .detail("proxy", sysproxy::redacted(url))
    };
    // reqwest silently treats unknown schemes as "no proxy".
    if !matches!(
        url.scheme(),
        "http" | "https" | "socks4" | "socks4a" | "socks5" | "socks5h"
    ) || url.host_str().is_none_or(str::is_empty)
    {
        return Err(invalid());
    }
    reqwest::Proxy::all(url.as_str()).map_err(|_| invalid())?;
    Ok(ProxyTarget {
        url: url.clone(),
        http_auth: None,
    })
}

/// Proxy URL (SOCKS credentials embedded) plus HTTP basic auth, if any.
/// Deliberately not `Debug`: it holds the password.
struct ProxyTarget {
    url: Url,
    http_auth: Option<(String, String)>,
}

impl ProxyTarget {
    fn apply(&self, proxy: reqwest::Proxy) -> reqwest::Proxy {
        match &self.http_auth {
            Some((user, pass)) => proxy.basic_auth(user, pass),
            None => proxy,
        }
    }
}

fn invalid_proxy(p: &ProxyCfg) -> AppError {
    AppError::new(ErrorCode::InvalidInput, "invalid proxy configuration")
        .detail("host", p.host.clone())
        .detail("port", p.port.to_string())
}

/// HTTP or SOCKS5 (`socks5h`, DNS resolved by the proxy) with optional auth.
/// Errors never include the password.
#[cfg(test)]
pub(crate) fn build_proxy(p: &ProxyCfg) -> AppResult<reqwest::Proxy> {
    let target = proxy_target(p)?;
    let proxy = reqwest::Proxy::all(target.url.as_str()).map_err(|_| invalid_proxy(p))?;
    Ok(target.apply(proxy))
}

fn proxy_target(p: &ProxyCfg) -> AppResult<ProxyTarget> {
    let invalid = || invalid_proxy(p);
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
    // Validate once here so the infallible `Proxy::custom` path is safe too.
    reqwest::Proxy::all(url.as_str()).map_err(|_| invalid())?;
    let http_auth = match (p.kind, username) {
        (ProxyKind::Http, Some(user)) => {
            Some((user.to_owned(), p.password.clone().unwrap_or_default()))
        }
        _ => None,
    };
    Ok(ProxyTarget { url, http_auth })
}

/// Keeps the first request per destination so one batch never races on a file.
fn dedup_by_dest(reqs: Vec<DownloadReq>) -> Vec<DownloadReq> {
    let mut seen: HashMap<PathBuf, usize> = HashMap::new();
    let mut out: Vec<DownloadReq> = Vec::with_capacity(reqs.len());
    for req in reqs {
        if let Some(&i) = seen.get(&req.dest) {
            let kept = &out[i];
            if kept.sha1 != req.sha1 || kept.size != req.size {
                tracing::warn!(
                    dest = %req.dest.display(),
                    "conflicting duplicate download request ignored"
                );
            }
            continue;
        }
        seen.insert(req.dest.clone(), out.len());
        out.push(req);
    }
    out
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn is_regular_part(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }
    meta.file_type().is_file()
}

/// Detach resumed data from pre-existing hardlinks before writing to it.
async fn resume_part(part: &Path, cancel: &CancelToken) -> std::io::Result<tokio::fs::File> {
    let mut source = tokio::fs::File::open(part).await?;
    // Only read the existing file; all writes go to an exclusively created inode.
    let mut name = part.as_os_str().to_owned();
    name.push(format!(".{}.tmp", lc_types::TaskId::new().0));
    let temp = PathBuf::from(name);
    // Create and arm cleanup in the same poll, before any cancellation point.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    // Also clean up if a sibling download fails and drops this future.
    let mut cleanup = PartCopyCleanup(Some(temp));
    let mut file = tokio::fs::File::from_std(file);
    tokio::select! {
        biased;
        () = cancel.cancelled() => {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "download cancelled"));
        }
        copied = tokio::io::copy(&mut source, &mut file) => { copied?; }
    }
    file.flush().await?;
    // `temp` is kept until the rename completes, including cancellation by drop.
    if let Some(temp) = cleanup.0.as_ref() {
        tokio::fs::rename(temp, part).await?;
    }
    cleanup.0 = None;
    Ok(file)
}

struct PartCopyCleanup(Option<PathBuf>);

impl Drop for PartCopyCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.0.as_ref() {
            // Open staging handles permit deletion on Windows as well as Unix.
            let _ = std::fs::remove_file(path);
        }
    }
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

    #[tokio::test]
    async fn cancelled_resume_copy_preserves_original_and_removes_temp() {
        let dir = std::env::temp_dir().join(format!("lc-http-copy-{}", lc_types::TaskId::new().0));
        std::fs::create_dir(&dir).unwrap();
        let part = dir.join("download.part");
        std::fs::write(&part, b"original partial data").unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = resume_part(&part, &cancel).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(std::fs::read(&part).unwrap(), b"original partial data");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn no_sys() -> DownloaderConfig {
        DownloaderConfig {
            system_proxy: SystemProxyMode::Off,
            ..DownloaderConfig::default()
        }
    }

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
    fn unverified_requests_never_prefer_mirrors() {
        let d = HttpDownloader::new(DownloaderConfig {
            prefer_mirrors: true,
            ..no_sys()
        })
        .unwrap();
        let orig = Url::parse("https://piston-meta.mojang.com/a.json").unwrap();
        let mirror = Url::parse("https://bmclapi2.bangbang93.com/a.json").unwrap();
        let both = [orig.clone()];
        assert_eq!(
            d.candidates(&both, true),
            vec![mirror.clone(), orig.clone()]
        );
        assert_eq!(
            d.candidates(&both, false),
            vec![orig.clone(), mirror.clone()]
        );
        d.set_prefer_mirrors(false);
        assert_eq!(d.candidates(&both, true), vec![orig, mirror]);
    }

    #[test]
    fn dedup_keeps_first_per_dest() {
        let r = |u: &str, dest: &str| DownloadReq {
            urls: vec![Url::parse(u).unwrap()],
            sha1: None,
            size: Some(1),
            dest: PathBuf::from(dest),
            kind: ResourceKind::Other,
        };
        let out = dedup_by_dest(vec![
            r("http://a/1", "x"),
            r("http://a/2", "y"),
            r("http://a/3", "x"),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].urls[0].as_str(), "http://a/1");
        assert_eq!(out[1].dest, PathBuf::from("y"));
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
        let d = HttpDownloader::new(no_sys()).unwrap();
        d.set_manual_proxy(Some(proxy(ProxyKind::Socks5, "127.0.0.1")));
        d.set_manual_proxy(Some(proxy(ProxyKind::Http, "bad host/")));
        d.set_manual_proxy(None);
        let _ = d.client(Route::Auto);
        let _ = d.client(Route::Proxied);
    }

    #[test]
    fn auto_without_proxy_is_direct_only() {
        let d = HttpDownloader::new(no_sys()).unwrap();
        let u = Url::parse("https://api.modrinth.com/v2/x").unwrap();
        let legs: Vec<Leg> = d
            .legs(&u, Route::Auto)
            .unwrap()
            .into_iter()
            .map(|(l, _)| l)
            .collect();
        assert_eq!(legs, vec![Leg::Direct]);
        assert_eq!(
            d.legs(&u, Route::Proxied).unwrap_err().code,
            ErrorCode::Network
        );
        d.set_manual_proxy(Some(proxy(ProxyKind::Http, "127.0.0.1")));
        let order = |d: &HttpDownloader| -> Vec<Leg> {
            d.legs(&u, Route::Auto)
                .unwrap()
                .into_iter()
                .map(|(l, _)| l)
                .collect()
        };
        assert_eq!(order(&d), vec![Leg::Direct, Leg::Proxy]);
        d.memory.remember(&u);
        assert_eq!(order(&d), vec![Leg::Proxy, Leg::Direct]);
    }
}
