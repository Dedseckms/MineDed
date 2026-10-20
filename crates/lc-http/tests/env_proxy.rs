//! `proxy: None` must mean direct even when `HTTP_PROXY` is set.
//! Lives in its own test binary (one process, one test) because it mutates
//! the process environment.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::Router;
use lc_http::{Downloader, DownloaderConfig, HttpDownloader, ResourceKind};
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
async fn none_proxy_ignores_env() {
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

    let _guard = EnvGuard::set("HTTP_PROXY", &format!("http://{addr}"));
    let _guard_lc = EnvGuard::set("http_proxy", &format!("http://{addr}"));
    let target = Url::parse("http://upstream.invalid/x").unwrap();

    // Control: a default reqwest client honours the variable, so the test is meaningful.
    let via_env = reqwest::Client::new()
        .get(target.clone())
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(via_env, "via-proxy");
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let d = HttpDownloader::new(DownloaderConfig {
        attempts_per_url: 1,
        ..DownloaderConfig::default()
    })
    .unwrap();
    let err = d.get_bytes(&target, ResourceKind::Other).await.unwrap_err();
    assert!(err.details.unwrap().contains_key("url"));
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "downloader must not use env proxy"
    );
}
