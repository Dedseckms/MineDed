//! `HttpDownloader` against a local axum server (no external network).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{OriginalUri, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use lc_http::{
    DownloadReq, Downloader, DownloaderConfig, HttpDownloader, ResourceKind, SystemProxyMode,
};
use lc_runtime::{CancelToken, ProgressSink};
use lc_types::settings::{ProxyCfg, ProxyKind};
use lc_types::task::TaskEvent;
use lc_types::{ErrorCode, TaskId};
use sha1::{Digest, Sha1};
use url::Url;

// ---------- test server ----------

#[derive(Clone, Default)]
struct Srv {
    data: Arc<Vec<u8>>,
    hits: Arc<Mutex<HashMap<String, usize>>>,
    ranges: Arc<Mutex<Vec<String>>>,
    proxy_auth: Arc<Mutex<Vec<String>>>,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
}

impl Srv {
    fn hit(&self, uri: &OriginalUri) {
        *self
            .hits
            .lock()
            .unwrap()
            .entry(uri.0.path().to_owned())
            .or_default() += 1;
    }

    fn hits(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }
}

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

fn sha1_hex(b: &[u8]) -> String {
    hex::encode(Sha1::digest(b))
}

async fn ok(State(s): State<Srv>, uri: OriginalUri, headers: HeaderMap) -> Response {
    s.hit(&uri);
    let data = s.data.clone();
    if let Some(r) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        s.ranges.lock().unwrap().push(r.to_owned());
        let start: usize = r
            .trim_start_matches("bytes=")
            .trim_end_matches('-')
            .parse()
            .unwrap();
        if start >= data.len() {
            return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
        }
        let body = data[start..].to_vec();
        return (
            StatusCode::PARTIAL_CONTENT,
            [(
                header::CONTENT_RANGE,
                format!("bytes {start}-{}/{}", data.len() - 1, data.len()),
            )],
            body,
        )
            .into_response();
    }
    data.to_vec().into_response()
}

async fn norange(State(s): State<Srv>, uri: OriginalUri) -> Response {
    s.hit(&uri);
    s.data.to_vec().into_response()
}

async fn bad(State(s): State<Srv>, uri: OriginalUri) -> Response {
    s.hit(&uri);
    vec![0u8; s.data.len()].into_response()
}

async fn status500(State(s): State<Srv>, uri: OriginalUri) -> Response {
    s.hit(&uri);
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

async fn status404(State(s): State<Srv>, uri: OriginalUri) -> Response {
    s.hit(&uri);
    StatusCode::NOT_FOUND.into_response()
}

/// Full payload without `Content-Length` (chunked transfer).
async fn chunked(State(s): State<Srv>, uri: OriginalUri) -> Response {
    s.hit(&uri);
    let chunks: Vec<Result<Bytes, std::io::Error>> = s
        .data
        .chunks(4096)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    Body::from_stream(futures_util::stream::iter(chunks)).into_response()
}

async fn slow(State(s): State<Srv>, uri: OriginalUri) -> Response {
    s.hit(&uri);
    let stream = futures_util::stream::unfold(0u32, |i| async move {
        if i >= 400 {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        Some((Ok::<_, std::io::Error>(Bytes::from(vec![7u8; 1024])), i + 1))
    });
    Body::from_stream(stream).into_response()
}

async fn conc(State(s): State<Srv>, uri: OriginalUri) -> Response {
    s.hit(&uri);
    let now = s.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    s.max_in_flight.fetch_max(now, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(80)).await;
    s.in_flight.fetch_sub(1, Ordering::SeqCst);
    s.data.to_vec().into_response()
}

/// Acts as an HTTP forward proxy for plain-HTTP requests (absolute-form URI).
async fn via_proxy(State(s): State<Srv>, uri: OriginalUri, headers: HeaderMap) -> Response {
    s.hit(&uri);
    if let Some(v) = headers
        .get(header::PROXY_AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        s.proxy_auth.lock().unwrap().push(v.to_owned());
    }
    b"proxied".to_vec().into_response()
}

async fn start(data_len: usize) -> (Srv, SocketAddr) {
    let srv = Srv {
        data: Arc::new(payload(data_len)),
        ..Srv::default()
    };
    let app = Router::new()
        .route("/ok/{name}", get(ok))
        .route("/norange/{name}", get(norange))
        .route("/bad/{name}", get(bad))
        .route("/500/{name}", get(status500))
        .route("/404/{name}", get(status404))
        .route("/slow/{name}", get(slow))
        .route("/conc/{name}", get(conc))
        .route("/proxied/{name}", get(via_proxy))
        .route("/stream/{name}", get(chunked))
        .with_state(srv.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (srv, addr)
}

// ---------- helpers ----------

struct RecSink {
    id: TaskId,
    cancel: CancelToken,
    events: Mutex<Vec<TaskEvent>>,
}

impl RecSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            id: TaskId::new(),
            cancel: CancelToken::new(),
            events: Mutex::new(Vec::new()),
        })
    }

    /// (done, total) of the last progress event.
    fn last_progress(&self) -> (u64, Option<u64>) {
        self.events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|e| match e {
                TaskEvent::Progress { done, total, .. } => Some((*done, *total)),
                _ => None,
            })
            .unwrap()
    }

    fn progress_count(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, TaskEvent::Progress { .. }))
            .count()
    }
}

