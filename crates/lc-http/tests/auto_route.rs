//! `Route::Auto` / `Route::Proxied` against local "blocked" hosts and a local
//! HTTP forward proxy (no external network).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{OriginalUri, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Router;
use lc_http::{
    DownloadReq, Downloader, DownloaderConfig, HttpDownloader, ResourceKind, Route, SystemProxyMode,
};
use lc_runtime::{CancelToken, ProgressSink};
use lc_types::settings::{ProxyCfg, ProxyKind};
use lc_types::task::TaskEvent;
use lc_types::{ErrorCode, TaskId};
use sha1::{Digest, Sha1};
use url::Url;

const JSON: &[u8] = br#"{"ok":true}"#;
const BLOCK_PAGE: &str = "<!DOCTYPE html><html><body>Access denied</body></html>";

fn payload() -> Vec<u8> {
    (0..50_000u32).map(|i| (i * 7 % 251) as u8).collect()
}

// ---------- servers ----------

#[derive(Clone, Default)]
struct Hits(Arc<AtomicUsize>);

impl Hits {
    fn get(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
    fn inc(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Blocked host that answers every request with an ISP block page.
/// `/typed/..` sends `Content-Type: text/html`, everything else sniffs as HTML
/// only by its body.
async fn html_host() -> (Hits, SocketAddr) {
    let hits = Hits::default();
    let h = hits.clone();
    let app = Router::new().fallback(move |uri: OriginalUri| {
        let h = h.clone();
        async move {
            h.inc();
            if uri.0.path().starts_with("/typed") {
                ([(header::CONTENT_TYPE, "text/html")], BLOCK_PAGE).into_response()
            } else {
                (
                    [(header::CONTENT_TYPE, "application/octet-stream")],
                    BLOCK_PAGE,
                )
                    .into_response()
            }
        }
    });
    serve(app, hits).await
}

/// Blocked host that drops every connection right after accepting it.
async fn reset_host() -> (Hits, SocketAddr) {
    let hits = Hits::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let h = hits.clone();
    tokio::spawn(async move {
        loop {
            let (s, _) = listener.accept().await.unwrap();
            h.inc();
            drop(s);
        }
    });
    (hits, addr)
}

/// Blocked host that accepts connections and never answers.
async fn hanging_host() -> (Hits, SocketAddr) {
    let hits = Hits::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let h = hits.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            let (s, _) = listener.accept().await.unwrap();
            h.inc();
            held.push(s);
        }
    });
    (hits, addr)
}

#[derive(Clone, Default)]
struct ProxySrv {
    hits: Hits,
    /// Absolute-form request targets seen by the proxy.
    targets: Arc<Mutex<Vec<String>>>,
}

/// HTTP forward proxy: serves what the "real" upstream would return
/// (`*.json` → JSON, anything else → `payload()`).
async fn proxy_server() -> (ProxySrv, SocketAddr) {
    let srv = ProxySrv::default();
    let app = Router::new()
        .fallback(|State(s): State<ProxySrv>, uri: OriginalUri| async move {
            s.hits.inc();
            s.targets.lock().unwrap().push(uri.0.to_string());
            if uri.0.path().ends_with(".json") {
                ([(header::CONTENT_TYPE, "application/json")], JSON.to_vec()).into_response()
            } else {
                payload().into_response()
            }
        })
        .with_state(srv.clone());
    let (_, addr) = serve(app, Hits::default()).await;
    (srv, addr)
}

async fn serve(app: Router, hits: Hits) -> (Hits, SocketAddr) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (hits, addr)
}

// ---------- helpers ----------

fn downloader() -> HttpDownloader {
    HttpDownloader::new(DownloaderConfig {
        attempts_per_url: 2,
        backoff_base: Duration::from_millis(5),
        connect_timeout: Duration::from_secs(2),
        read_timeout: Duration::from_millis(500),
        request_timeout: Duration::from_millis(500),
        system_proxy: SystemProxyMode::Off,
        ..DownloaderConfig::default()
    })
    .unwrap()
}

