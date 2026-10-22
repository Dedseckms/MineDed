//! Environment proxy (`HTTP(S)_PROXY`) is the OS-level fallback: never used by
//! `Route::Direct`, used by `Route::Auto` only after the direct attempt failed.
//! Lives in its own test binary (one process, one test) because it mutates
//! the process environment.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::Router;
use lc_http::{Downloader, DownloaderConfig, HttpDownloader, ResourceKind, Route};
use url::Url;

/// Restores the previous value of an environment variable on drop.
struct EnvGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

#[tokio::test]
async fn env_proxy_is_auto_fallback_only() {
    // A fake forward proxy that counts every request it sees.
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let app = Router::new().fallback(move || {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            "via-proxy"
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let proxy = format!("http://{addr}");
    let _g = [
        EnvGuard::set("HTTP_PROXY", &proxy),
        EnvGuard::set("http_proxy", &proxy),
        EnvGuard::set("HTTPS_PROXY", &proxy),
        EnvGuard::set("https_proxy", &proxy),
        EnvGuard::set("NO_PROXY", ""),
        EnvGuard::set("no_proxy", ""),
    ];
    let target = Url::parse("http://upstream.invalid/x").unwrap();

    let d = HttpDownloader::new(DownloaderConfig {
        attempts_per_url: 1,
        ..DownloaderConfig::default()
    })
    .unwrap();

    // Direct never consults the environment.
    let err = d
        .get_bytes_via(&target, ResourceKind::Other, Route::Direct)
        .await
        .unwrap_err();
    assert!(err.details.unwrap().contains_key("url"));
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "direct must not use env proxy"
    );

    // Auto: direct fails (unresolvable host), then the env proxy answers.
    let body = d.get_bytes(&target, ResourceKind::Other).await.unwrap();
    assert_eq!(body, b"via-proxy");
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    // Proxied goes straight to it.
    let body = d
        .get_bytes_via(&target, ResourceKind::Other, Route::Proxied)
        .await
        .unwrap();
    assert_eq!(body, b"via-proxy");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
