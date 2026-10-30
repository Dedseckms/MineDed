#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use axum::{
    extract::{State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response as AxumResponse},
    routing::get,
    Json, Router,
};
use lc_accounts::{LaunchAuth, Secret, UserType};
use lc_types::account::{Account, YggdrasilServer};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex as StdMutex,
};

struct Accounts {
    id: AccountId,
    valid: AtomicBool,
    official: AtomicBool,
}
#[async_trait]
impl AccountService for Accounts {
    async fn list(&self) -> AppResult<Vec<Account>> {
        Ok(vec![Account {
            id: self.id,
            kind: AccountKind::Yggdrasil,
            name: "player".into(),
            uuid: "abcd".into(),
            server: Some("https://auth.test/".into()),
            skin_url: None,
            is_default: true,
            valid: self.valid.load(Ordering::SeqCst),
        }])
    }
    async fn offline_add(&self, _: &str) -> AppResult<Account> {
        Err(invalid("unused"))
    }
    async fn yggdrasil_login(&self, _: &Url, _: &str, _: &str) -> AppResult<Account> {
        Err(invalid("unused"))
    }
    async fn yggdrasil_register(&self, _: &Url, _: &str, _: &str, _: &str) -> AppResult<Account> {
        Err(invalid("unused"))
    }
    async fn yggdrasil_servers(&self) -> AppResult<Vec<YggdrasilServer>> {
        Ok(vec![YggdrasilServer {
            name: "own".into(),
            api_root: "https://auth.test/".into(),
            official: self.official.load(Ordering::SeqCst),
        }])
    }
    async fn remove(&self, _: AccountId) -> AppResult<()> {
        self.valid.store(false, Ordering::SeqCst);
        Ok(())
    }
    async fn set_default(&self, _: AccountId) -> AppResult<()> {
        Ok(())
    }
}
#[async_trait]
impl AuthProvider for Accounts {
    async fn launch_auth(&self, id: Option<AccountId>) -> AppResult<LaunchAuth> {
        assert_eq!(id, Some(self.id));
        Ok(LaunchAuth {
            name: "player".into(),
            uuid: "abcd".into(),
            access_token: Secret::new("test-bearer"),
            user_type: UserType::Mojang,
            authlib: None,
        })
    }
}
#[derive(Default)]
struct Observer(StdMutex<Vec<SocialEvent>>);
impl SocialObserver for Observer {
    fn on_event(&self, event: SocialEvent) {
        self.0.lock().unwrap().push(event);
    }
}
#[derive(Default)]
struct Server {
    calls: StdMutex<Vec<String>>,
    auths: StdMutex<Vec<String>>,
    bodies: StdMutex<Vec<(String, Value)>>,
    sockets: AtomicUsize,
    unauthorized: AtomicBool,
    deny: AtomicBool,
}
fn user() -> SocialUser {
    SocialUser {
        is_blocked: false,
        uuid: "abcd".into(),
        name: "player".into(),
        skin_url: None,
        status: Some(SocialStatus::Online),
        role: SocialRole::User,
        is_friend: false,
        can_message: true,
        can_invite: true,
        guest: false,
        display_name: None,
        bio: None,
        accent: None,
    }
}
fn guest_user() -> SocialUser {
    SocialUser {
        uuid: "guest-uuid".into(),
        name: "guest_1".into(),
        role: SocialRole::Guest,
        guest: true,
        ..user()
    }
}
async fn rest(
    State(state): State<Arc<Server>>,
    method: axum::http::Method,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> AxumResponse {
    let path = uri.path().to_owned();
    let auth = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(!uri.to_string().contains("test-bearer"));
    assert!(!uri.to_string().contains("guest-token"));
    state.calls.lock().unwrap().push(format!("{method} {uri}"));
    state.auths.lock().unwrap().push(auth.clone());
    if !body.is_empty() {
        if let Ok(value) = serde_json::from_slice::<Value>(&body) {
            state.bodies.lock().unwrap().push((path.clone(), value));
        }
    }
    // Public endpoints.
    if path == "/api/v1/presence" {
        return Json(PresenceSummary { online: 7 }).into_response();
    }
    if path == "/api/v1/presence/beat" {
        return StatusCode::NO_CONTENT.into_response();
    }
    if path == "/api/v1/chat/guest" {
        return Json(json!({
            "token": "guest-token",
            "user": guest_user(),
            "expiresAt": "2026-11-29T00:00:00Z",
        }))
        .into_response();
    }
    if auth != "Bearer test-bearer" && auth != "Bearer guest-token" {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if state.unauthorized.load(Ordering::SeqCst) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if state.deny.load(Ordering::SeqCst) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "code": "permissionDenied",
                "message": "Guests can read the global chat only",
                "details": { "reason": "guestReadOnly" },
            })),
        )
            .into_response();
    }
    if path.ends_with("/me") {
        return Json(user()).into_response();
    }
    if path.contains("/chat/avatar/") {
        return (
            [(axum::http::header::CONTENT_TYPE, "image/png")],
            b"\x89PNG".to_vec(),
        )
            .into_response();
    }
    if path.ends_with("/profile") || path.ends_with("/avatar") {
        return Json(user()).into_response();
    }
    if path.ends_with("/admin/presence") {
        return Json(AdminPresence::default()).into_response();
    }
    if path.ends_with("/admin/audit") {
        return Json(Vec::<AuditEntry>::new()).into_response();
    }
    if path.ends_with("/messages") {
        return Json(SocialHistory {
            messages: vec![],
            before: Some("older".into()),
        })
        .into_response();
    }
    if path.ends_with("/channels") {
        return Json(Vec::<SocialChannel>::new()).into_response();
    }
    if path.ends_with("/chat/files/pic") {
        return (
            [(axum::http::header::CONTENT_TYPE, "image/webp")],
            b"RIFF0000WEBP".to_vec(),
        )
            .into_response();
    }
    if path.ends_with("/chat/files/doc") {
        return (
            [(axum::http::header::CONTENT_TYPE, "application/pdf")],
            b"%PDF".to_vec(),
        )
            .into_response();
    }
    if path.contains("/files/") {
        return vec![0u8; MAX_FILE + 1].into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}