fn with_proxy(addr: SocketAddr) -> HttpDownloader {
    let d = downloader();
    d.set_manual_proxy(Some(ProxyCfg {
        kind: ProxyKind::Http,
        host: addr.ip().to_string(),
        port: addr.port(),
        username: Some("bob".into()),
        password: Some("pr0xy-secret".into()),
    }));
    d
}

fn url(addr: SocketAddr, path: &str) -> Url {
    Url::parse(&format!("http://{addr}{path}")).unwrap()
}

struct Sink {
    id: TaskId,
    cancel: CancelToken,
}

impl ProgressSink for Sink {
    fn task_id(&self) -> TaskId {
        self.id
    }
    fn emit(&self, _event: TaskEvent) {}
    fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
}

fn sink() -> Sink {
    Sink {
        id: TaskId::new(),
        cancel: CancelToken::new(),
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("lc-http-route-{tag}-{}", TaskId::new().0));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------- tests ----------

#[tokio::test]
async fn spoofed_html_falls_back_and_host_is_remembered() {
    let (blocked, b_addr) = html_host().await;
    let (proxy, p_addr) = proxy_server().await;
    let d = with_proxy(p_addr);

    for path in ["/typed/v.json", "/body/v.json"] {
        let body = d
            .get_bytes(&url(b_addr, path), ResourceKind::VersionMeta)
            .await
            .unwrap();
        assert_eq!(body, JSON, "{path}");
    }
    // First request: one direct hit (spoofed → no same-route retry), then proxy.
    // Second request: the host is already "proxy first".
    assert_eq!(blocked.get(), 1);
    assert_eq!(proxy.hits.get(), 2);
    assert!(proxy.targets.lock().unwrap()[0].starts_with(&format!("http://{b_addr}/")));

    // The `Auto` client honours the memory as well.
    let text = d
        .client(Route::Auto)
        .get(url(b_addr, "/x.json"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&text[..], JSON);
    assert_eq!(blocked.get(), 1);
    assert_eq!(proxy.hits.get(), 3);
    // ...while `Direct` never uses the proxy.
    let direct = d
        .client(Route::Direct)
        .get(url(b_addr, "/x.json"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(direct, BLOCK_PAGE);
    assert_eq!(blocked.get(), 2);
}

#[tokio::test]
async fn memory_expires() {
    let (blocked, b_addr) = html_host().await;
    let (proxy, p_addr) = proxy_server().await;
    let d = HttpDownloader::new(DownloaderConfig {
        attempts_per_url: 1,
        proxy_first_ttl: Duration::from_millis(100),
        proxy: Some(ProxyCfg {
            kind: ProxyKind::Http,
            host: p_addr.ip().to_string(),
            port: p_addr.port(),
            username: None,
            password: None,
        }),
        system_proxy: SystemProxyMode::Off,
        ..DownloaderConfig::default()
    })
    .unwrap();
    let u = url(b_addr, "/typed/a.json");
    d.get_bytes(&u, ResourceKind::Content).await.unwrap();
    d.get_bytes(&u, ResourceKind::Content).await.unwrap();
    assert_eq!((blocked.get(), proxy.hits.get()), (1, 2));
    tokio::time::sleep(Duration::from_millis(150)).await;
    d.get_bytes(&u, ResourceKind::Content).await.unwrap();
    assert_eq!((blocked.get(), proxy.hits.get()), (2, 3));
}

#[tokio::test]
async fn html_is_accepted_for_other_kind() {
    let (blocked, b_addr) = html_host().await;
    let (proxy, p_addr) = proxy_server().await;
    let d = with_proxy(p_addr);
    let body = d
        .get_bytes(&url(b_addr, "/typed/page"), ResourceKind::Other)
        .await
        .unwrap();
    assert_eq!(body, BLOCK_PAGE.as_bytes());
    assert_eq!((blocked.get(), proxy.hits.get()), (1, 0));
}

#[tokio::test]
async fn connection_reset_falls_back() {
    let (blocked, b_addr) = reset_host().await;
    let (proxy, p_addr) = proxy_server().await;
    let d = with_proxy(p_addr);
    let body = d
        .get_bytes(&url(b_addr, "/meta.json"), ResourceKind::Loader)
        .await
        .unwrap();
    assert_eq!(body, JSON);
    // Transport failure with a proxy available: no same-route retry.
    assert_eq!((blocked.get(), proxy.hits.get()), (1, 1));
}

#[tokio::test]
async fn timeout_falls_back() {
    let (blocked, b_addr) = hanging_host().await;
    let (proxy, p_addr) = proxy_server().await;
    let d = with_proxy(p_addr);
    let started = std::time::Instant::now();
    let body = d
        .get_bytes(&url(b_addr, "/meta.json"), ResourceKind::VersionMeta)
        .await
        .unwrap();
    assert_eq!(body, JSON);
    assert_eq!((blocked.get(), proxy.hits.get()), (1, 1));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn fetch_many_falls_back_on_hash_mismatch_and_spoof() {
    let (blocked, b_addr) = html_host().await;
    let (proxy, p_addr) = proxy_server().await;
    let tmp = TempDir::new("fetch");
    let data = payload();
    let d = with_proxy(p_addr);
    let reqs = vec![
        // Hash-checked: the block page fails verification.
        DownloadReq {
            urls: vec![url(b_addr, "/lib.jar")],
            sha1: Some(hex::encode(Sha1::digest(&data))),
            size: Some(data.len() as u64),
            dest: tmp.0.join("lib.jar"),
            kind: ResourceKind::Library,
        },
    ];
    d.fetch_many(reqs, &sink()).await.unwrap();
    assert_eq!(std::fs::read(tmp.0.join("lib.jar")).unwrap(), data);
    assert_eq!((blocked.get(), proxy.hits.get()), (1, 1));

    // Unhashed content on another host: the HTML body is caught by sniffing.
    let (blocked2, b2_addr) = html_host().await;
    let reqs = vec![DownloadReq {
        urls: vec![url(b2_addr, "/mod.jar")],
        sha1: None,
        size: None,
        dest: tmp.0.join("mod.jar"),
        kind: ResourceKind::Content,
    }];
    d.fetch_many(reqs, &sink()).await.unwrap();
    assert_eq!(std::fs::read(tmp.0.join("mod.jar")).unwrap(), data);
    assert_eq!((blocked2.get(), proxy.hits.get()), (1, 2));
}

#[tokio::test]
async fn auto_without_proxy_behaves_like_direct() {
    let (blocked, b_addr) = html_host().await;
    let d = downloader();
    let err = d
        .get_bytes(&url(b_addr, "/typed/v.json"), ResourceKind::VersionMeta)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Blocked);
    assert!(err.details.unwrap().contains_key("url"));
    // Detected by the body alone (no `text/html` header).
    let err = d
        .get_bytes(&url(b_addr, "/body/v.json"), ResourceKind::Content)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Blocked);
    assert_eq!(blocked.get(), 2);

    let (reset, r_addr) = reset_host().await;
    let err = d
        .get_bytes(&url(r_addr, "/v.json"), ResourceKind::VersionMeta)
        .await
        .unwrap_err();
    assert!(
        matches!(err.code, ErrorCode::Network | ErrorCode::Blocked),
        "{err:?}"
    );
    // Without a fallback route the usual retries apply.
    assert_eq!(reset.get(), 2);
}

#[tokio::test]
async fn proxied_without_proxy_is_an_error() {
    let (blocked, b_addr) = html_host().await;
    let d = downloader();
    let u = url(b_addr, "/v.json");
    let err = d
        .get_bytes_via(&u, ResourceKind::VersionMeta, Route::Proxied)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Network);
    assert!(err.message.contains("no proxy"), "{}", err.message);
    assert!(d.client(Route::Proxied).get(u).send().await.is_err());
    assert_eq!(blocked.get(), 0);
}

#[tokio::test]
async fn proxied_uses_proxy_only() {
    let (blocked, b_addr) = html_host().await;
    let (proxy, p_addr) = proxy_server().await;
    let d = with_proxy(p_addr);
    let body = d
        .get_bytes_via(
            &url(b_addr, "/v.json"),
            ResourceKind::VersionMeta,
            Route::Proxied,
        )
        .await
        .unwrap();
    assert_eq!(body, JSON);
    assert_eq!((blocked.get(), proxy.hits.get()), (0, 1));
}

#[tokio::test]
async fn proxy_password_not_in_errors() {
    let (_, b_addr) = reset_host().await;
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let d = with_proxy(closed);
    let err = d
        .get_bytes(&url(b_addr, "/v.json"), ResourceKind::VersionMeta)
        .await
        .unwrap_err();
    assert!(!format!("{err:?}").contains("pr0xy-secret"), "{err:?}");
}

/// Forward proxy that answers every request with its own name.
async fn named_proxy(name: &'static str) -> (Hits, SocketAddr) {
    let hits = Hits::default();
    let h = hits.clone();
    let app = Router::new().fallback(move || {
        let h = h.clone();
        async move {
            h.inc();
            name
        }
    });
    serve(app, hits).await
}

#[tokio::test]
async fn proxy_priority_vpn_then_manual_then_system() {
    let (vpn_hits, vpn) = named_proxy("vpn").await;
    let (_, manual) = named_proxy("manual").await;
    let (_, system) = named_proxy("system").await;
    let d = HttpDownloader::new(DownloaderConfig {
        attempts_per_url: 1,
        system_proxy: SystemProxyMode::Fixed(Url::parse(&format!("http://{system}")).unwrap()),
        ..DownloaderConfig::default()
    })
    .unwrap();
    // Not resolvable: direct always fails, so `Auto` reaches the proxy leg.
    let target = Url::parse("http://upstream.invalid/v").unwrap();
    let via = |route| {
        let (d, target) = (&d, target.clone());
        async move {
            let body = d
                .get_bytes_via(&target, ResourceKind::Other, route)
                .await
                .unwrap_or_else(|e| panic!("{route:?} {e:?}"));
            String::from_utf8(body).unwrap()
        }
    };
    assert_eq!(via(Route::Proxied).await, "system");
    assert_eq!(via(Route::Auto).await, "system");

    d.set_manual_proxy(Some(ProxyCfg {
        kind: ProxyKind::Http,
        host: manual.ip().to_string(),
        port: manual.port(),
        username: None,
        password: None,
    }));
    assert_eq!(via(Route::Proxied).await, "manual");

    d.set_vpn_proxy(Some(Url::parse(&format!("http://{vpn}")).unwrap()));
    assert_eq!(via(Route::Proxied).await, "vpn");
    assert_eq!(via(Route::Auto).await, "vpn");
    // The `Proxied` client follows the switch as well.
    let text = d
        .client(Route::Proxied)
        .get(target.clone())
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(text, "vpn");
    assert_eq!(vpn_hits.get(), 3);

    // Malformed VPN URL is rejected; the previous route stays.
    d.set_vpn_proxy(Some(Url::parse("ftp://127.0.0.1:1").unwrap()));
    assert_eq!(via(Route::Proxied).await, "vpn");

    d.set_vpn_proxy(None);
    assert_eq!(via(Route::Proxied).await, "manual");
    d.set_manual_proxy(None);
    assert_eq!(via(Route::Proxied).await, "system");

    // Local targets never go through the OS proxy.
    let (local_hits, local) = html_host().await;
    let err = d
        .get_bytes_via(
            &url(local, "/v.json"),
            ResourceKind::VersionMeta,
            Route::Auto,
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Blocked);
    assert_eq!(local_hits.get(), 1);
    // ...and `Proxied` to such a target goes direct instead of failing.
    let body = d
        .get_bytes_via(&url(local, "/page"), ResourceKind::Other, Route::Proxied)
        .await
        .unwrap();
    assert_eq!(body, BLOCK_PAGE.as_bytes());
    assert_eq!(local_hits.get(), 2);
}
