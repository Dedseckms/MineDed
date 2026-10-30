//! Security and persistence regression tests for the chat API.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::auth::{Identity, TokenValidator};
use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use rusqlite::{params, OptionalExtension};
use tower::ServiceExt;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("hub-chat-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn identity(n: u8) -> Identity {
    Identity {
        user_id: format!("{n:032x}"),
        name: format!("Player{n}"),
    }
}
fn chat() -> (Arc<Chat>, Temp) {
    let temp = Temp::new();
    (Arc::new(Chat::open(&temp.0).unwrap()), temp)
}
fn seed(db: &store::Store) {
    for n in 1..=3 {
        db.identify(&identity(n), false, None).unwrap();
    }
}
fn direct(db: &store::Store, a: u8, b: u8) -> String {
    let a = identity(a).user_id;
    let b = identity(b).user_id;
    let c = format!("dm:{a}:{b}");
    db.db
        .execute(
            "INSERT OR IGNORE INTO channels(id,a,b) VALUES(?1,?2,?3)",
            params![c, a, b],
        )
        .unwrap();
    c
}
fn text(s: &str) -> SocialContent {
    SocialContent::Text { text: s.into() }
}

fn pack(code: Option<&str>) -> SocialContent {
    SocialContent::PackShare {
        code: code.map(str::to_owned),
        password_protected: true,
        name: "Shared pack".into(),
        mc: "1.21.1".into(),
        loader: "fabric".into(),
        mods: 12,
    }
}

