//! Endpoint tests against the shapes in `server/openapi/hub.yaml`.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tower::ServiceExt;

use super::*;

struct FakeValidator;

#[async_trait]
impl TokenValidator for FakeValidator {
    async fn validate(&self, token: &str) -> Result<Option<Identity>, ApiError> {
        Ok(match token {
            "alice" | "bob" => Some(Identity {
                // Drasl profile ids are undashed UUIDs.
                user_id: if token == "alice" {
                    "0000000000000000000000000000000a".into()
                } else {
                    "0000000000000000000000000000000b".into()
                },
                name: token.into(),
            }),
            // Valid token without a Yggdrasil profile.
            "noprofile" => Some(Identity {
                user_id: "token:0123456789abcdef".into(),
                name: "Player".into(),
            }),
            _ => None,
        })
    }
}

fn app() -> Router {
    router(state())
}

fn state() -> AppState {
    let cfg = Config {
        relay_urls: vec!["https://relay.example.ru".into()],
        trust_proxy: true,
        ..Config::default()
    };
    AppState::new(cfg, Arc::new(FakeValidator))
}

async fn call(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, headers, v)
}

fn post_json(uri: &str, token: Option<&str>, ip: &str, body: Value) -> Request<Body> {
    let mut b = Request::post(uri)
        .header("content-type", "application/json")
        .header("x-real-ip", ip);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

fn room_body(mode: &str, max_peers: u8) -> Value {
    json!({
        "endpointId": "hostendpoint1",
        "sig": { "mc": "1.21.1", "loader": "fabric", "loaderVersion": "0.16.0", "modsHash": "abc" },
        "authMode": mode,
        "maxPeers": max_peers,
        "passwordProtected": true
    })
}

/// `Error` schema: required `code` + `message` strings, `details` object|null.
fn assert_app_error(v: &Value, code: &str) {
    assert_eq!(v["code"], code, "{v}");
    assert!(v["message"].is_string());
    assert!(v["details"].is_null() || v["details"].is_object());
}

fn is_code(c: &str) -> bool {
    let b = c.as_bytes();
    b.len() == 9
        && b[4] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, x)| i == 4 || x.is_ascii_uppercase() || x.is_ascii_digit())
}

