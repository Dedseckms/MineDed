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
fn app() -> (Router, Temp) {
    let temp = Temp::new();
    let state = AppState::new(crate::Config::default(), Arc::new(Fake))
        .with_chat(Chat::open(&temp.0).unwrap());
    (crate::router(state), temp)
}
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: bool,
    body: Value,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(format!("/api/v1/chat/{path}"));
    if token {
        req = req.header("authorization", "Bearer good");
    }
    let response = app
        .clone()
        .oneshot(
            req.body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
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