async fn ws(
    State(state): State<Arc<Server>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> AxumResponse {
    let auth = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    assert!(auth == "Bearer test-bearer" || auth == "Bearer guest-token");
    state.sockets.fetch_add(1, Ordering::SeqCst);
    upgrade
        .on_upgrade(move |mut socket| async move {
            if let Some(Ok(axum::extract::ws::Message::Text(text))) = socket.recv().await {
                let value: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(value["type"], "presence");
            }
            let _ = socket.send(axum::extract::ws::Message::Close(None)).await;
        })
        .into_response()
}
struct Fixture {
    client: ChatClient,
    accounts: Arc<Accounts>,
    observer: Arc<Observer>,
    server: Arc<Server>,
    settings: Arc<lc_runtime::MemorySettingsStore>,
    downloader: Arc<dyn Downloader>,
    tokens: Arc<lc_accounts::MemoryTokenStore>,
    task: JoinHandle<()>,
    root: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
async fn fixture() -> Fixture {
    let server = Arc::new(Server::default());
    let app = Router::new()
        .route("/api/v1/chat/ws", get(ws))
        .fallback(rest)
        .with_state(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let accounts = Arc::new(Accounts {
        id: AccountId::new(),
        valid: AtomicBool::new(true),
        official: AtomicBool::new(true),
    });
    let settings = Arc::new(lc_runtime::MemorySettingsStore::default());
    let mut value = settings.get().await.unwrap();
    value.hub_url = format!("http://{addr}/api/v1");
    settings.set(value).await.unwrap();
    let observer = Arc::new(Observer::default());
    let tokens = Arc::new(lc_accounts::MemoryTokenStore::new());
    let root = std::env::temp_dir().join(format!("chat-test-{}", accounts.id));
    let downloader: Arc<dyn Downloader> = Arc::new(
        lc_http::HttpDownloader::new(lc_http::DownloaderConfig {
            system_proxy: lc_http::SystemProxyMode::Off,
            ..Default::default()
        })
        .unwrap(),
    );
    let client = ChatClient::new(
        Paths::new(&root),
        downloader.clone(),
        accounts.clone(),
        accounts.clone(),
        settings.clone(),
        observer.clone(),
    )
    .with_token_store(tokens.clone());
    Fixture {
        client,
        accounts,
        observer,
        server,
        settings,
        downloader,
        tokens,
        task,
        root,
    }
}

#[tokio::test]
async fn auth_history_reconnect_and_disconnect_are_account_scoped() {
    let f = fixture().await;
    assert_eq!(f.client.connect(f.accounts.id).await.unwrap(), user());
    let page = f
        .client
        .history(&"dm:a/b:c".into(), Some("x&y".into()))
        .await
        .unwrap();
    assert_eq!(page.before.as_deref(), Some("older"));
    assert!(f
        .server
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|s| s.contains("dm:a%2Fb:c") && s.contains("before=x%26y")));
    let session = f.client.current_session().await.unwrap();
    assert!(session.cache.exists());
    tokio::time::timeout(Duration::from_secs(6), async {
        while f.server.sockets.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(f
        .observer
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|event| matches!(event, SocialEvent::Connection { connected: true })));
    f.client.disconnect().await.unwrap();
    assert!(!session.cache.exists());
    assert!(session.cancel.is_cancelled());
    assert_eq!(
        f.client.channels().await.unwrap_err().code,
        ErrorCode::AuthRequired
    );
    let sockets = f.server.sockets.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(sockets, f.server.sockets.load(Ordering::SeqCst));
}
#[tokio::test]
async fn third_party_accounts_and_logout_reject_before_network() {
    let f = fixture().await;
    f.accounts.official.store(false, Ordering::SeqCst);
    assert_eq!(
        f.client.connect(f.accounts.id).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert!(f.server.calls.lock().unwrap().is_empty());
    f.accounts.official.store(true, Ordering::SeqCst);
    f.client.connect(f.accounts.id).await.unwrap();
    let session = f.client.current_session().await.unwrap();
    f.client.history(&"global".into(), None).await.unwrap();
    f.accounts.remove(f.accounts.id).await.unwrap();
    assert_eq!(
        f.client.channels().await.unwrap_err().code,
        ErrorCode::AuthExpired
    );
    assert!(!session.cache.exists());
}
#[tokio::test]
async fn expired_bearer_cancels_session_and_file_bounds_are_enforced() {
    let f = fixture().await;
    f.client.connect(f.accounts.id).await.unwrap();
    tokio::fs::create_dir_all(&f.root).await.unwrap();
    let path = f.root.join("large.bin");
    let file = tokio::fs::File::create(&path).await.unwrap();
    file.set_len(MAX_FILE as u64 + 1).await.unwrap();
    drop(file);
    let sink = lc_runtime::NullSink::default();
    assert_eq!(
        f.client
            .send_file(&"global".into(), path.to_str().unwrap(), None, &sink)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    sink.cancel_token().cancel();
    assert_eq!(
        f.client
            .save_attachment("file", path.to_str().unwrap(), &sink)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    let session = f.client.current_session().await.unwrap();
    f.server.unauthorized.store(true, Ordering::SeqCst);
    assert_eq!(
        f.client.channels().await.unwrap_err().code,
        ErrorCode::AuthExpired
    );
    assert!(session.cancel.is_cancelled());
}
#[test]
fn endpoint_escapes_segments_and_never_accepts_a_new_origin() {
    let base = Url::parse("https://hub.test/api/v1/").unwrap();
    let url = endpoint(&base, &["users", "https://evil.test/x?token=abc", "direct"]).unwrap();
    assert_eq!(url.host_str(), Some("hub.test"));
    assert_eq!(url.query(), None);
    assert!(url.path().contains("https:%2F%2Fevil.test"));
}

fn reason(error: &AppError) -> &str {
    error
        .details
        .as_ref()
        .and_then(|details| details.get("reason"))
        .map(String::as_str)
        .unwrap_or_default()
}

#[tokio::test]
async fn guests_are_read_only_and_use_the_guest_token() {
    let f = fixture().await;
    let err = f.client.guest_login("ab").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(f.server.calls.lock().unwrap().is_empty());

    let guest = f.client.guest_login("guest_1").await.unwrap();
    assert!(guest.guest);
    assert_eq!(guest.role, SocialRole::Guest);
    assert!(f.client.current_session().await.unwrap().is_guest());

    let calls = f.server.calls.lock().unwrap().len();
    let send = f
        .client
        .send(&"global".into(), SocialContent::Text { text: "hi".into() })
        .await
        .unwrap_err();
    assert_eq!(send.code, ErrorCode::PermissionDenied);
    assert_eq!(reason(&send), "guestReadOnly");
    assert_eq!(
        f.client.open_direct("abcd").await.unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        f.client.add_friend("abcd").await.unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        f.client.friends().await.unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        f.client
            .update_profile(ProfileUpdate {
                display_name: Some("hi".into()),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        f.client.clear_avatar().await.unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    // Nothing reached the hub for any of the refused calls.
    assert_eq!(f.server.calls.lock().unwrap().len(), calls);

    // Reading the global chat is allowed with the session guest token.
    f.client.history(&"global".into(), None).await.unwrap();
    assert!(f
        .server
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| call.contains("/chat/channels/global/messages")));
    assert!(f
        .server
        .auths
        .lock()
        .unwrap()
        .iter()
        .any(|auth| auth == "Bearer guest-token"));

    // The WebSocket authenticates with the same guest token.
    tokio::time::timeout(Duration::from_secs(6), async {
        while f.server.sockets.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    f.client.disconnect().await.unwrap();
    assert_eq!(
        f.client.channels().await.unwrap_err().code,
        ErrorCode::AuthRequired
    );
}

#[tokio::test]
async fn profile_edits_are_validated_client_side() {
    let f = fixture().await;
    f.client.connect(f.accounts.id).await.unwrap();
    let calls = f.server.calls.lock().unwrap().len();
    let invalid = [
        ProfileUpdate {
            display_name: Some("x".repeat(MAX_DISPLAY_NAME + 1)),
            ..Default::default()
        },
        ProfileUpdate {
            bio: Some("b".repeat(MAX_BIO + 1)),
            ..Default::default()
        },
        ProfileUpdate {
            accent: Some("Not Accent".into()),
            ..Default::default()
        },
    ];
    for patch in invalid {
        let err = f.client.update_profile(patch).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }
    assert_eq!(f.server.calls.lock().unwrap().len(), calls);

    let updated = f
        .client
        .update_profile(ProfileUpdate {
            display_name: Some("Player".into()),
            bio: Some("hello".into()),
            accent: Some("green".into()),
        })
        .await
        .unwrap();
    assert_eq!(updated, user());
    assert!(f
        .server
        .calls
        .lock()
        .unwrap()
        .last()
        .is_some_and(|call| call.starts_with("PATCH") && call.contains("/chat/profile")));
}

#[tokio::test]
async fn hub_errors_map_to_app_errors() {
    let f = fixture().await;
    f.client.connect(f.accounts.id).await.unwrap();
    f.server.deny.store(true, Ordering::SeqCst);
    let err = f.client.admin_presence().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert_eq!(reason(&err), "guestReadOnly");
    f.server.deny.store(false, Ordering::SeqCst);
    assert!(f.client.audit().await.unwrap().is_empty());
}

#[tokio::test]
async fn presence_is_public_and_the_beat_reports_the_active_account() {
    let f = fixture().await;
    assert_eq!(f.client.presence().await.unwrap().online, 7);
    assert!(f.server.auths.lock().unwrap().iter().all(|a| a.is_empty()));

    beat(
        f.accounts.as_ref(),
        f.settings.as_ref(),
        f.downloader.as_ref(),
        "client-1",
        &CancelToken::new(),
    )
    .await
    .unwrap();
    let bodies = f.server.bodies.lock().unwrap();
    let (path, body) = bodies
        .iter()
        .find(|(path, _)| path == "/api/v1/presence/beat")
        .unwrap();
    assert_eq!(path, "/api/v1/presence/beat");
    assert_eq!(body["clientId"], "client-1");
    assert_eq!(body["kind"], "yggdrasil");
    drop(bodies);
    assert!(f
        .server
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| { call.starts_with("POST") && call.contains("/api/v1/presence/beat") }));
}

#[tokio::test]
async fn heartbeat_is_started_only_once_per_process() {
    let f = fixture().await;
    assert!(HEARTBEAT_CLAIMED.load(Ordering::SeqCst));
    let claimed = AtomicBool::new(false);
    let first = spawn_heartbeat(
        &claimed,
        f.accounts.clone(),
        f.settings.clone(),
        f.downloader.clone(),
    );
    assert!(first.is_some());
    let second = spawn_heartbeat(
        &claimed,
        f.accounts.clone(),
        f.settings.clone(),
        f.downloader.clone(),
    );
    assert!(second.is_none());
    // Creating more clients never adds a second loop to the process.
    let another = ChatClient::new(
        Paths::new(f.root.join("second")),
        f.downloader.clone(),
        f.accounts.clone(),
        f.accounts.clone(),
        f.settings.clone(),
        f.observer.clone(),
    );
    assert_eq!(another.presence().await.unwrap().online, 7);
    assert!(HEARTBEAT_LOOPS.load(Ordering::SeqCst) <= 1);
}

#[tokio::test]
async fn guest_session_is_restored_from_the_keyring_without_the_hub() {
    let f = fixture().await;
    let guest = f.client.guest_login("guest_1").await.unwrap();
    assert_eq!(f.client.session().await.unwrap(), Some(guest.clone()));

    // A fresh client sharing the keyring store restores the same guest session.
    let fresh = ChatClient::new(
        Paths::new(f.root.join("fresh")),
        f.downloader.clone(),
        f.accounts.clone(),
        f.accounts.clone(),
        f.settings.clone(),
        f.observer.clone(),
    )
    .with_token_store(f.tokens.clone());
    let calls = f.server.calls.lock().unwrap().len();
    assert_eq!(fresh.session().await.unwrap(), Some(guest));
    assert_eq!(f.server.calls.lock().unwrap().len(), calls);
    assert!(fresh.current_session().await.unwrap().is_guest());
}

#[tokio::test]
async fn attachment_original_returns_data_urls_for_images_only() {
    let f = fixture().await;
    f.client.connect(f.accounts.id).await.unwrap();
    let url = f.client.attachment_original("pic").await.unwrap();
    assert!(url.starts_with("data:image/webp;base64,"));
    assert_eq!(
        f.client.attachment_original("doc").await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
    let avatar = f.client.avatar_preview("abcd").await.unwrap();
    assert!(avatar.starts_with("data:image/png;base64,"));
}