#[test]
fn pack_sharing_requires_room_code_and_direct_channel() {
    let (chat, _temp) = chat();
    let db = chat.store.lock().unwrap();
    seed(&db);
    let a = identity(1).user_id;
    let channel = direct(&db, 1, 2);
    for code in [
        None,
        Some(""),
        Some("invalid"),
        Some("https://attacker.invalid"),
    ] {
        assert_eq!(
            db.send(&a, &channel, pack(code), &chat).unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        db.send(&a, "global", pack(Some("abcd1234")), &chat)
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    let sent = db
        .send(&a, &channel, pack(Some(" abcd1234 ")), &chat)
        .unwrap();
    assert!(
        matches!(sent.content,SocialContent::PackShare {code:Some(code),password_protected:true,..} if code == "ABCD-1234")
    );
    // Legacy metadata cards remain readable but cannot be posted again as invitations.
    let legacy: SocialContent = serde_json::from_value(
        json!({"type":"packShare","name":"Old","mc":"1.20.1","loader":"forge","mods":1}),
    )
    .unwrap();
    assert!(matches!(
        legacy,
        SocialContent::PackShare {
            code: None,
            password_protected: false,
            ..
        }
    ));
}

#[test]
fn pack_sharing_obeys_invite_privacy_and_blocks() {
    let (chat, _temp) = chat();
    let mut db = chat.store.lock().unwrap();
    seed(&db);
    let a = identity(1).user_id;
    let b = identity(2).user_id;
    let channel = direct(&db, 1, 2);
    db.handle(&b,"PUT",&["privacy"],&Value::Null,json!({"whoCanMessage":"everyone","whoCanInvite":"friends","whoSeesStatus":"everyone","whoCanAddFriend":"everyone"}),&chat).unwrap();
    assert!(db
        .send(&a, &channel, text("ordinary message"), &chat)
        .is_ok());
    assert_eq!(
        db.send(&a, &channel, pack(Some("abcd1234")), &chat)
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    db.handle(&b,"PUT",&["privacy"],&Value::Null,json!({"whoCanMessage":"everyone","whoCanInvite":"everyone","whoSeesStatus":"everyone","whoCanAddFriend":"everyone"}),&chat).unwrap();
    assert!(db.send(&a, &channel, pack(Some("abcd1234")), &chat).is_ok());
    db.handle(
        &b,
        "PUT",
        &["users", &a, "block"],
        &Value::Null,
        Value::Null,
        &chat,
    )
    .unwrap();
    assert_eq!(
        db.send(&a, &channel, pack(Some("abcd1234")), &chat)
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
}
#[test]
fn wal_history_survives_restart_and_cursor_is_exclusive() {
    let (chat, temp) = chat();
    {
        let mut db = chat.store.lock().unwrap();
        seed(&db);
        assert_eq!(
            db.db
                .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "wal"
        );
        for i in 0..3 {
            db.send(
                &identity(1).user_id,
                "global",
                text(&format!("message{i}")),
                &chat,
            )
            .unwrap();
        }
        let raw: Vec<u8> = db
            .db
            .query_row("SELECT body FROM messages LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_ne!(&raw[..4], b"{\"ty");
        let history = db
            .history(&identity(2).user_id, "global", None, 2, &chat)
            .unwrap();
        assert_eq!(history.messages.len(), 2);
        let before = history.before.unwrap().parse().unwrap();
        let earlier = db
            .history(&identity(2).user_id, "global", Some(before), 2, &chat)
            .unwrap();
        assert_eq!(earlier.messages.len(), 1);
        let read = history.messages[0].id.clone();
        db.handle(
            &identity(2).user_id,
            "POST",
            &["channels", "global", "read"],
            &Value::Null,
            json!({"messageId":read}),
            &chat,
        )
        .unwrap();
        assert_eq!(
            db.channel(&identity(2).user_id, "global", &chat)
                .unwrap()
                .unread,
            1
        );
    }
    drop(chat);
    let reopened = Chat::open(&temp.0).unwrap();
    let db = reopened.store.lock().unwrap();
    assert_eq!(
        db.history(&identity(1).user_id, "global", None, 100, &reopened)
            .unwrap()
            .messages
            .len(),
        3
    );
}
#[test]
fn dm_membership_privacy_and_block_apply_at_send_time() {
    let (chat, _temp) = chat();
    let mut db = chat.store.lock().unwrap();
    seed(&db);
    let channel = direct(&db, 1, 2);
    let a = identity(1).user_id;
    let b = identity(2).user_id;
    assert_eq!(
        db.history(&identity(3).user_id, &channel, None, 50, &chat)
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    db.handle(&b,"PUT",&["privacy"],&Value::Null,json!({"whoCanMessage":"nobody","whoCanInvite":"friends","whoSeesStatus":"nobody","whoCanAddFriend":"everyone"}),&chat).unwrap();
    assert_eq!(
        db.send(&a, &channel, text("hello"), &chat)
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(db.user(&a, &b, &chat).unwrap().status, None);
    db.handle(
        &b,
        "PUT",
        &["users", &a, "block"],
        &Value::Null,
        Value::Null,
        &chat,
    )
    .unwrap();
    assert!(db.send(&a, &channel, text("hello"), &chat).is_err());
    assert!(db.user(&b, &a, &chat).unwrap().is_blocked);
    assert!(!db.user(&a, &b, &chat).unwrap().is_blocked);
}
#[test]
fn profanity_only_filters_global_and_metadata_is_server_owned() {
    let (chat, _temp) = chat();
    let db = chat.store.lock().unwrap();
    seed(&db);
    let a = identity(1).user_id;
    let channel = direct(&db, 1, 2);
    assert_eq!(
        db.send(&a, "global", text("х.у.й"), &chat)
            .unwrap_err()
            .details
            .unwrap()["reason"],
        "filtered"
    );
    assert!(db.send(&a, &channel, text("х.у.й"), &chat).is_ok());
    let attachment = SocialAttachment {
        id: "file1".into(),
        file_name: "safe.bin".into(),
        mime: "application/octet-stream".into(),
        size: 5,
        url: "/api/v1/chat/files/file1".into(),
        thumb_url: None,
        width: None,
        height: None,
    };
    db.db
        .execute(
            "INSERT INTO attachments(id,owner,metadata,hash) VALUES(?1,?2,?3,'hash')",
            params![
                attachment.id,
                a,
                serde_json::to_string(&attachment).unwrap()
            ],
        )
        .unwrap();
    let mut forged = attachment.clone();
    forged.url = "https://evil.invalid".into();
    forged.mime = "text/html".into();
    forged.size = u64::MAX;
    let sent = db
        .send(
            &a,
            &channel,
            SocialContent::Attachment {
                attachment: forged,
                caption: None,
            },
            &chat,
        )
        .unwrap();
    assert_eq!(
        sent.content,
        SocialContent::Attachment {
            attachment: attachment.clone(),
            caption: None
        }
    );
    assert!(db
        .send(
            &identity(2).user_id,
            &channel,
            SocialContent::Attachment {
                attachment,
                caption: None
            },
            &chat
        )
        .is_err());
}
#[test]
fn friend_acceptance_requires_other_party_and_blocks_override() {
    let (chat, _temp) = chat();
    let mut db = chat.store.lock().unwrap();
    seed(&db);
    let a = identity(1).user_id;
    let b = identity(2).user_id;
    for _ in 0..2 {
        db.handle(
            &a,
            "PUT",
            &["friends", &b],
            &Value::Null,
            Value::Null,
            &chat,
        )
        .unwrap();
    }
    assert!(!db.friends_with(&a, &b).unwrap());
    db.handle(
        &b,
        "PUT",
        &["friends", &a],
        &Value::Null,
        Value::Null,
        &chat,
    )
    .unwrap();
    assert!(db.friends_with(&a, &b).unwrap());
    db.handle(
        &a,
        "PUT",
        &["users", &b, "block"],
        &Value::Null,
        Value::Null,
        &chat,
    )
    .unwrap();
    assert!(!db.friends_with(&a, &b).unwrap());
    assert!(db
        .handle(
            &b,
            "PUT",
            &["friends", &a],
            &Value::Null,
            Value::Null,
            &chat
        )
        .is_err());
}
#[test]
fn moderation_never_trusts_role_claims_and_is_public_only() {
    let (chat, _temp) = chat();
    let mut db = chat.store.lock().unwrap();
    seed(&db);
    let a = identity(1).user_id;
    let b = identity(2).user_id;
    assert!(db
        .handle(
            &a,
            "PUT",
            &["admin", "roles", &a],
            &Value::Null,
            json!({"role":"admin"}),
            &chat
        )
        .is_err());
    db.db
        .execute("UPDATE users SET moderator=1 WHERE uuid=?1", [&a])
        .unwrap();
    assert!(db
        .handle(
            &a,
            "POST",
            &["mod", "users", &b],
            &Value::Null,
            json!({"action":"ban"}),
            &chat
        )
        .is_err());
    assert!(db
        .handle(
            &a,
            "POST",
            &["mod", "users", &b],
            &Value::Null,
            json!({"action":"mute","seconds":86401}),
            &chat
        )
        .is_err());
    db.handle(
        &a,
        "POST",
        &["mod", "users", &b],
        &Value::Null,
        json!({"action":"mute","seconds":3600}),
        &chat,
    )
    .unwrap();
    assert!(db.send(&b, "global", text("hello"), &chat).is_err());
    let channel = direct(&db, 1, 2);
    let msg = db.send(&b, &channel, text("private"), &chat).unwrap();
    // Moderator is a member, but not the author; private moderation is still forbidden.
    assert!(db
        .handle(
            &a,
            "DELETE",
            &["messages", &msg.id],
            &Value::Null,
            Value::Null,
            &chat
        )
        .is_err());
}
#[test]
fn migration_without_configuration_never_reports_success() {
    let (chat, _temp) = chat();
    let db = chat.store.lock().unwrap();
    let source = db.files.clone();
    // An invalid target cannot alter the persisted storage root.
    assert!(db
        .prepare_migration(json!({"target":"unknown"}), &chat)
        .is_err());
    assert_eq!(db.files, source);
    let pointer: Option<String> = db
        .db
        .query_row("SELECT value FROM settings WHERE key='files'", [], |r| {
            r.get(0)
        })
        .optional()
        .unwrap();
    assert!(pointer.is_none());
}
#[async_trait]
impl TokenValidator for Fake {
    async fn validate(&self, token: &str) -> Result<Option<Identity>, ApiError> {
        Ok(match token {
            "good" => Some(identity(1)),
            "other" => Some(identity(2)),
            "third" => Some(identity(3)),
            _ => None,
        })
    }
}
struct Fake;
fn app_with_chat() -> (Router, Arc<Chat>, Temp) {
    let temp = Temp::new();
    let state = AppState::new(crate::Config::default(), Arc::new(Fake))
        .with_chat(Chat::open(&temp.0).unwrap());
    let chat = state.chat.clone().unwrap();
    (crate::router(state), chat, temp)
}
fn app() -> (Router, Temp) {
    let (app, _, temp) = app_with_chat();
    (app, temp)
}
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: bool,
    body: Value,
) -> (StatusCode, Value) {
    request_abs(
        app,
        method,
        &format!("/api/v1/chat/{path}"),
        token.then_some("Bearer good"),
        body,
    )
    .await
}
async fn request_abs(
    app: &Router,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let (status, bytes) = request_bytes(
        app,
        method,
        path,
        authorization,
        Body::from(serde_json::to_vec(&body).unwrap()),
    )
    .await;
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}
async fn request_bytes(
    app: &Router,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: Body,
) -> (StatusCode, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(value) = authorization {
        req = req.header("authorization", value);
    }
    let response = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}
#[tokio::test]
async fn auth_errors_use_app_error_on_every_surface() {
    let (app, _temp) = app();
    for (method, path) in [
        ("GET", "me"),
        ("GET", "ws"),
        ("POST", "files"),
        ("GET", "files/file"),
        ("GET", "channels"),
        ("GET", "friends"),
        ("GET", "mod/reports"),
        ("GET", "admin/server"),
    ] {
        let (status, error) = request(&app, method, path, false, Value::Null).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(error["code"], "authRequired");
        assert!(error.get("details").is_some());
    }
    let (status, user) = request(&app, "GET", "me", true, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user["uuid"], identity(1).user_id);
    assert_eq!(user["role"], "user");
    let (status, _) = request(&app, "GET", "admin/server", true, Value::Null).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
#[tokio::test]
async fn attachment_download_requires_posted_channel_membership() {
    let (chat, _temp) = chat();
    let a = identity(1).user_id;
    let b = identity(2).user_id;
    let outsider = identity(3).user_id;
    {
        let db = chat.store.lock().unwrap();
        seed(&db);
        let c = direct(&db, 1, 2);
        let hash = files::persist(&db.files, b"hello").unwrap();
        let attachment = SocialAttachment {
            id: "private".into(),
            file_name: "x".into(),
            mime: "application/octet-stream".into(),
            size: 5,
            url: "/private".into(),
            thumb_url: None,
            width: None,
            height: None,
        };
        db.db
            .execute(
                "INSERT INTO attachments(id,owner,metadata,hash) VALUES(?1,?2,?3,?4)",
                params![
                    attachment.id,
                    a,
                    serde_json::to_string(&attachment).unwrap(),
                    hash
                ],
            )
            .unwrap();
        db.send(
            &a,
            &c,
            SocialContent::Attachment {
                attachment,
                caption: None,
            },
            &chat,
        )
        .unwrap();
    }
    assert!(files::download_bytes(chat.clone(), &b, "private", false)
        .await
        .is_ok());
    assert_eq!(
        files::download_bytes(chat, &outsider, "private", false)
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn websocket_dm_delivery_never_reaches_non_members() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let temp = Temp::new();
    let state = AppState::new(crate::Config::default(), Arc::new(Fake))
        .with_chat(Chat::open(&temp.0).unwrap());
    let chat = state.chat.clone().unwrap();
    let channel = {
        let db = chat.store.lock().unwrap();
        seed(&db);
        direct(&db, 1, 2)
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            crate::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let endpoint = format!("ws://{addr}/api/v1/chat/ws");
    let mut req = endpoint.clone().into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", "Bearer other".parse().unwrap());
    let (mut member, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let mut req = endpoint.into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", "Bearer third".parse().unwrap());
    let (mut outsider, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(member.next().await.unwrap().unwrap().to_text().unwrap())
            .unwrap()["type"],
        "connection"
    );
    assert_eq!(
        serde_json::from_str::<Value>(outsider.next().await.unwrap().unwrap().to_text().unwrap())
            .unwrap()["type"],
        "connection"
    );
    let id = {
        let db = chat.store.lock().unwrap();
        db.send(
            &identity(1).user_id,
            &channel,
            text("private payload"),
            &chat,
        )
        .unwrap()
        .id
        .parse()
        .unwrap()
    };
    let _ = chat.events.send(Notice::Message(id, false));
    let received = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let frame = member.next().await.unwrap().unwrap();
            if frame.is_text() {
                let value: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                if value["type"] == "message" {
                    break value;
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(received["message"]["content"]["text"], "private payload");
    assert!(tokio::time::timeout(Duration::from_millis(150), async {
        loop {
            let frame = outsider.next().await.unwrap().unwrap();
            if frame.is_text() {
                let value: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                if value["type"] == "message" {
                    break;
                }
            }
        }
    })
    .await
    .is_err());
    member.close(None).await.unwrap();
    outsider.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn websocket_fanout_waits_for_jobs_without_disconnecting_or_leaking_dms() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    struct LoadValidator;
    #[async_trait]
    impl TokenValidator for LoadValidator {
        async fn validate(&self, token: &str) -> Result<Option<Identity>, ApiError> {
            Ok(token
                .strip_prefix("load-")
                .and_then(|n| n.parse::<u8>().ok())
                .filter(|n| (1..=10).contains(n))
                .map(identity))
        }
    }
    let temp = Temp::new();
    let state = AppState::new(crate::Config::default(), Arc::new(LoadValidator))
        .with_chat(Chat::open(&temp.0).unwrap());
    let chat = state.chat.clone().unwrap();
    let (private, public) = {
        let db = chat.store.lock().unwrap();
        for n in 1..=10 {
            db.identify(&identity(n), false, None).unwrap();
        }
        let channel = direct(&db, 1, 2);
        let private = db
            .send(
                &identity(1).user_id,
                &channel,
                text("private under load"),
                &chat,
            )
            .unwrap()
            .id
            .parse::<i64>()
            .unwrap();
        let public = db
            .send(
                &identity(1).user_id,
                "global",
                text("fanout resumed"),
                &chat,
            )
            .unwrap()
            .id
            .parse::<i64>()
            .unwrap();
        (private, public)
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/api/v1/chat/ws", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            crate::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let mut receivers = Vec::new();
    for n in 1..=10 {
        let mut request = endpoint.clone().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer load-{n}").parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(frame.to_text().unwrap()).unwrap()["type"],
            "connection"
        );
        receivers.push(socket);
    }
    // Saturate admission after every handshake, then exercise actual socket fanout.
    let held = chat.jobs.clone().acquire_many_owned(8).await.unwrap();
    assert_eq!(
        chat.run(|_, _| Ok(())).await.unwrap_err().status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(chat.events.send(Notice::Message(private, false)).is_ok());
    assert!(chat.events.send(Notice::Message(public, false)).is_ok());
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(held);
    for (index, socket) in receivers.iter_mut().enumerate() {
        let saw_private = tokio::time::timeout(Duration::from_secs(3), async {
            let mut saw_private = false;
            loop {
                let frame = socket
                    .next()
                    .await
                    .expect("subscriber disconnected under load")
                    .unwrap();
                assert!(!frame.is_close(), "subscriber closed under load");
                if !frame.is_text() {
                    continue;
                }
                let value: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                if value["type"] != "message" {
                    continue;
                }
                let text = &value["message"]["content"]["text"];
                if text == "private under load" {
                    saw_private = true;
                }
                if text == "fanout resumed" {
                    break saw_private;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            saw_private,
            index < 2,
            "DM membership must be preserved during fanout"
        );
        socket.close(None).await.unwrap();
    }
    server.abort();
}

#[tokio::test]
async fn multipart_limit_and_metadata_round_trip() {
    let (app, _temp) = app();
    for (size, expected) in [
        (5, StatusCode::OK),
        (files::MAX_UPLOAD + 1, StatusCode::PAYLOAD_TOO_LARGE),
    ] {
        let mut body=b"--chat\r\nContent-Disposition: form-data; name=\"file\"; filename=\"safe.bin\"\r\nContent-Type: text/html\r\n\r\n".to_vec();
        body.extend(std::iter::repeat_n(b'x', size));
        body.extend_from_slice(b"\r\n--chat--\r\n");
        let req = Request::post("/api/v1/chat/files")
            .header("authorization", "Bearer good")
            .header("content-type", "multipart/form-data; boundary=chat")
            .body(Body::from(body))
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), expected);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let payload: Value = serde_json::from_slice(&bytes).unwrap();
        if size == 5 {
            assert_eq!(payload["mime"], "application/octet-stream");
            assert_eq!(payload["size"], 5);
        } else {
            assert_eq!(payload["code"], "invalidInput");
        }
    }
}

#[test]
fn trained_dictionary_preserves_old_frames_after_restart() {
    let (chat, temp) = chat();
    {
        let db = chat.store.lock().unwrap();
        seed(&db);
        for i in 0..205 {
            db.send(
                &identity(1).user_id,
                "global",
                text(&format!(
                    "message {i}: Minecraft world seed and build number {}",
                    i * 179
                )),
                &chat,
            )
            .unwrap();
        }
        assert!(db
            .db
            .query_row(
                "SELECT length(body) FROM compression_dictionary WHERE id=1",
                [],
                |r| r.get::<_, usize>(0)
            )
            .is_ok());
    }
    drop(chat);
    let reopened = Chat::open(&temp.0).unwrap();
    let db = reopened.store.lock().unwrap();
    assert_eq!(
        db.message(&identity(1).user_id, 1, &reopened).unwrap().id,
        "1"
    );
    assert_eq!(
        db.message(&identity(1).user_id, 205, &reopened).unwrap().id,
        "205"
    );
}

#[test]
fn directory_sync_is_claimed_once_a_minute_and_retried_after_failure() {
    use std::time::{Duration, Instant};
    let (chat, _temp) = chat();
    assert!(
        chat.claim_directory_sync(),
        "the first request fills the catalogue"
    );
    assert!(
        !chat.claim_directory_sync(),
        "a second request inside the window skips it"
    );
    chat.finish_directory_sync(false);
    assert!(
        chat.claim_directory_sync(),
        "an empty refresh must retry on the next request"
    );
    chat.finish_directory_sync(true);
    assert!(
        !chat.claim_directory_sync(),
        "a filled refresh keeps the claim"
    );
    *chat.users_synced.lock().unwrap() = Some(Instant::now() - Duration::from_secs(61));
    assert!(
        chat.claim_directory_sync(),
        "the window expires after a minute"
    );
}

#[tokio::test]
async fn reqwest_handshake_upgrades_like_the_launcher_client() {
    let temp = Temp::new();
    let state = AppState::new(crate::Config::default(), Arc::new(Fake))
        .with_chat(Chat::open(&temp.0).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            crate::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .get(format!("http://{addr}/api/v1/chat/ws"))
        .version(reqwest::Version::HTTP_11)
        .bearer_auth("good")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::SWITCHING_PROTOCOLS);
    server.abort();
}

fn reason(error: &ApiError) -> String {
    error
        .details
        .as_ref()
        .and_then(|d| d.get("reason"))
        .cloned()
        .unwrap_or_default()
}

#[tokio::test]
async fn guest_token_is_hashed_and_authenticates_read_only() {
    let (app, chat, temp) = app_with_chat();
    let (status, issued) = request_abs(
        &app,
        "POST",
        "/api/v1/chat/guest",
        None,
        json!({"name":"Nick_1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = issued["token"].as_str().unwrap().to_owned();
    assert!(token.len() >= 64, "32+ random bytes");
    assert_eq!(issued["user"]["role"], "guest");
    assert_eq!(issued["user"]["guest"], true);
    assert_eq!(issued["user"]["name"], "Nick_1");
    assert!(issued["expiresAt"].as_str().unwrap().ends_with('Z'));
    // The plaintext token is returned once and never stored.
    let hashed: Vec<Vec<u8>> = {
        let guard = chat.store.lock().unwrap();
        let mut stmt = guard.db.prepare("SELECT token FROM guest_tokens").unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(hashed.len(), 1);
    assert_eq!(hashed[0].len(), 32);
    assert_ne!(hashed[0], token.as_bytes());
    let (status, me) = request_abs(
        &app,
        "GET",
        "/api/v1/chat/me",
        Some(&format!("Bearer {token}")),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["uuid"], issued["user"]["uuid"]);
    assert_eq!(me["role"], "guest");
    let (status, error) = request_abs(
        &app,
        "GET",
        "/api/v1/chat/me",
        Some("Bearer not-a-token"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error["code"], "authExpired");
    // Re-login with the same nick keeps the profile, a new token works too.
    let (status, again) = request_abs(
        &app,
        "POST",
        "/api/v1/chat/guest",
        None,
        json!({"name":"nick_1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["user"]["uuid"], issued["user"]["uuid"]);
    assert_ne!(again["token"], issued["token"]);
    drop(temp);
}

#[tokio::test]
async fn guest_login_validates_the_nickname() {
    let (app, _chat, _temp) = app_with_chat();
    let (status, _) = request_abs(
        &app,
        "POST",
        "/api/v1/chat/guest",
        None,
        json!({"name":"Nick_1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Per-IP limits are shared, so the invalid names get their own app.
    let (app, _chat, _temp) = app_with_chat();
    for name in ["", "ab", "has space", "semi;colon", &"x".repeat(17)] {
        let (status, error) = request_abs(
            &app,
            "POST",
            "/api/v1/chat/guest",
            None,
            json!({"name":name}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}");
        assert_eq!(error["code"], "invalidInput");
    }
}

#[test]
fn guest_reads_global_but_cannot_write_anywhere() {
    let (chat, _temp) = chat();
    let mut db = chat.store.lock().unwrap();
    seed(&db);
    let guest = db.guest_login("GuestNick", &chat).unwrap().user.uuid;
    let a = identity(1).user_id;
    let message = db.send(&a, "global", text("hello all"), &chat).unwrap();
    // Reading the global chat works, both history and the channel list.
    let history = db.history(&guest, "global", None, 50, &chat).unwrap();
    assert_eq!(history.messages.len(), 1);
    let channels = db
        .handle(
            &guest,
            "GET",
            &["channels"],
            &Value::Null,
            Value::Null,
            &chat,
        )
        .unwrap();
    assert_eq!(
        channels
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("global")]
    );
    // Writing to the global chat is refused with the documented reason.
    let a_id = a.as_str();
    let message_id = message.id.as_str();
    for (method, path, body) in [
        (
            "POST",
            vec!["channels", "global", "messages"],
            json!({"type":"text","text":"hi"}),
        ),
        ("POST", vec!["users", a_id, "direct"], Value::Null),
        ("GET", vec!["friends"], Value::Null),
        ("PUT", vec!["friends", a_id], Value::Null),
        ("GET", vec!["privacy"], Value::Null),
        ("PUT", vec!["privacy"], json!({"whoCanMessage":"nobody"})),
        ("PATCH", vec!["profile"], json!({"displayName":"Guest"})),
        ("POST", vec!["messages", message_id, "report"], Value::Null),
    ] {
        let error = db
            .handle(&guest, method, &path, &Value::Null, body, &chat)
            .unwrap_err();
        assert_eq!(error.status, StatusCode::FORBIDDEN, "{method} {path:?}");
        assert_eq!(error.code, "permissionDenied");
        assert_eq!(reason(&error), "guestReadOnly", "{method} {path:?}");
    }
    // Direct audience sees the guest as unverified and cannot message it.
    let view = db.user(&a, &guest, &chat).unwrap();
    assert!(view.guest);
    assert_eq!(view.role, SocialRole::Guest);
    assert!(!view.can_message && !view.can_invite);
    // A chat ban revokes the guest token.
    db.db
        .execute("INSERT INTO moderation(user,ban) VALUES(?1,1)", [&guest])
        .unwrap();
    assert_eq!(db.role(&guest).unwrap(), SocialRole::Guest);
}

#[tokio::test]
async fn guest_avatar_and_ban_revocation() {
    let (chat, _temp) = chat();
    let issued = chat.guest_login("GuestNick").await.unwrap();
    let guest = issued.user.uuid.clone();
    let error = chat.clear_avatar(guest.clone()).await.unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(reason(&error), "guestReadOnly");
    let error = chat
        .set_avatar(guest.clone(), b"not an image".to_vec())
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert!(chat.guest_identity(&issued.token).await.unwrap().is_some());
    chat.store
        .lock()
        .unwrap()
        .db
        .execute("INSERT INTO moderation(user,ban) VALUES(?1,1)", [&guest])
        .unwrap();
    let error = chat.guest_identity(&issued.token).await.unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn presence_counts_unique_clients_and_expires() {
    let (chat, _temp) = chat();
    let a = uuid::Uuid::new_v4().simple().to_string();
    let b = uuid::Uuid::new_v4().simple().to_string();
    let c = uuid::Uuid::new_v4().simple().to_string();
    assert_eq!(chat.online().online, 0);
    chat.beat(&a, "yggdrasil").unwrap();
    assert_eq!(chat.online().online, 1);
    // A repeated beat extends the entry instead of adding one.
    chat.beat(&a, "offline").unwrap();
    assert_eq!(chat.online().online, 1);
    chat.beat(&b, "anonymous").unwrap();
    assert_eq!(chat.online().online, 2);
    assert_eq!(
        chat.beat(&b, "bogus").unwrap_err().status,
        StatusCode::BAD_REQUEST
    );
    // Two missed heartbeats drop a client.
    let future = Instant::now() + presence::PRESENCE_TTL + Duration::from_secs(1);
    chat.beat_at(&c, presence::Kind::Anonymous, future).unwrap();
    assert_eq!(chat.online_at(future).online, 1);
    assert_eq!(chat.online().online, 2 + 1);
}

#[tokio::test]
async fn presence_endpoints_are_public_and_count_once() {
    let (app, _chat, _temp) = app_with_chat();
    let client = uuid::Uuid::new_v4().to_string();
    for _ in 0..2 {
        let (status, body) = request_abs(
            &app,
            "POST",
            "/api/v1/presence/beat",
            None,
            json!({"clientId":client,"kind":"yggdrasil"}),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(body, Value::Null);
    }
    let (status, summary) = request_abs(&app, "GET", "/api/v1/presence", None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(summary, json!({"online":1}));
    let (status, _) = request_abs(
        &app,
        "POST",
        "/api/v1/presence/beat",
        None,
        json!({"clientId":uuid::Uuid::new_v4().to_string(),"kind":"anonymous"}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, summary) = request_abs(&app, "GET", "/api/v1/presence", None, Value::Null).await;
    assert_eq!(summary["online"], 2);
    for body in [json!({"clientId":"not-a-uuid"}), json!({})] {
        let (status, error) = request_abs(&app, "POST", "/api/v1/presence/beat", None, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["code"], "invalidInput");
    }
}

#[test]
fn profile_patch_validates_fields_and_is_visible_to_others() {
    let (chat, _temp) = chat();
    let mut db = chat.store.lock().unwrap();
    seed(&db);
    let a = identity(1).user_id;
    let b = identity(2).user_id;
    for body in [
        json!({"displayName":"x".repeat(25)}),
        json!({"bio":"x".repeat(121)}),
        json!({"accent":"rainbow"}),
        json!({"bio":"line\nbreak"}),
    ] {
        let error = db
            .handle(&a, "PATCH", &["profile"], &Value::Null, body.clone(), &chat)
            .unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST, "{body}");
    }
    let user = db
        .handle(
            &a,
            "PATCH",
            &["profile"],
            &Value::Null,
            json!({"displayName":"  Ded  ","bio":"hello","accent":"teal"}),
            &chat,
        )
        .unwrap();
    assert_eq!(user["displayName"], "Ded");
    assert_eq!(user["bio"], "hello");
    assert_eq!(user["accent"], "teal");
    // Everyone sees the profile in `SocialUser`.
    let seen = db
        .handle(
            &b,
            "GET",
            &["profile", a.as_str()],
            &Value::Null,
            Value::Null,
            &chat,
        )
        .unwrap();
    assert_eq!(seen["displayName"], "Ded");
    assert_eq!(seen["accent"], "teal");
    assert_eq!(
        db.user(&b, &a, &chat).unwrap().bio.as_deref(),
        Some("hello")
    );
    // An empty string clears a field; untouched fields stay.
    let user = db
        .handle(
            &a,
            "PATCH",
            &["profile"],
            &Value::Null,
            json!({"displayName":"","accent":""}),
            &chat,
        )
        .unwrap();
    assert!(user["displayName"].is_null());
    assert!(user["accent"].is_null());
    assert_eq!(user["bio"], "hello");
    assert_eq!(
        db.handle(
            &b,
            "GET",
            &["profile", "not-a-uuid"],
            &Value::Null,
            Value::Null,
            &chat
        )
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn admin_audit_records_roles_moderation_and_deletions() {
    let (chat, _temp) = chat();
    let mut db = chat.store.lock().unwrap();
    seed(&db);
    let admin = identity(1).user_id;
    let target = identity(2).user_id;
    db.db
        .execute("UPDATE users SET admin=1 WHERE uuid=?1", [&admin])
        .unwrap();
    assert_eq!(
        db.handle(
            &admin,
            "GET",
            &["admin", "audit"],
            &Value::Null,
            Value::Null,
            &chat
        )
        .unwrap(),
        json!([])
    );
    assert_eq!(
        db.handle(
            &target,
            "GET",
            &["admin", "audit"],
            &Value::Null,
            Value::Null,
            &chat
        )
        .unwrap_err()
        .status,
        StatusCode::FORBIDDEN
    );
    db.handle(
        &admin,
        "PUT",
        &["admin", "roles", &target],
        &Value::Null,
        json!({"role":"moderator"}),
        &chat,
    )
    .unwrap();
    db.handle(
        &admin,
        "POST",
        &["mod", "users", &target],
        &Value::Null,
        json!({"action":"mute","seconds":600}),
        &chat,
    )
    .unwrap();
    db.handle(
        &admin,
        "POST",
        &["mod", "users", &target],
        &Value::Null,
        json!({"action":"ban"}),
        &chat,
    )
    .unwrap();
    let spam = db
        .send(&admin, "global", text("look at my link"), &chat)
        .unwrap();
    db.handle(
        &target,
        "DELETE",
        &["messages", &spam.id],
        &Value::Null,
        Value::Null,
        &chat,
    )
    .unwrap();
    let audit = db
        .handle(
            &admin,
            "GET",
            &["admin", "audit"],
            &Value::Null,
            Value::Null,
            &chat,
        )
        .unwrap();
    let entries: Vec<AuditEntry> = serde_json::from_value(audit).unwrap();
    let actions: Vec<&str> = entries.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(
        actions,
        vec!["delete", "ban", "mute", "role"],
        "newest first"
    );
    assert!(entries
        .iter()
        .all(|e| e.actor == "Player1" || e.actor == "Player2"));
    assert_eq!(entries[0].actor, "Player2", "the moderator deleted");
    assert_eq!(entries[3].actor, "Player1", "the admin changed the role");
    assert_eq!(entries[3].target.as_deref(), Some(target.as_str()));
    assert_eq!(entries[3].note.as_deref(), Some("moderator"));
    assert_eq!(entries[2].note.as_deref(), Some("600"));
    assert_eq!(entries[0].target.as_deref(), Some(spam.id.as_str()));
    assert!(entries.iter().all(|e| e.at.ends_with('Z')));
}

#[tokio::test]
async fn avatar_is_reencoded_resized_and_size_bounded() {
    let noisy = |side: u32| {
        let mut buffer = Vec::with_capacity((side * side * 3) as usize);
        for i in 0..side * side {
            buffer.push((i.wrapping_mul(2_654_435_761) >> 13) as u8);
            buffer.push((i.wrapping_mul(40_503) >> 7) as u8);
            buffer.push((i.wrapping_mul(22_695_477) >> 11) as u8);
        }
        let image = image::RgbImage::from_raw(side, side, buffer).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        png.into_inner()
    };
    let png = noisy(240);
    assert!(png.len() > 16 * 1024, "the route limit must be raised");
    let webp = files::avatar(&png).unwrap();
    let decoded = image::load_from_memory(&webp).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (256, 256));
    assert!(webp.len() <= MAX_AVATAR);
    assert!(files::avatar(b"not an image").is_err());

    let (app, chat, _temp) = app_with_chat();
    let (status, bytes) = request_bytes(
        &app,
        "PUT",
        "/api/v1/chat/avatar",
        Some("Bearer good"),
        Body::from(png.clone()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{:?}",
        String::from_utf8_lossy(&bytes)
    );
    let user: SocialUser = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(user.uuid, identity(1).user_id);
    let stored: Option<String> = chat
        .store
        .lock()
        .unwrap()
        .db
        .query_row(
            "SELECT avatar FROM users WHERE uuid=?1",
            [&user.uuid],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stored.is_some());
    // Too large: refused before any decoding.
    let (status, error) = request_bytes(
        &app,
        "PUT",
        "/api/v1/chat/avatar",
        Some("Bearer good"),
        Body::from(vec![0u8; MAX_AVATAR + 1]),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        serde_json::from_slice::<Value>(&error).unwrap()["code"],
        "invalidInput"
    );
    // Clearing removes the blob pointer.
    let (status, bytes) = request_bytes(
        &app,
        "DELETE",
        "/api/v1/chat/avatar",
        Some("Bearer good"),
        Body::empty(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cleared: SocialUser = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(cleared.uuid, user.uuid);
    let stored: Option<String> = chat
        .store
        .lock()
        .unwrap()
        .db
        .query_row(
            "SELECT avatar FROM users WHERE uuid=?1",
            [&user.uuid],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stored.is_none());
    // Unauthenticated uploads are rejected before any image work.
    let (status, _) =
        request_bytes(&app, "PUT", "/api/v1/chat/avatar", None, Body::from(png)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn avatar_download_serves_the_stored_webp_to_other_players() {
    let (app, chat, _temp) = app_with_chat();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(64, 64)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let owner = identity(1).user_id;
    let (status, _) = request_bytes(
        &app,
        "PUT",
        "/api/v1/chat/avatar",
        Some("Bearer good"),
        Body::from(png.into_inner()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // No avatar yet: 404 instead of an empty body.
    let (status, error) = request_abs(
        &app,
        "GET",
        &format!("/api/v1/chat/avatar/{}", identity(2).user_id),
        Some("Bearer other"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], "notFound");
    // Another authenticated player reads the same bytes.
    let (status, bytes) = request_bytes(
        &app,
        "GET",
        &format!("/api/v1/chat/avatar/{owner}"),
        Some("Bearer other"),
        Body::empty(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(bytes.starts_with(b"RIFF"), "WebP container");
    let stored: String = chat
        .store
        .lock()
        .unwrap()
        .db
        .query_row("SELECT avatar FROM users WHERE uuid=?1", [&owner], |r| {
            r.get(0)
        })
        .unwrap();
    let path = chat.store.lock().unwrap().files.join(stored);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    // Unauthenticated requests never leak bytes.
    let (status, _) = request_bytes(
        &app,
        "GET",
        &format!("/api/v1/chat/avatar/{owner}"),
        None,
        Body::empty(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_presence_lists_beats_and_named_sockets() {
    let (app, chat, _temp) = app_with_chat();
    let (status, _) = request_abs(
        &app,
        "POST",
        "/api/v1/presence/beat",
        None,
        json!({"clientId":uuid::Uuid::new_v4().to_string(),"kind":"offline"}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    {
        let db = chat.store.lock().unwrap();
        db.identify(&identity(1), true, None).unwrap();
    }
    let (status, error) = request_abs(
        &app,
        "GET",
        "/api/v1/chat/admin/presence",
        Some("Bearer good"),
        Value::Null,
    )
    .await;
    // `good` is not an admin for the hub: the Drasl directory is not configured.
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error["code"], "permissionDenied");
    let listing = {
        let db = chat.store.lock().unwrap();
        presence::admin_presence(&chat, &db).unwrap()
    };
    assert_eq!(listing.online, 1);
    assert_eq!(listing.clients.len(), 1);
    assert_eq!(listing.clients[0].label, "anonymous");
    assert_eq!(listing.clients[0].kind, "guest");
    assert!(listing.clients[0].since.ends_with('Z'));
}
