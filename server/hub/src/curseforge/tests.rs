#![allow(clippy::unwrap_used)]

use super::*;
use axum::{body::Body, http::Request, Router};
use http_body_util::BodyExt;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tower::ServiceExt;

const TEST_KEY: &str = "test-only-credential-never-real";

async fn mock<F, Fut>(handler: F) -> (Broker, Arc<AtomicUsize>, tokio::task::JoinHandle<()>)
where
    F: Fn(Request<Body>) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = Response> + Send + 'static,
{
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let app = Router::new().fallback(move |request: Request<Body>| {
        let handler = handler.clone();
        counter.fetch_add(1, Ordering::Relaxed);
        async move {
            assert_eq!(request.headers()["x-api-key"], TEST_KEY);
            handler(request).await
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut broker = Broker::new(Secret::new(TEST_KEY)).unwrap();
    broker.upstream = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (broker, hits, task)
}

fn project(allowed: bool) -> Value {
    json!({"data":{"id":1,"gameId":432,"allowModDistribution":allowed}})
}
fn file() -> Value {
    json!({"id":2,"modId":1,"gameId":432,"downloadUrl":"https://edge.forgecdn.net/files/a.jar"})
}

async fn app(broker: Option<Broker>) -> Router {
    let mut state = AppState::new(
        crate::Config::default(),
        Arc::new(crate::auth::DraslValidator::new("http://127.0.0.1:9")),
    );
    if let Some(broker) = broker {
        state = state.with_curseforge(broker);
    }
    crate::router(state)
}
async fn call(app: &Router, method: Method, path: &str, credential: bool) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("{PREFIX}{path}"));
    if credential {
        request = request.header("x-api-key", "untrusted-client-key");
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(!body.to_string().contains(TEST_KEY));
    (status, body)
}

#[tokio::test]
async fn allowlist_rejects_methods_paths_queries_and_client_credentials() {
    let app = app(None).await;
    for path in [
        "/games",
        "/mods/0",
        "/mods/-1",
        "/mods/2147483648",
        "/mods/1/description",
        "/mods/search?url=http://localhost",
        "/mods/search?gameId=1",
        "/mods/search?gameId=432&gameId=432",
        "/mods/search?pageSize=51",
        "/mods/search?index=9951",
        "/mods/search?modLoaderType=2",
        "/mods/search?sortOrder=sideways",
        "/mods/1?gameId=432",
        "/mods/1/files?slug=test",
        "/mods/search?%78-api-key=abc",
        "/mods/search?gameVersion=%0A",
    ] {
        let (status, body) = call(&app, Method::GET, path, false).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert_eq!(body["code"], "invalidInput");
    }
    for method in [
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
        Method::OPTIONS,
    ] {
        assert_eq!(
            call(&app, method, "/mods/1", false).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        call(&app, Method::GET, "/mods/1", true).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&app, Method::GET, "/mods/1", false).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn all_six_routes_preserve_envelopes_and_force_minecraft() {
    let (broker, hits, task) = mock(|request| async move {
        let path = request.uri().path();
        let value = match path {
            "/mods/search" => {
                assert_eq!(request.uri().query(), Some("gameId=432"));
                json!({"data":[project(true)["data"].clone()],"pagination":{"index":0}})
            }
            "/mods/1" => project(true),
            "/mods/1/files" => json!({"data":[file()]}),
            "/mods/1/files/2" => json!({"data":file()}),
            "/mods/1/files/2/download-url" => {
                json!({"data":"https://edge.forgecdn.net/files/a.jar"})
            }
            "/mods/files" => {
                assert_eq!(request.method(), Method::POST);
                let body = request.into_body().collect().await.unwrap().to_bytes();
                assert_eq!(
                    serde_json::from_slice::<Value>(&body).unwrap(),
                    json!({"fileIds":[2]})
                );
                json!({"data":[file()]})
            }
            _ => panic!("unexpected upstream path"),
        };
        axum::Json(value).into_response()
    })
    .await;
    let app = app(Some(broker)).await;
    for path in [
        "/mods/search",
        "/mods/1",
        "/mods/1/files",
        "/mods/1/files/2",
        "/files/2",
        "/mods/1/files/2/download-url",
    ] {
        let (status, value) = call(&app, Method::GET, path, false).await;
        assert_eq!(status, StatusCode::OK, "{path}: {value}");
        assert!(value.get("data").is_some());
    }
    let before = hits.load(Ordering::Relaxed);
    assert_eq!(
        call(&app, Method::GET, "/mods/1", false).await.0,
        StatusCode::OK
    );
    assert_eq!(hits.load(Ordering::Relaxed), before);
    task.abort();
}

#[tokio::test]
async fn cross_game_and_wrong_file_ownership_fail_closed() {
    for wrong in [
        json!({"id":2,"gameId":1,"modId":1}),
        json!({"id":2,"gameId":432,"modId":7}),
        json!({"id":3,"gameId":432,"modId":1}),
    ] {
        let (broker, _, task) = mock(move |request| {
            let wrong = wrong.clone();
            async move {
                axum::Json(if request.uri().path() == "/mods/1" {
                    project(true)
                } else {
                    json!({"data":wrong})
                })
                .into_response()
            }
        })
        .await;
        assert_eq!(
            call(
                &app(Some(broker)).await,
                Method::GET,
                "/mods/1/files/2",
                false
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        task.abort();
    }
    let (broker, _, task) =
        mock(|_| async { axum::Json(json!({"data":{"id":1,"gameId":99}})).into_response() }).await;
    assert_eq!(
        call(&app(Some(broker)).await, Method::GET, "/mods/1", false)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    task.abort();
}

#[tokio::test]
async fn distribution_denied_and_null_urls_never_resolve_downloads() {
    for denied in [true, false] {
        let (broker, hits, task) = mock(move |request| async move {
            if request.uri().path() == "/mods/1" {
                axum::Json(project(!denied)).into_response()
            } else {
                let mut f = file();
                if !denied {
                    f["downloadUrl"] = Value::Null;
                }
                axum::Json(json!({"data":f})).into_response()
            }
        })
        .await;
        assert_eq!(
            call(
                &app(Some(broker)).await,
                Method::GET,
                "/mods/1/files/2/download-url",
                false
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(hits.load(Ordering::Relaxed), 2);
        task.abort();
    }
}

#[tokio::test]
async fn upstream_errors_redirects_and_secret_echo_are_sanitized() {
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::TEMPORARY_REDIRECT,
    ] {
        let (broker, hits, task) = mock(move |_| async move {
            (
                status,
                [
                    ("location", "http://127.0.0.1:9/stolen"),
                    ("x-api-key", TEST_KEY),
                ],
                TEST_KEY,
            )
                .into_response()
        })
        .await;
        let (status, body) = call(&app(Some(broker)).await, Method::GET, "/mods/1", false).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["message"], "catalogue upstream unavailable");
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        task.abort();
    }
    let (broker, _, task) = mock(|_| async {
        axum::Json(json!({"data":{"id":1,"gameId":432,"name":TEST_KEY}})).into_response()
    })
    .await;
    assert_eq!(
        call(&app(Some(broker)).await, Method::GET, "/mods/1", false)
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    task.abort();
    assert!(!format!("{:?}", Secret::new(TEST_KEY)).contains(TEST_KEY));
}

#[tokio::test]
async fn streaming_cap_applies_without_content_length_and_malformed_json_fails() {
    for oversized in [true, false] {
        let (broker, _, task) = mock(move |_| async move {
            let bytes = if oversized {
                vec![b' '; BODY_LIMIT + 1]
            } else {
                b"not JSON".to_vec()
            };
            let stream = futures_util::stream::iter(
                bytes
                    .chunks(4096)
                    .map(|b| Ok::<_, std::io::Error>(axum::body::Bytes::copy_from_slice(b)))
                    .collect::<Vec<_>>(),
            );
            Body::from_stream(stream).into_response()
        })
        .await;
        assert_eq!(
            call(&app(Some(broker)).await, Method::GET, "/mods/1", false)
                .await
                .0,
            StatusCode::BAD_GATEWAY
        );
        task.abort();
    }
}

#[tokio::test]
async fn rate_and_concurrency_limits_apply_before_upstream_work() {
    let (broker, hits, task) = mock(|_| async { axum::Json(project(true)).into_response() }).await;
    let app = app(Some(broker)).await;
    for _ in 0..60 {
        assert_eq!(
            call(&app, Method::GET, "/mods/1", false).await.0,
            StatusCode::OK
        );
    }
    assert_eq!(
        call(&app, Method::GET, "/mods/1", false).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(hits.load(Ordering::Relaxed), 1);
    task.abort();
    let (broker, hits, task) = mock(|_| async { axum::Json(project(true)).into_response() }).await;
    let state = AppState::new(
        crate::Config::default(),
        Arc::new(crate::auth::DraslValidator::new("http://127.0.0.1:9")),
    )
    .with_curseforge(broker);
    let shared = state.curseforge.clone().unwrap();
    let permit = shared.concurrent.try_acquire_many(8).unwrap();
    let app = crate::router(state);
    assert_eq!(
        call(&app, Method::GET, "/mods/1", false).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(hits.load(Ordering::Relaxed), 0);
    drop(permit);
    assert!(shared.concurrent.try_acquire().is_ok());
    task.abort();
}

#[test]
fn cache_is_bounded_by_bytes_entries_and_ttl() {
    let mut cache = Cache::default();
    for n in 0..100 {
        cache.put(n.to_string(), vec![0; 1024]);
    }
    assert_eq!(cache.entries.len(), CACHE_ENTRIES);
    assert!(cache.get("0").is_none());
    for n in 100..110 {
        cache.put(n.to_string(), vec![0; BODY_LIMIT]);
    }
    assert_eq!(cache.entries.len(), 4);
    assert_eq!(cache.bytes, CACHE_BYTES);
    for (_, time, _) in &mut cache.entries {
        *time = Instant::now() - TTL;
    }
    cache.sweep();
    assert!(cache.entries.is_empty());
    assert_eq!(cache.bytes, 0);
}

async fn post(app: &Router, path: &str, body: Vec<u8>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("{PREFIX}{path}"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn batch_reads_are_bounded_fixed_and_validate_ownership() {
    let (broker, hits, task) = mock(|request| async move {
        assert_eq!(request.method(), Method::POST);
        let path = request.uri().path().to_owned();
        let body = request.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        axum::Json(match path.as_str() {
            "/mods" => {
                assert_eq!(body, json!({"modIds":[1]}));
                json!({"data":[project(true)["data"].clone()]})
            }
            "/mods/files" => {
                assert_eq!(body, json!({"fileIds":[2]}));
                json!({"data":[file()]})
            }
            _ => panic!("unexpected path"),
        })
        .into_response()
    })
    .await;
    let app = app(Some(broker)).await;
    for (path, body) in [
        ("/mods", json!({"modIds":[1]})),
        ("/files", json!({"fileIds":[2]})),
    ] {
        let (status, value) = post(&app, path, serde_json::to_vec(&body).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(value["data"][0]["gameId"], 432);
    }
    let before = hits.load(Ordering::Relaxed);
    assert_eq!(
        post(&app, "/files", br#"{"fileIds":[2]}"#.to_vec()).await.0,
        StatusCode::OK
    );
    assert_eq!(before, hits.load(Ordering::Relaxed));
    for body in [
        json!({"fileIds":[]}),
        json!({"fileIds":[0]}),
        json!({"fileIds":[1,1]}),
        json!({"fileIds":[1],"url":"https://attacker"}),
        json!({"modIds":[1]}),
        json!({"fileIds":[1.5]}),
        json!({"fileIds":(1..=51).collect::<Vec<_>>()}),
    ] {
        assert_eq!(
            post(&app, "/files", serde_json::to_vec(&body).unwrap())
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        post(&app, "/files", vec![b' '; 4097]).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        post(&app, "/files?url=bad", br#"{"fileIds":[2]}"#.to_vec())
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    task.abort();
}

#[tokio::test]
async fn batch_cross_game_unrequested_ids_and_owner_mismatches_fail_closed() {
    for wrong in [
        json!({"id":2,"gameId":1,"modId":1}),
        json!({"id":7,"gameId":432,"modId":1}),
        json!({"id":2,"gameId":432,"modId":7}),
    ] {
        let (broker, _, task) = mock(move |request| {
            let wrong = wrong.clone();
            async move {
                axum::Json(if request.uri().path() == "/mods" {
                    json!({"data":[]})
                } else {
                    json!({"data":[wrong]})
                })
                .into_response()
            }
        })
        .await;
        let status = post(
            &app(Some(broker)).await,
            "/files",
            br#"{"fileIds":[2]}"#.to_vec(),
        )
        .await
        .0;
        assert!([StatusCode::NOT_FOUND, StatusCode::BAD_GATEWAY].contains(&status));
        task.abort();
    }
}

#[test]
fn batch_parser_accepts_fifty_and_rejects_duplicate_fields_and_wrong_shapes() {
    assert!(batch(
        "/mods",
        &serde_json::to_vec(&json!({"modIds": (1..=50).collect::<Vec<_>>()})).unwrap()
    )
    .is_ok());
    for body in [
        br#"{"fileIds":[1],"fileIds":[2]}"#.as_slice(),
        br#"{"fileIds":null}"#,
        br#"{"fileIds":[2147483648]}"#,
        br#"{"fileIds":["1"]}"#,
        br#"{"fileIds":[-1]}"#,
        br#"[]"#,
    ] {
        assert!(batch("/files", body).is_err());
    }
}

#[tokio::test]
async fn whole_operation_times_out_with_sanitized_error() {
    let (broker, _, task) = mock(|_| async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        axum::Json(project(true)).into_response()
    })
    .await;
    let start = Instant::now();
    let (status, body) = call(&app(Some(broker)).await, Method::GET, "/mods/1", false).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["message"], "catalogue upstream unavailable");
    assert!(start.elapsed() < Duration::from_secs(20));
    task.abort();
}