async fn create(app: &Router, mode: &str, max_peers: u8) -> Value {
    let (st, _, v) = call(
        app,
        post_json(
            "/api/v1/rooms",
            Some("alice"),
            "10.0.0.1",
            room_body(mode, max_peers),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    v
}

#[tokio::test]
async fn health_ok() {
    let (st, _, v) = call(
        &app(),
        Request::get("/api/v1/health").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["status"], "ok");
    assert!(v["version"].is_string());
}

#[tokio::test]
async fn stubs_return_501_app_error() {
    let (st, _, v) = call(
        &app(),
        Request::get("/api/v1/mirrors").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_IMPLEMENTED);
    assert_app_error(&v, "notImplemented");
    let (st, _, v) = call(
        &app(),
        Request::get("/api/v1/nope").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_app_error(&v, "notFound");
}

#[tokio::test]
async fn create_room_shape_and_auth() {
    let app = app();
    let v = create(&app, "yggdrasil", 4).await;
    assert!(Uuid::parse_str(v["roomId"].as_str().unwrap()).is_ok());
    assert!(is_code(v["code"].as_str().unwrap()), "{v}");
    assert!(v["hostToken"].as_str().unwrap().len() >= 32);
    let exp = v["expiresAt"].as_str().unwrap();
    assert!(
        exp.len() == 20 && exp.ends_with('Z') && exp.as_bytes()[10] == b'T',
        "{exp}"
    );
    assert_eq!(v["relayUrls"], json!(["https://relay.example.ru"]));

    // Bearer is required for yggdrasil rooms; an invalid bearer is rejected for any mode.
    let (st, _, v) = call(
        &app,
        post_json("/api/v1/rooms", None, "10.0.0.2", room_body("yggdrasil", 4)),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_app_error(&v, "authRequired");
    let (st, _, v) = call(
        &app,
        post_json(
            "/api/v1/rooms",
            Some("mallory"),
            "10.0.0.2",
            room_body("open", 4),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_app_error(&v, "authExpired");

    let (st, _, v) = call(
        &app,
        post_json(
            "/api/v1/rooms",
            Some("alice"),
            "10.0.0.3",
            room_body("open", 40),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_app_error(&v, "invalidInput");
    let (st, _, v) = call(
        &app,
        post_json("/api/v1/rooms", Some("alice"), "10.0.0.3", json!({"x": 1})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_app_error(&v, "invalidInput");
}

#[tokio::test]
async fn anonymous_open_room_per_ip_limits() {
    let app = app();
    let anon = |ip: &str| post_json("/api/v1/rooms", None, ip, room_body("open", 4));
    let (st, _, a) = call(&app, anon("10.5.0.1")).await;
    assert_eq!(st, StatusCode::CREATED, "{a}");
    let (st, _, _) = call(&app, anon("10.5.0.1")).await;
    assert_eq!(st, StatusCode::CREATED);
    // Third active anonymous room from the same IP.
    let (st, h, v) = call(&app, anon("10.5.0.1")).await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
    assert_app_error(&v, "rateLimited");
    assert!(h.contains_key("retry-after"));
    // Another IP is independent.
    let (st, _, _) = call(&app, anon("10.5.0.2")).await;
    assert_eq!(st, StatusCode::CREATED);
    // Closing with hostToken frees a slot; anonymous host gets a working room.
    let id = a["roomId"].as_str().unwrap();
    let (st, _, _) = call(&app, delete_req(id, a["hostToken"].as_str())).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _, _) = call(&app, anon("10.5.0.1")).await;
    assert_eq!(st, StatusCode::CREATED);
}

#[tokio::test]
async fn anonymous_creates_capped_per_hour() {
    // 10 creates/hour/IP. Over HTTP the per-minute limiter (burst 5) fires first,
    // so the hourly bucket is checked directly.
    let rl = RateLimiter::<IpAddr>::per_hour(10, 10);
    let ip: IpAddr = "10.6.0.1".parse().unwrap();
    for _ in 0..10 {
        assert!(rl.check(&ip).is_ok());
    }
    let wait = rl.check(&ip).unwrap_err();
    assert!((300..=360).contains(&wait), "{wait}");
}

#[tokio::test]
async fn peer_token_carries_bearer_uuid() {
    let app = app();
    let room = create(&app, "open", 8).await;
    let uri = format!("/api/v1/rooms/{}/join", room["code"].as_str().unwrap());
    let (_, _, j) = call(
        &app,
        post_json(&uri, Some("bob"), "10.8.0.1", json!({"endpointId": "gb"})),
    )
    .await;
    let parts: Vec<&str> = j["peerToken"].as_str().unwrap().split('.').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[1], "0000000000000000000000000000000b");
    let exp: u64 = parts[0].parse().unwrap();
    let rid = Uuid::parse_str(room["roomId"].as_str().unwrap()).unwrap();
    let expect = rooms::peer_token(
        room["hostToken"].as_str().unwrap(),
        rid,
        "gb",
        parts[1],
        exp,
    );
    assert_eq!(j["peerToken"], expect.as_str());
    // A client-supplied uuid is not trusted for anonymous joins.
    let body = json!({"endpointId": "ga", "uuid": "0000000000000000000000000000000b"});
    let (_, _, j) = call(&app, post_json(&uri, None, "10.8.0.1", body)).await;
    assert_eq!(
        j["peerToken"].as_str().unwrap().split('.').nth(1),
        Some("-")
    );
}

#[tokio::test]
async fn create_room_rate_limited_per_ip() {
    let app = app();
    let mut last = StatusCode::OK;
    let mut headers = HeaderMap::new();
    let mut body = Value::Null;
    for _ in 0..6 {
        (last, headers, body) = call(
            &app,
            post_json("/api/v1/rooms", Some("bob"), "10.9.9.9", json!({})),
        )
        .await;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    assert_app_error(&body, "rateLimited");
    assert!(body["details"]["retryAfter"].is_string());
    assert!(headers.contains_key("retry-after"));
    // Another IP is unaffected.
    let (st, _, _) = call(
        &app,
        post_json("/api/v1/rooms", Some("bob"), "10.9.9.8", json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn join_room_flow() {
    let app = app();
    let room = create(&app, "open", 2).await;
    let code = room["code"].as_str().unwrap().to_lowercase();
    let uri = format!("/api/v1/rooms/{code}/join");

    // Open room: anonymous join is allowed.
    let (st, _, v) = call(
        &app,
        post_json(&uri, None, "10.1.0.1", json!({"endpointId": "guest1"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    for k in [
        "roomId",
        "peerToken",
        "hostEndpointId",
        "sig",
        "authMode",
        "relayUrls",
        "passwordProtected",
    ] {
        assert!(!v[k].is_null(), "missing {k}: {v}");
    }
    assert_eq!(v["roomId"], room["roomId"]);
    assert_eq!(v["hostEndpointId"], "hostendpoint1");
    assert_eq!(v["sig"]["loader"], "fabric");
    assert_eq!(v["kind"], "game"); // default when omitted on create
    assert_eq!(v["authMode"], "open");
    assert_eq!(v["passwordProtected"], true);

    // maxPeers=2 includes the host -> second guest gets 409.
    let (st, _, v) = call(
        &app,
        post_json(&uri, None, "10.1.0.1", json!({"endpointId": "guest2"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_app_error(&v, "conflict");

    let (st, _, v) = call(
        &app,
        post_json(
            "/api/v1/rooms/ZZZZ-ZZZZ/join",
            None,
            "10.1.0.1",
            json!({"endpointId": "g"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_app_error(&v, "notFound");

    // Yggdrasil room requires a valid bearer.
    let room = create(&app, "yggdrasil", 4).await;
    let uri = format!("/api/v1/rooms/{}/join", room["code"].as_str().unwrap());
    let (st, _, v) = call(
        &app,
        post_json(&uri, None, "10.1.0.2", json!({"endpointId": "g"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_app_error(&v, "authRequired");
    let (st, _, _) = call(
        &app,
        post_json(&uri, Some("bad"), "10.1.0.2", json!({"endpointId": "g"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, _) = call(
        &app,
        post_json(&uri, Some("bob"), "10.1.0.2", json!({"endpointId": "g"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
async fn voice_room_without_sig() {
    let app = app();
    // Game rooms (explicit or default kind) still require sig.
    for kind in [json!("game"), Value::Null] {
        let mut body = json!({"endpointId": "h1", "authMode": "open"});
        if !kind.is_null() {
            body["kind"] = kind;
        }
        let (st, _, v) = call(
            &app,
            post_json("/api/v1/rooms", Some("alice"), "10.11.0.1", body),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
        assert_app_error(&v, "invalidInput");
    }
    // Unknown kind is rejected.
    let body = json!({"endpointId": "h1", "kind": "video", "authMode": "open"});
    let (st, _, v) = call(
        &app,
        post_json("/api/v1/rooms", Some("alice"), "10.11.0.1", body),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_app_error(&v, "invalidInput");

    // Voice room: no sig on create; join returns kind=voice and no sig key.
    // A sig sent anyway is dropped. Same anonymous/open rules as game rooms.
    for (i, body) in [
        json!({"endpointId": "h1", "kind": "voice", "authMode": "open", "maxPeers": 3}),
        json!({"endpointId": "h2", "kind": "voice", "authMode": "open",
               "sig": {"mc": "1.21.1", "loader": "vanilla", "modsHash": ""}}),
    ]
    .into_iter()
    .enumerate()
    {
        let (st, _, room) = call(
            &app,
            post_json("/api/v1/rooms", None, &format!("10.11.1.{i}"), body),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{room}");
        let uri = format!("/api/v1/rooms/{}/join", room["code"].as_str().unwrap());
        let (st, _, j) = call(
            &app,
            post_json(&uri, None, "10.11.2.1", json!({"endpointId": "g"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{j}");
        assert_eq!(j["kind"], "voice");
        assert!(j.as_object().unwrap().get("sig").is_none(), "{j}");
        for k in [
            "roomId",
            "peerToken",
            "hostEndpointId",
            "authMode",
            "relayUrls",
            "passwordProtected",
        ] {
            assert!(!j[k].is_null(), "missing {k}: {j}");
        }
    }

    // Yggdrasil voice room still requires a bearer to create.
    let body = json!({"endpointId": "h3", "kind": "voice", "authMode": "yggdrasil"});
    let (st, _, v) = call(&app, post_json("/api/v1/rooms", None, "10.11.3.1", body)).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_app_error(&v, "authRequired");
}

#[tokio::test]
async fn join_rate_limited() {
    let app = app();
    let mut last = StatusCode::OK;
    for _ in 0..11 {
        (last, _, _) = call(
            &app,
            post_json(
                "/api/v1/rooms/AAAA-AAAA/join",
                None,
                "10.7.7.7",
                json!({"endpointId": "g"}),
            ),
        )
        .await;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}

fn delete_req(id: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::delete(format!("/api/v1/rooms/{id}")).header("x-real-ip", "10.2.0.1");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn close_room_auth() {
    let app = app();
    let room = create(&app, "open", 4).await;
    let id = room["roomId"].as_str().unwrap();

    let (st, _, v) = call(&app, delete_req(id, None)).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_app_error(&v, "authRequired");
    let (st, _, _) = call(&app, delete_req(id, Some("bad"))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, v) = call(&app, delete_req(id, Some("bob"))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert_app_error(&v, "permissionDenied");
    let (st, _, _) = call(&app, delete_req(id, Some("alice"))).await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Closed room no longer joinable; host token works too.
    let room = create(&app, "open", 4).await;
    let (st, _, _) = call(
        &app,
        delete_req(room["roomId"].as_str().unwrap(), room["hostToken"].as_str()),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let uri = format!("/api/v1/rooms/{}/join", room["code"].as_str().unwrap());
    let (st, _, _) = call(
        &app,
        post_json(&uri, None, "10.2.0.2", json!({"endpointId": "g"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ws_rejects_bad_token() {
    let app = app();
    let room = create(&app, "open", 4).await;
    let id = room["roomId"].as_str().unwrap();
    for uri in [
        format!("/api/v1/rooms/{id}/ws?token=nope"),
        format!("/api/v1/rooms/{id}/ws"),
    ] {
        let (st, _, v) = call(&app, Request::get(uri).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        assert!(v["code"].is_string());
    }
}

async fn next_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let WsMsg::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

async fn next_of(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    t: &str,
) -> Value {
    loop {
        let v = next_json(ws).await;
        if v["t"] == t {
            return v;
        }
    }
}

#[tokio::test]
async fn ws_presence_chat_and_close() {
    let st = state();
    let app = router(st);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let svc = app
        .clone()
        .into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(l, svc).await.unwrap() });

    let room = create(&app, "open", 4).await;
    let id = room["roomId"].as_str().unwrap();
    let uri = format!("/api/v1/rooms/{}/join", room["code"].as_str().unwrap());
    let (_, _, j) = call(
        &app,
        post_json(&uri, None, "10.3.0.1", json!({"endpointId": "guest9"})),
    )
    .await;

    let url = |t: &str| format!("ws://{addr}/api/v1/rooms/{id}/ws?token={t}");
    let (mut host, _) = tokio_tungstenite::connect_async(url(room["hostToken"].as_str().unwrap()))
        .await
        .unwrap();
    let hello = next_of(&mut host, "host_endpoint").await;
    assert_eq!(hello["endpointId"], "hostendpoint1");

    let (mut guest, _) = tokio_tungstenite::connect_async(url(j["peerToken"].as_str().unwrap()))
        .await
        .unwrap();
    let joined = next_of(&mut host, "peer_joined").await;
    let joined = if joined["name"] == "alice" {
        next_of(&mut host, "peer_joined").await
    } else {
        joined
    };
    assert_eq!(joined["name"], "Guest-gues");
    let guest_id = joined["peerId"].clone();

    guest
        .send(WsMsg::text(json!({"t": "ping", "nonce": 1}).to_string()))
        .await
        .unwrap();
    guest
        .send(WsMsg::text(
            json!({"t": "chat", "id": "m1", "text": " hi "}).to_string(),
        ))
        .await
        .unwrap();
    let chat = next_of(&mut host, "chat").await;
    assert_eq!(chat["id"], "m1");
    assert_eq!(chat["text"], "hi");
    assert_eq!(chat["fromPeer"], guest_id);
    assert_eq!(chat["fromName"], "Guest-gues");
    assert!(chat["sentAt"].as_str().unwrap().ends_with('Z'));

    drop(guest);
    let left = next_of(&mut host, "peer_left").await;
    assert_eq!(left["peerId"], guest_id);

    let (st, _, _) = call(&app, delete_req(id, Some("alice"))).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    next_of(&mut host, "closed").await;
}

async fn serve(app: &Router) -> SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let svc = app
        .clone()
        .into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(l, svc).await.unwrap() });
    addr
}

#[tokio::test]
async fn ws_join_leave_x10_and_kick_free_slots() {
    let app = app();
    let addr = serve(&app).await;
    // maxPeers=2: host + one live guest.
    let room = create(&app, "open", 2).await;
    let id = room["roomId"].as_str().unwrap();
    let uri = format!("/api/v1/rooms/{}/join", room["code"].as_str().unwrap());
    let url = |t: &str| format!("ws://{addr}/api/v1/rooms/{id}/ws?token={t}");
    let (mut host, _) = tokio_tungstenite::connect_async(url(room["hostToken"].as_str().unwrap()))
        .await
        .unwrap();
    next_of(&mut host, "host_endpoint").await;
    assert_eq!(next_of(&mut host, "peer_joined").await["name"], "alice"); // host itself

    for i in 0..10 {
        let body = json!({"endpointId": format!("guest{i}"), "name": "Steve"});
        let (st, _, j) = call(&app, post_json(&uri, None, &format!("10.4.0.{i}"), body)).await;
        assert_eq!(st, StatusCode::OK, "iteration {i}: {j}");
        let (mut g, _) = tokio_tungstenite::connect_async(url(j["peerToken"].as_str().unwrap()))
            .await
            .unwrap();
        let joined = next_of(&mut host, "peer_joined").await;
        assert_eq!(joined["name"], "Steve");
        if i % 2 == 0 {
            g.send(WsMsg::text(json!({"t": "leave"}).to_string()))
                .await
                .unwrap();
        } else {
            g.close(None).await.unwrap();
        }
        next_of(&mut host, "peer_left").await;
    }

    // Host kick frees the slot and closes the guest's socket.
    let (_, _, j) = call(
        &app,
        post_json(&uri, None, "10.4.1.1", json!({"endpointId": "k"})),
    )
    .await;
    let (mut g, _) = tokio_tungstenite::connect_async(url(j["peerToken"].as_str().unwrap()))
        .await
        .unwrap();
    let pid = next_of(&mut host, "peer_joined").await["peerId"].clone();
    let busy = call(
        &app,
        post_json(&uri, None, "10.4.1.2", json!({"endpointId": "x"})),
    )
    .await;
    assert_eq!(busy.0, StatusCode::CONFLICT);
    let kick = json!({"t": "kick", "peerId": pid, "reason": "bye"});
    host.send(WsMsg::text(kick.to_string())).await.unwrap();
    let k = next_of(&mut g, "kick").await;
    assert_eq!(k["reason"], "bye");
    let (st, _, _) = call(
        &app,
        post_json(&uri, None, "10.4.1.3", json!({"endpointId": "x"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // Name validation.
    let long = json!({"endpointId": "y", "name": "a".repeat(17)});
    let (st, _, v) = call(&app, post_json(&uri, None, "10.4.1.4", long)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_app_error(&v, "invalidInput");
}

mod vpn_api;