impl ProgressSink for RecSink {
    fn task_id(&self) -> TaskId {
        self.id
    }

    fn emit(&self, event: TaskEvent) {
        self.events.lock().unwrap().push(event);
    }

    fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
}

fn downloader() -> HttpDownloader {
    HttpDownloader::new(DownloaderConfig {
        attempts_per_url: 2,
        backoff_base: Duration::from_millis(5),
        connect_timeout: Duration::from_secs(5),
        read_timeout: Duration::from_secs(5),
        system_proxy: SystemProxyMode::Off,
        ..DownloaderConfig::default()
    })
    .unwrap()
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("lc-http-test-{tag}-{}", TaskId::new().0));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn url(addr: SocketAddr, path: &str) -> Url {
    Url::parse(&format!("http://{addr}{path}")).unwrap()
}

fn req(urls: Vec<Url>, data: &[u8], dest: PathBuf) -> DownloadReq {
    DownloadReq {
        urls,
        sha1: Some(sha1_hex(data)),
        size: Some(data.len() as u64),
        dest,
        kind: ResourceKind::Library,
    }
}

fn part_of(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

const LEN: usize = 300_000;

// ---------- tests ----------

#[tokio::test]
async fn staging_hardlink_never_modifies_its_other_name() {
    let (srv, addr) = start(LEN).await;
    // Exercise both resume and an upstream that ignores Range.
    for route in ["/ok/hardlink", "/norange/hardlink"] {
        let tmp = TempDir::new("hardlink");
        let dest = tmp.path().join("file.jar");
        let other = tmp.path().join("user-data");
        let prefix = &srv.data[..1234];
        std::fs::write(&other, prefix).unwrap();
        std::fs::hard_link(&other, part_of(&dest)).unwrap();
        downloader()
            .fetch_many(
                vec![req(vec![url(addr, route)], &srv.data, dest.clone())],
                &*RecSink::new(),
            )
            .await
            .unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), prefix);
        assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
    }
}

