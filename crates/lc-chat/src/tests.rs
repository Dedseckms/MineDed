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
    sockets: AtomicUsize,
    unauthorized: AtomicBool,
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
    }
}
async fn rest(
    State(state): State<Arc<Server>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> AxumResponse {
    assert_eq!(headers.get("authorization").unwrap(), "Bearer test-bearer");
    assert!(!uri.to_string().contains("test-bearer"));
    state.calls.lock().unwrap().push(uri.to_string());
    if state.unauthorized.load(Ordering::SeqCst) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if uri.path().ends_with("/me") {
        return Json(user()).into_response();
    }
    if uri.path().ends_with("/messages") {
        return Json(SocialHistory {
            messages: vec![],
            before: Some("older".into()),
        })
        .into_response();
    }
    if uri.path().ends_with("/channels") {
        return Json(Vec::<SocialChannel>::new()).into_response();
    }
    if uri.path().contains("/files/") {
        return vec![0u8; MAX_FILE + 1].into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}
async fn ws(
    State(state): State<Arc<Server>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> AxumResponse {
    assert_eq!(headers.get("authorization").unwrap(), "Bearer test-bearer");
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
    let root = std::env::temp_dir().join(format!("chat-test-{}", accounts.id));
    let downloader = Arc::new(
        lc_http::HttpDownloader::new(lc_http::DownloaderConfig {
            system_proxy: lc_http::SystemProxyMode::Off,
            ..Default::default()
        })
        .unwrap(),
    );
    let client = ChatClient::new(
        Paths::new(&root),
        downloader,
        accounts.clone(),
        accounts.clone(),
        settings,
        observer.clone(),
    );
    Fixture {
        client,
        accounts,
        observer,
        server,
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
    let session = f.client.session().await.unwrap();
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
    let session = f.client.session().await.unwrap();
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
    let session = f.client.session().await.unwrap();
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
