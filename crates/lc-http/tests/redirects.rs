//! Redirect handling for clients carrying custom credential headers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{routing::get, Router};
use lc_http::{Downloader, DownloaderConfig, HttpDownloader, Route, SystemProxyMode};
use lc_types::settings::{ProxyCfg, ProxyKind};
use url::Url;

async fn server() -> (Url, Arc<Mutex<Vec<HeaderMap>>>, tokio::task::JoinHandle<()>) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let capture = received.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let destination = base.join("destination").unwrap().to_string();
    let app = Router::new()
        .route(
            "/destination",
            get(move |headers: HeaderMap| {
                let capture = capture.clone();
                async move {
                    capture.lock().unwrap().push(headers);
                    StatusCode::OK
                }
            }),
        )
        .fallback(move || {
            let destination = destination.clone();
            async move { (StatusCode::FOUND, [(header::LOCATION, destination)]).into_response() }
        });
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, received, task)
}

fn config() -> DownloaderConfig {
    DownloaderConfig {
        system_proxy: SystemProxyMode::Off,
        ..DownloaderConfig::default()
    }
}

#[tokio::test]
async fn no_redirect_clients_keep_custom_credentials_on_initial_request() {
    let (base, received, server) = server().await;
    let downloader = HttpDownloader::new_no_redirects(config()).unwrap();
    let url = base.join("redirect").unwrap();
    let check = |client: reqwest::Client| {
        let url = url.clone();
        async move {
            let response = client
                .get(url)
                .header("x-api-key", "test-only-credential")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FOUND);
        }
    };
    check(downloader.client(Route::Direct)).await;
    check(downloader.client(Route::Auto)).await;
    downloader.set_manual_proxy(Some(ProxyCfg {
        kind: ProxyKind::Http,
        host: "127.0.0.1".into(),
        port: base.port().unwrap(),
        username: None,
        password: None,
    }));
    check(downloader.client(Route::Proxied)).await;
    check(downloader.client(Route::Auto)).await;
    downloader.set_vpn_proxy(Some(base.clone()));
    check(downloader.client(Route::Proxied)).await;
    downloader.set_vpn_proxy(None);
    check(downloader.client(Route::Proxied)).await;
    downloader.set_manual_proxy(None);
    check(downloader.client(Route::Auto)).await;
    assert!(received.lock().unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn default_constructor_still_follows_redirects() {
    let (base, received, server) = server().await;
    let downloader = HttpDownloader::new(config()).unwrap();
    let response = downloader
        .client(Route::Direct)
        .get(base.join("redirect").unwrap())
        .header("x-api-key", "test-only-credential")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let headers = received.lock().unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0]["x-api-key"], "test-only-credential");
    server.abort();
}