#[tokio::test]
async fn rejects_staging_directory_without_modifying_it() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("part-directory");
    let dest = tmp.path().join("file.jar");
    std::fs::create_dir(part_of(&dest)).unwrap();
    let marker = part_of(&dest).join("user-data");
    std::fs::write(&marker, b"keep").unwrap();
    assert!(downloader()
        .fetch_many(
            vec![req(vec![url(addr, "/ok/a")], &srv.data, dest.clone())],
            &*RecSink::new(),
        )
        .await
        .is_err());
    assert_eq!(std::fs::read(&marker).unwrap(), b"keep");
    assert!(!dest.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_staging_symlink_without_modifying_target() {
    let (srv, addr) = start(LEN).await;
    for existing in [false, true] {
        let tmp = TempDir::new("part-symlink");
        let dest = tmp.path().join("file.jar");
        let other = tmp.path().join("user-data");
        if existing {
            std::fs::write(&other, &srv.data[..1234]).unwrap();
        }
        std::os::unix::fs::symlink(&other, part_of(&dest)).unwrap();
        assert!(downloader()
            .fetch_many(
                vec![req(vec![url(addr, "/ok/a")], &srv.data, dest.clone())],
                &*RecSink::new(),
            )
            .await
            .is_err());
        if existing {
            assert_eq!(std::fs::read(&other).unwrap(), &srv.data[..1234]);
        } else {
            assert!(!other.exists());
        }
        assert!(!dest.exists());
    }
}

#[tokio::test]
async fn downloads_and_verifies_hash() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("plain");
    let dest = tmp.path().join("nested/dir/a.jar");
    let sink = RecSink::new();
    downloader()
        .fetch_many(
            vec![req(vec![url(addr, "/ok/a")], &srv.data, dest.clone())],
            &*sink,
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
    assert!(!part_of(&dest).exists());
    assert_eq!(sink.last_progress(), (LEN as u64, Some(LEN as u64)));
}

#[tokio::test]
async fn skips_existing_file() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("skip");
    let dest = tmp.path().join("b.jar");
    std::fs::write(&dest, &*srv.data).unwrap();
    let sink = RecSink::new();
    downloader()
        .fetch_many(
            vec![req(vec![url(addr, "/ok/b")], &srv.data, dest.clone())],
            &*sink,
        )
        .await
        .unwrap();
    assert_eq!(srv.hits("/ok/b"), 0);
    assert_eq!(sink.last_progress().0, LEN as u64);
}

#[tokio::test]
async fn redownloads_existing_file_with_wrong_hash() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("stale");
    let dest = tmp.path().join("b.jar");
    std::fs::write(&dest, vec![1u8; LEN]).unwrap();
    downloader()
        .fetch_many(
            vec![req(vec![url(addr, "/ok/b")], &srv.data, dest.clone())],
            &*RecSink::new(),
        )
        .await
        .unwrap();
    assert_eq!(srv.hits("/ok/b"), 1);
    assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
}

#[tokio::test]
async fn resumes_with_range() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("resume");
    let dest = tmp.path().join("c.jar");
    std::fs::write(part_of(&dest), &srv.data[..100_000]).unwrap();
    let sink = RecSink::new();
    downloader()
        .fetch_many(
            vec![req(vec![url(addr, "/ok/c")], &srv.data, dest.clone())],
            &*sink,
        )
        .await
        .unwrap();
    assert_eq!(
        *srv.ranges.lock().unwrap(),
        vec!["bytes=100000-".to_owned()]
    );
    assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
    assert!(!part_of(&dest).exists());
    assert_eq!(sink.last_progress().0, LEN as u64);
}

#[tokio::test]
async fn restarts_when_server_ignores_range() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("norange");
    let dest = tmp.path().join("d.jar");
    // Garbage prefix: if it were kept, the hash would not match.
    std::fs::write(part_of(&dest), vec![9u8; 50_000]).unwrap();
    let sink = RecSink::new();
    downloader()
        .fetch_many(
            vec![req(vec![url(addr, "/norange/d")], &srv.data, dest.clone())],
            &*sink,
        )
        .await
        .unwrap();
    assert_eq!(srv.hits("/norange/d"), 1);
    assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
    assert_eq!(sink.last_progress().0, LEN as u64);
}

#[tokio::test]
async fn hash_mismatch_is_reported_and_part_removed() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("bad");
    let dest = tmp.path().join("e.jar");
    let err = downloader()
        .fetch_many(
            vec![req(vec![url(addr, "/bad/e")], &srv.data, dest.clone())],
            &*RecSink::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::HashMismatch);
    let details = err.details.unwrap();
    assert!(details["url"].ends_with("/bad/e"));
    assert_eq!(details["expectedSha1"], sha1_hex(&srv.data));
    assert_eq!(
        srv.hits("/bad/e"),
        1,
        "bad body is not retried on the same URL"
    );
    assert!(!dest.exists());
    assert!(!part_of(&dest).exists());
}

#[tokio::test]
async fn falls_back_after_500() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("f500");
    let dest = tmp.path().join("f.jar");
    downloader()
        .fetch_many(
            vec![req(
                vec![url(addr, "/500/f"), url(addr, "/ok/f")],
                &srv.data,
                dest.clone(),
            )],
            &*RecSink::new(),
        )
        .await
        .unwrap();
    assert_eq!(srv.hits("/500/f"), 2, "5xx is retried");
    assert_eq!(srv.hits("/ok/f"), 1);
    assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
}

#[tokio::test]
async fn falls_back_after_404() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("f404");
    let dest = tmp.path().join("g.jar");
    downloader()
        .fetch_many(
            vec![req(
                vec![url(addr, "/404/g"), url(addr, "/ok/g")],
                &srv.data,
                dest.clone(),
            )],
            &*RecSink::new(),
        )
        .await
        .unwrap();
    assert_eq!(srv.hits("/404/g"), 1, "404 is not retried");
    assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
}

#[tokio::test]
async fn not_found_when_all_candidates_404() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("nf");
    let dest = tmp.path().join("h.jar");
    let err = downloader()
        .fetch_many(
            vec![req(vec![url(addr, "/404/h")], &srv.data, dest)],
            &*RecSink::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(err.details.unwrap()["url"].ends_with("/404/h"));
}

#[tokio::test]
async fn cancellation_stops_download_and_keeps_part() {
    let (_srv, addr) = start(LEN).await;
    let tmp = TempDir::new("cancel");
    let dest = tmp.path().join("i.jar");
    let sink = RecSink::new();
    let token = sink.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        token.cancel();
    });
    let started = std::time::Instant::now();
    let err = downloader()
        .fetch_many(
            vec![DownloadReq {
                urls: vec![url(addr, "/slow/i")],
                sha1: None,
                size: None,
                dest: dest.clone(),
                kind: ResourceKind::Other,
            }],
            &*sink,
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(!dest.exists());
    assert!(part_of(&dest).exists(), "partial data kept for resume");
    // ~300 ms of a slow stream: throttled events carrying a speed estimate.
    let count = sink.progress_count();
    assert!(count <= 10, "{count}");
    let events = sink.events.lock().unwrap();
    assert!(events.iter().any(|e| matches!(
        e,
        TaskEvent::Progress {
            speed_bps: Some(s),
            ..
        } if *s > 0
    )));
}

#[tokio::test]
async fn already_cancelled_does_nothing() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("precancel");
    let sink = RecSink::new();
    sink.cancel.cancel();
    let err = downloader()
        .fetch_many(
            vec![req(
                vec![url(addr, "/ok/j")],
                &srv.data,
                tmp.path().join("j"),
            )],
            &*sink,
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert_eq!(srv.hits("/ok/j"), 0);
}

#[tokio::test]
async fn progress_sums_all_files() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("many");
    // One file already present, one half-downloaded, the rest fresh.
    let reqs: Vec<DownloadReq> = (0..6)
        .map(|i| {
            req(
                vec![url(addr, &format!("/ok/m{i}"))],
                &srv.data,
                tmp.path().join(format!("m{i}.bin")),
            )
        })
        .collect();
    std::fs::write(&reqs[0].dest, &*srv.data).unwrap();
    std::fs::write(part_of(&reqs[1].dest), &srv.data[..1234]).unwrap();
    let sink = RecSink::new();
    downloader().fetch_many(reqs, &*sink).await.unwrap();
    let total = 6 * LEN as u64;
    assert_eq!(sink.last_progress(), (total, Some(total)));
    // Throttled: far fewer events than chunks.
    assert!(sink.progress_count() < 100, "{}", sink.progress_count());
}

#[tokio::test]
async fn per_host_limit_is_respected() {
    let (srv, addr) = start(1000).await;
    let tmp = TempDir::new("conc");
    let d = HttpDownloader::new(DownloaderConfig {
        max_parallel: 16,
        max_per_host: 2,
        system_proxy: SystemProxyMode::Off,
        ..DownloaderConfig::default()
    })
    .unwrap();
    let reqs = (0..6)
        .map(|i| {
            req(
                vec![url(addr, &format!("/conc/{i}"))],
                &srv.data,
                tmp.path().join(format!("{i}")),
            )
        })
        .collect();
    d.fetch_many(reqs, &*RecSink::new()).await.unwrap();
    let max = srv.max_in_flight.load(Ordering::SeqCst);
    assert!((1..=2).contains(&max), "max in flight = {max}");
}

#[tokio::test]
async fn get_bytes_falls_back_and_reports_not_found() {
    let (srv, addr) = start(LEN).await;
    let d = downloader();
    let body = d
        .get_bytes(&url(addr, "/ok/x"), ResourceKind::VersionMeta)
        .await
        .unwrap();
    assert_eq!(body, *srv.data);

    let err = d
        .get_bytes(&url(addr, "/404/x"), ResourceKind::VersionMeta)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    let err = d
        .get_bytes(&url(addr, "/500/y"), ResourceKind::VersionMeta)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Network);
    assert_eq!(srv.hits("/500/y"), 2);
}

#[tokio::test]
async fn connection_refused_is_network_error() {
    // Bind and drop to get a port that is (almost certainly) closed.
    let addr = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let err = downloader()
        .get_bytes(&url(addr, "/x"), ResourceKind::Other)
        .await
        .unwrap_err();
    assert!(
        matches!(err.code, ErrorCode::Network | ErrorCode::Blocked),
        "{err:?}"
    );
    assert!(err.details.unwrap().contains_key("url"));
}

/// No proxy and the origin unreachable: after the first failure the host is
/// demoted (mirror first for verified downloads, a single try otherwise),
/// and a total failure suggests a proxy.
#[tokio::test]
async fn unreachable_host_is_demoted_and_hinted() {
    let (srv, addr) = start(1000).await;
    // Accepts and drops every connection: a reset-style block.
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let acc = accepts.clone();
    tokio::spawn(async move {
        while let Ok((s, _)) = dead.accept().await {
            acc.fetch_add(1, Ordering::SeqCst);
            drop(s);
        }
    });
    let d = downloader();
    let tmp = TempDir::new("demote");
    let urls = |name: &str| {
        vec![
            url(dead_addr, &format!("/{name}")),
            url(addr, &format!("/ok/{name}")),
        ]
    };
    let sink = RecSink::new();
    d.fetch_many(
        vec![req(urls("a"), &srv.data, tmp.path().join("a"))],
        &*sink,
    )
    .await
    .unwrap();
    let first = accepts.load(Ordering::SeqCst);
    assert!(first >= 1);
    // Verified: the mirror goes first now, the dead origin isn't touched.
    d.fetch_many(
        vec![req(urls("b"), &srv.data, tmp.path().join("b"))],
        &*sink,
    )
    .await
    .unwrap();
    assert_eq!(accepts.load(Ordering::SeqCst), first);
    assert_eq!(srv.hits("/ok/b"), 1);

    // Unverified: original first, but only one attempt.
    let err = d
        .get_bytes(&url(dead_addr, "/x"), ResourceKind::Other)
        .await
        .unwrap_err();
    assert_eq!(accepts.load(Ordering::SeqCst), first + 1);
    assert!(
        matches!(err.code, ErrorCode::Network | ErrorCode::Blocked),
        "{err:?}"
    );
    assert_eq!(err.details.unwrap()["hint"], "useProxy");

    // A plain HTTP error is not a reason to suggest a proxy.
    let err = d
        .get_bytes(&url(addr, "/404/x"), ResourceKind::Other)
        .await
        .unwrap_err();
    assert!(!err.details.unwrap().contains_key("hint"));
}

#[tokio::test]
async fn manual_http_proxy_is_used_with_auth() {
    let (srv, addr) = start(10).await;
    let d = downloader();
    d.set_manual_proxy(Some(ProxyCfg {
        kind: ProxyKind::Http,
        host: addr.ip().to_string(),
        port: addr.port(),
        username: Some("alice".into()),
        password: Some("s3cret".into()),
    }));
    let body = d
        .get_bytes(
            &Url::parse("http://upstream.invalid/proxied/z").unwrap(),
            ResourceKind::Other,
        )
        .await
        .unwrap();
    assert_eq!(body, b"proxied");
    let auth = srv.proxy_auth.lock().unwrap().clone();
    // base64("alice:s3cret")
    assert_eq!(auth, vec!["Basic YWxpY2U6czNjcmV0".to_owned()]);

    d.set_manual_proxy(None);
    let body = d
        .get_bytes(&url(addr, "/ok/direct"), ResourceKind::Other)
        .await
        .unwrap();
    assert_eq!(body.len(), 10);
}

#[tokio::test]
async fn duplicate_dest_in_one_batch() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("dup");
    let dest = tmp.path().join("dup.jar");
    let sink = RecSink::new();
    downloader()
        .fetch_many(
            vec![
                req(vec![url(addr, "/conc/dup")], &srv.data, dest.clone()),
                req(vec![url(addr, "/conc/dup")], &srv.data, dest.clone()),
            ],
            &*sink,
        )
        .await
        .unwrap();
    assert_eq!(srv.hits("/conc/dup"), 1);
    assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
    assert_eq!(sink.last_progress(), (LEN as u64, Some(LEN as u64)));
}

#[tokio::test]
async fn concurrent_fetch_many_same_dest() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("cdup");
    let dest = tmp.path().join("same.jar");
    let d = downloader();
    let (s1, s2) = (RecSink::new(), RecSink::new());
    let mk = || vec![req(vec![url(addr, "/conc/same")], &srv.data, dest.clone())];
    let (a, b) = tokio::join!(d.fetch_many(mk(), &*s1), d.fetch_many(mk(), &*s2));
    a.unwrap();
    b.unwrap();
    // The second call waits for the first and then skips the verified file.
    assert_eq!(srv.hits("/conc/same"), 1);
    assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
    assert!(!part_of(&dest).exists());
    assert_eq!(s1.last_progress().0, LEN as u64);
    assert_eq!(s2.last_progress().0, LEN as u64);
}

#[tokio::test]
async fn unhashed_part_not_resumed_blindly() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("unhashed");
    let dest = tmp.path().join("u.json");
    std::fs::write(part_of(&dest), vec![9u8; 50_000]).unwrap();
    downloader()
        .fetch_many(
            vec![DownloadReq {
                urls: vec![url(addr, "/ok/u")],
                sha1: None,
                size: None,
                dest: dest.clone(),
                kind: ResourceKind::VersionMeta,
            }],
            &*RecSink::new(),
        )
        .await
        .unwrap();
    assert!(srv.ranges.lock().unwrap().is_empty(), "no Range request");
    assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
}

#[tokio::test]
async fn hash_mismatch_moves_to_next_candidate() {
    let (srv, addr) = start(LEN).await;
    let tmp = TempDir::new("mm-next");
    let dest = tmp.path().join("k.jar");
    downloader()
        .fetch_many(
            vec![req(
                vec![url(addr, "/bad/k"), url(addr, "/ok/k")],
                &srv.data,
                dest.clone(),
            )],
            &*RecSink::new(),
        )
        .await
        .unwrap();
    assert_eq!(srv.hits("/bad/k"), 1);
    assert_eq!(srv.hits("/ok/k"), 1);
    assert_eq!(std::fs::read(&dest).unwrap(), *srv.data);
}

#[tokio::test]
async fn get_bytes_rejects_oversized_body() {
    let (srv, addr) = start(LEN).await;
    let d = HttpDownloader::new(DownloaderConfig {
        max_body_bytes: 1000,
        attempts_per_url: 2,
        backoff_base: Duration::from_millis(5),
        system_proxy: SystemProxyMode::Off,
        ..DownloaderConfig::default()
    })
    .unwrap();
    // With Content-Length.
    let err = d
        .get_bytes(&url(addr, "/ok/big"), ResourceKind::Other)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Network);
    assert!(err.details.unwrap()["url"].ends_with("/ok/big"));
    // Chunked, counted while streaming.
    let err = d
        .get_bytes(&url(addr, "/stream/big"), ResourceKind::Other)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Network);
    assert!(err.details.unwrap()["url"].ends_with("/stream/big"));
    assert_eq!(srv.hits("/stream/big"), 1, "oversize is not retried");

    // Default limit (32 MiB) accepts the same bodies.
    let body = downloader()
        .get_bytes(&url(addr, "/stream/ok"), ResourceKind::Other)
        .await
        .unwrap();
    assert_eq!(body, *srv.data);
}
