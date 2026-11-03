use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex, OnceLock,
    },
    time::Duration,
};

use crate::{ChatService, SocialObserver};
use async_trait::async_trait;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use lc_accounts::{AccountService, AuthProvider, KeyringTokenStore, TokenStore};
use lc_http::{Downloader, Route};
use lc_runtime::{CancelToken, Paths, ProgressSink, SettingsStore};
use lc_types::{
    account::AccountKind, social::*, AccountId, AppError, AppResult, ErrorCode, Secret,
};
use reqwest::{Method, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{watch, Mutex, RwLock},
    task::JoinHandle,
};
use tokio_tungstenite::{
    tungstenite::{
        handshake::{client::generate_key, derive_accept_key},
        protocol::{Role, WebSocketConfig},
        Message,
    },
    WebSocketStream,
};
use url::Url;

const MAX_FILE: usize = 10 * 1024 * 1024;
const MAX_JSON: usize = 4 * 1024 * 1024;
const MAX_AVATAR: u64 = 2 * 1024 * 1024;
/// Hub avatars are re-encoded to a 256×256 WebP; anything bigger is refused.
const MAX_AVATAR_PREVIEW: usize = 1024 * 1024;
/// Avatar previews kept in process memory; bytes are never written to disk.
const AVATAR_CACHE_ENTRIES: usize = 64;
const AVATAR_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_DISPLAY_NAME: usize = 24;
const MAX_BIO: usize = 120;
const MAX_ACCENT: usize = 16;
const BEAT_INTERVAL: Duration = Duration::from_secs(60);

fn thumbnail_data_url(bytes: &[u8]) -> AppResult<String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(invalid("thumbnailType"));
    }
    Ok(format!(
        "data:image/webp;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[test]
fn thumbnail_rejects_active_content_and_wrong_container() {
    assert!(thumbnail_data_url(b"<svg onload='bad()'/>").is_err());
    assert!(thumbnail_data_url(b"RIFF0000WAVE").is_err());
    assert!(thumbnail_data_url(b"RIFF").is_err());
    assert!(thumbnail_data_url(b"RIFF0000WEBP").is_ok());
}

#[test]
fn avatar_cache_is_bounded_and_evicts_the_least_recently_used() {
    let mut cache = AvatarCache::new(4, 1024);
    for i in 0..10 {
        cache.insert(format!("k{i}"), format!("data:image/webp;base64,{i}"));
    }
    assert_eq!(cache.len(), 4);
    assert!(cache.get("k0").is_none());
    assert!(cache.get("k9").is_some());
    // Reading an entry keeps it alive, so the next insert evicts its neighbour.
    cache.get("k6");
    cache.insert("k10".into(), "data:image/webp;base64,x".into());
    assert!(cache.get("k6").is_some());
    assert!(cache.get("k7").is_none());

    // The byte budget also holds: one oversized preview is never cached.
    let mut small = AvatarCache::new(64, 32);
    small.insert("big".into(), "x".repeat(64));
    assert_eq!(small.len(), 0);
    small.insert("a".into(), "a".repeat(20));
    small.insert("b".into(), "b".repeat(20));
    assert_eq!(small.len(), 1);
    assert!(small.get("b").is_some());
}

/// Bounded least-recently-used cache of encoded avatar previews.
///
/// Avatars are only ever held in process memory for as long as they are used:
/// the oldest entry is dropped as soon as either the entry or the byte budget
/// is exceeded, and the whole cache is cleared whenever the session changes.
struct AvatarCache {
    max_entries: usize,
    max_bytes: usize,
    bytes: usize,
    tick: u64,
    entries: HashMap<String, (String, u64)>,
}

impl AvatarCache {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries,
            max_bytes,
            bytes: 0,
            tick: 0,
            entries: HashMap::new(),
        }
    }
    /// Returns the cached preview and marks the entry as freshly used.
    fn get(&mut self, key: &str) -> Option<String> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        let entry = self.entries.get_mut(key)?;
        entry.1 = tick;
        Some(entry.0.clone())
    }
    fn insert(&mut self, key: String, value: String) {
        if value.len() > self.max_bytes {
            // A single oversized preview is served but never cached.
            return;
        }
        self.tick = self.tick.wrapping_add(1);
        if let Some((old, _)) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.len());
        }
        self.bytes += value.len();
        self.entries.insert(key, (value, self.tick));
        self.trim();
    }
    fn forget(&mut self, key: &str) {
        if let Some((old, _)) = self.entries.remove(key) {
            self.bytes = self.bytes.saturating_sub(old.len());
        }
    }
    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
    /// Evicts the least recently used entries until both budgets hold.
    fn trim(&mut self) {
        while self.entries.len() > self.max_entries || self.bytes > self.max_bytes {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, tick))| *tick)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.forget(&oldest);
        }
    }
    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

fn invalid(reason: &str) -> AppError {
    AppError::new(ErrorCode::InvalidInput, "Invalid chat input").detail("reason", reason)
}
fn network() -> AppError {
    AppError::new(ErrorCode::Network, "Chat request failed")
}
fn expired() -> AppError {
    AppError::new(
        ErrorCode::AuthExpired,
        "Chat account is no longer available",
    )
}
fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "Chat operation cancelled")
}
/// Guests are read-only in the hub; the client refuses locally instead of
/// sending a request the server would reject anyway.
fn guest_read_only() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "Guests can read the global chat only",
    )
    .detail("reason", "guestReadOnly")
}

/// How a session authenticates: a Yggdrasil account or a hub guest token.
#[derive(Clone)]
enum Credential {
    Account(AccountId),
    /// Kept in memory for the lifetime of the session only (never in the keyring).
    Guest(Secret),
}

struct Session {
    credential: Credential,
    base: Url,
    cancel: CancelToken,
    cache: PathBuf,
    presence: watch::Sender<SocialStatus>,
}
impl Session {
    fn is_guest(&self) -> bool {
        matches!(self.credential, Credential::Guest(_))
    }
    fn account(&self) -> Option<AccountId> {
        match self.credential {
            Credential::Account(id) => Some(id),
            Credential::Guest(_) => None,
        }
    }
}
struct Live {
    session: Arc<Session>,
    task: JoinHandle<()>,
}
impl Drop for Live {
    fn drop(&mut self) {
        self.session.cancel.cancel();
        self.task.abort();
    }
}

/// Process-wide presence heartbeat: started once, cancelled by `disconnect`.
struct Heartbeat {
    cancel: CancelToken,
    task: JoinHandle<()>,
}
impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}
static HEARTBEAT_CLAIMED: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static HEARTBEAT_LOOPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static HEARTBEAT: OnceLock<StdMutex<Option<Heartbeat>>> = OnceLock::new();

fn heartbeat_slot() -> &'static StdMutex<Option<Heartbeat>> {
    HEARTBEAT.get_or_init(|| StdMutex::new(None))
}

/// Starts the heartbeat if `claimed` was free and a runtime is available.
/// The caller owns the returned task; dropping it cancels the loop.
fn spawn_heartbeat(
    claimed: &AtomicBool,
    accounts: Arc<dyn AccountService>,
    settings: Arc<dyn SettingsStore>,
    downloader: Arc<dyn Downloader>,
) -> Option<Heartbeat> {
    if tokio::runtime::Handle::try_current().is_err() || claimed.swap(true, Ordering::SeqCst) {
        return None;
    }
    let cancel = CancelToken::new();
    let task = tokio::spawn(heartbeat_loop(
        accounts,
        settings,
        downloader,
        cancel.clone(),
    ));
    Some(Heartbeat { cancel, task })
}

/// Keeps the one process-wide heartbeat alive; later calls are no-ops.
fn start_heartbeat(
    accounts: Arc<dyn AccountService>,
    settings: Arc<dyn SettingsStore>,
    downloader: Arc<dyn Downloader>,
) {
    let Some(heartbeat) = spawn_heartbeat(&HEARTBEAT_CLAIMED, accounts, settings, downloader)
    else {
        return;
    };
    #[cfg(test)]
    HEARTBEAT_LOOPS.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut slot) = heartbeat_slot().lock() {
        *slot = Some(heartbeat);
    }
}

async fn heartbeat_loop(
    accounts: Arc<dyn AccountService>,
    settings: Arc<dyn SettingsStore>,
    downloader: Arc<dyn Downloader>,
    cancel: CancelToken,
) {
    let client_id = uuid::Uuid::new_v4().to_string();
    loop {
        // Best effort: a missing counter must never disturb the chat.
        let _ = beat(
            accounts.as_ref(),
            settings.as_ref(),
            downloader.as_ref(),
            &client_id,
            &cancel,
        )
        .await;
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(BEAT_INTERVAL) => {}
        }
    }
}

async fn beat(
    accounts: &dyn AccountService,
    settings: &dyn SettingsStore,
    downloader: &dyn Downloader,
    client_id: &str,
    cancel: &CancelToken,
) -> AppResult<()> {
    let base = hub_base(settings).await?;
    let kind = active_kind(accounts).await;
    let request = downloader
        .client(Route::Auto)
        .post(root_endpoint(&base, &["presence", "beat"])?)
        .json(&json!({ "clientId": client_id, "kind": kind }))
        .send();
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(cancelled()),
        r = request => r.map_err(|_| network())?,
    };
    checked(response, cancel).await?;
    Ok(())
}

/// Kind reported to the presence counter for the currently active account.
async fn active_kind(accounts: &dyn AccountService) -> &'static str {
    let Ok(list) = accounts.list().await else {
        return "anonymous";
    };
    let active = list
        .iter()
        .find(|a| a.is_default && a.valid)
        .or_else(|| list.iter().find(|a| a.valid));
    match active.map(|a| a.kind) {
        Some(AccountKind::Offline) => "offline",
        Some(AccountKind::Yggdrasil) => "yggdrasil",
        None => "anonymous",
    }
}

/// Authenticated hub client. One account owns each connection and cache namespace.
pub struct ChatClient {
    accounts: Arc<dyn AccountService>,
    auth: Arc<dyn AuthProvider>,
    downloader: Arc<dyn Downloader>,
    paths: Paths,
    settings: Arc<dyn SettingsStore>,
    observer: Arc<dyn SocialObserver>,
    live: RwLock<Option<Live>>,
    /// Identity of the current session (returned by `session()`).
    me: RwLock<Option<SocialUser>>,
    /// Bounded in-memory cache of avatar previews for the current session.
    avatars: StdMutex<AvatarCache>,
    /// Lazily opened keyring store for the persisted guest token.
    tokens: OnceLock<Arc<dyn TokenStore>>,
    lifecycle: Mutex<()>,
}
/// Keyring key of the persisted guest token for a hub (`chat-guest:<hash>`).
pub fn guest_key(base: &Url) -> String {
    format!("chat-guest:{:x}", Sha256::digest(base.as_str().as_bytes()))
}

impl ChatClient {
    /// Creates a disconnected client; credentials are refreshed for each request.
    pub fn new(
        paths: Paths,
        downloader: Arc<dyn Downloader>,
        accounts: Arc<dyn AccountService>,
        auth: Arc<dyn AuthProvider>,
        settings: Arc<dyn SettingsStore>,
        observer: Arc<dyn SocialObserver>,
    ) -> Self {
        start_heartbeat(accounts.clone(), settings.clone(), downloader.clone());
        Self {
            accounts,
            auth,
            downloader,
            paths,
            settings,
            observer,
            live: RwLock::new(None),
            me: RwLock::new(None),
            avatars: StdMutex::new(AvatarCache::new(AVATAR_CACHE_ENTRIES, AVATAR_CACHE_BYTES)),
            tokens: OnceLock::new(),
            lifecycle: Mutex::new(()),
        }
    }
    /// Overrides the keyring store (tests and alternative secure storage).
    pub fn with_token_store(self, tokens: Arc<dyn TokenStore>) -> Self {
        let _ = self.tokens.set(tokens);
        self
    }
    fn token_store(&self) -> Arc<dyn TokenStore> {
        self.tokens
            .get_or_init(|| Arc::new(KeyringTokenStore::new(&self.paths)))
            .clone()
    }
    async fn current_session(&self) -> AppResult<Arc<Session>> {
        let session = self
            .live
            .read()
            .await
            .as_ref()
            .map(|l| l.session.clone())
            .ok_or_else(|| AppError::new(ErrorCode::AuthRequired, "Connect to chat first"))?;
        if session.cancel.is_cancelled() {
            return Err(expired());
        }
        if let Some(account) = session.account() {
            if !own_account(self.accounts.as_ref(), account).await? {
                session.cancel.cancel();
                self.clear_cache(&session).await;
                return Err(expired());
            }
        }
        Ok(session)
    }
    /// Session that may use everything except the guest read-only surface.
    async fn full_session(&self) -> AppResult<Arc<Session>> {
        let session = self.current_session().await?;
        if session.is_guest() {
            return Err(guest_read_only());
        }
        Ok(session)
    }
    async fn token(&self, session: &Session) -> AppResult<Secret> {
        token_for(session, self.auth.as_ref()).await
    }
    async fn clear_cache(&self, s: &Session) {
        let _ = tokio::fs::remove_dir_all(&s.cache).await;
    }
    /// Tears down the current session (if any), its socket and cached history.
    async fn shutdown_current(&self) {
        self.clear_avatar_cache();
        if let Some(old) = self.live.write().await.take() {
            old.session.cancel.cancel();
            old.task.abort();
            self.clear_cache(&old.session).await;
        }
        *self.me.write().await = None;
    }
    /// Builds a session against the configured hub; the heartbeat is armed too.
    async fn start_session(&self, credential: Credential, seed: &str) -> AppResult<Arc<Session>> {
        // Preview bytes belong to the previous session and are dropped with it.
        self.clear_avatar_cache();
        start_heartbeat(
            self.accounts.clone(),
            self.settings.clone(),
            self.downloader.clone(),
        );
        let base = hub_base(self.settings.as_ref()).await?;
        let cache = self
            .paths
            .cache()
            .join("chat")
            .join(format!(
                "{:x}",
                Sha256::digest(format!("{base}:{seed}").as_bytes())
            ))
            .join(AccountId::new().to_string());
        Ok(Arc::new(Session {
            credential,
            base,
            cancel: CancelToken::new(),
            cache,
            presence: watch::channel(SocialStatus::Online).0,
        }))
    }
    async fn spawn_socket(&self, session: Arc<Session>) {
        let task = tokio::spawn(socket(
            session.clone(),
            self.accounts.clone(),
            self.auth.clone(),
            self.downloader.clone(),
            self.observer.clone(),
        ));
        *self.live.write().await = Some(Live { session, task });
    }
    async fn request(
        &self,
        s: &Session,
        method: Method,
        segments: &[&str],
        body: Option<Value>,
    ) -> AppResult<Response> {
        if s.cancel.is_cancelled() {
            return Err(cancelled());
        }
        let token = self.token(s).await?;
        let mut req = self
            .downloader
            .client(Route::Auto)
            .request(method, endpoint(&s.base, segments)?)
            .bearer_auth(token.expose());
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = tokio::select! { _ = s.cancel.cancelled() => return Err(cancelled()), r = req.send() => r.map_err(|_| network())? };
        checked(response, &s.cancel).await
    }
    async fn api<T: DeserializeOwned>(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<Value>,
    ) -> AppResult<T> {
        let s = self.current_session().await?;
        let r = self.request(&s, method, segments, body).await?;
        decode(r, &s.cancel).await
    }
    async fn empty(&self, method: Method, segments: &[&str], body: Option<Value>) -> AppResult<()> {
        let s = self.current_session().await?;
        self.request(&s, method, segments, body).await?;
        Ok(())
    }

    // ------------------------------------------------ session and originals
    fn guest_key(base: &Url) -> String {
        guest_key(base)
    }
    async fn load_guest(&self, base: &Url) -> Option<(Secret, SocialUser)> {
        let key = Self::guest_key(base);
        let store = self.token_store();
        let raw = store.get(&key).await.ok().flatten()?;
        match serde_json::from_str::<StoredGuest>(raw.expose()) {
            Ok(stored) => Some((Secret::new(stored.token), stored.user)),
            Err(_) => {
                let _ = store.delete(&key).await;
                None
            }
        }
    }
    async fn store_guest(&self, base: &Url, token: &Secret, user: &SocialUser) {
        let stored = StoredGuest {
            token: token.expose().to_owned(),
            user: user.clone(),
        };
        if let Ok(json) = serde_json::to_string(&stored) {
            let _ = self
                .token_store()
                .set(&Self::guest_key(base), &Secret::new(json))
                .await;
        }
    }
    /// Cached preview for `key`, marked as recently used.
    fn cached_avatar(&self, key: &str) -> Option<String> {
        self.avatars.lock().ok()?.get(key)
    }
    /// Stores a preview, evicting the least recently used entry when full.
    fn remember_avatar(&self, key: String, data: String) {
        if let Ok(mut cache) = self.avatars.lock() {
            cache.insert(key, data);
        }
    }
    /// Drops the preview of `uuid` (own avatar changed or was removed).
    fn forget_avatar(&self, base: &Url, uuid: &str) {
        if let Ok(mut cache) = self.avatars.lock() {
            cache.forget(&avatar_key(base, uuid));
        }
    }
    fn clear_avatar_cache(&self) {
        if let Ok(mut cache) = self.avatars.lock() {
            cache.clear();
        }
    }
}

/// Cache key: the hub origin plus the avatar owner, so a hub switch never
/// serves a preview fetched from another server.
fn avatar_key(base: &Url, uuid: &str) -> String {
    format!("{}|{uuid}", base.as_str())
}

/// Fetches an avatar and encodes it as a `data:` URL; bounded to 1 MiB and
/// restricted to image types.
async fn avatar_data_url(
    s: &Session,
    token: Secret,
    url: Url,
    downloader: &dyn Downloader,
) -> AppResult<String> {
    let response = tokio::select! {
        _ = s.cancel.cancelled() => return Err(cancelled()),
        r = downloader.client(Route::Auto).get(url).bearer_auth(token.expose()).send() => r.map_err(|_| network())?,
    };
    let response = checked(response, &s.cancel).await?;
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    // Only raster image types: SVG is active content and must never be inlined.
    if !mime.starts_with("image/") || mime == "image/svg+xml" {
        return Err(invalid("avatarType"));
    }
    let bytes = bounded(response, MAX_AVATAR_PREVIEW, &s.cancel).await?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Fetches an image and encodes it for the UI; never returns a public URL.
async fn image_data_url(
    s: &Session,
    token: Secret,
    url: Url,
    downloader: &dyn Downloader,
) -> AppResult<String> {
    let response = tokio::select! {
        _ = s.cancel.cancelled() => return Err(cancelled()),
        r = downloader.client(Route::Auto).get(url).bearer_auth(token.expose()).send() => r.map_err(|_| network())?,
    };
    let response = checked(response, &s.cancel).await?;
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    if !["image/png", "image/jpeg", "image/webp", "image/gif"].contains(&mime.as_str()) {
        return Err(invalid("attachmentType"));
    }
    let bytes = bounded(response, MAX_FILE, &s.cancel).await?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Guest token + public profile persisted in the OS keyring.
#[derive(Serialize, Deserialize)]
struct StoredGuest {
    token: String,
    user: SocialUser,
}

async fn token_for(s: &Session, auth: &dyn AuthProvider) -> AppResult<Secret> {
    match &s.credential {
        Credential::Account(id) => Ok(auth.launch_auth(Some(*id)).await?.access_token),
        Credential::Guest(token) => Ok(token.clone()),
    }
}

/// Hub API root from settings, defaulting to the project server.
async fn hub_base(settings: &dyn SettingsStore) -> AppResult<Url> {
    let settings = settings.get().await?;
    let raw = if settings.hub_url.trim().is_empty() {
        lc_types::settings::DEFAULT_HUB_URL
    } else {
        settings.hub_url.trim()
    };
    let base = Url::parse(raw).map_err(|_| invalid("hubUrl"))?;
    if (base.scheme() != "https"
        && !(base.scheme() == "http"
            && base.host_str().is_some_and(|h| {
                h == "localhost"
                    || h.parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })))
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err(invalid("hubUrl"));
    }
    Ok(base)
}

/// `^[A-Za-z0-9_]{3,16}$`: the hub applies the same rule.
fn valid_guest_name(name: &str) -> bool {
    (3..=16).contains(&name.chars().count())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Checked locally so obviously invalid edits never reach the hub.
fn validate_profile(patch: &ProfileUpdate) -> AppResult<()> {
    if let Some(name) = &patch.display_name {
        if name.chars().count() > MAX_DISPLAY_NAME || name.chars().any(char::is_control) {
            return Err(invalid("displayName"));
        }
    }
    if let Some(bio) = &patch.bio {
        if bio.chars().count() > MAX_BIO || bio.chars().any(|c| c.is_control() && c != '\n') {
            return Err(invalid("bio"));
        }
    }
    if let Some(accent) = &patch.accent {
        // Accent ids are short lowercase slugs from the shared palette.
        if accent.len() > MAX_ACCENT
            || !accent
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(invalid("accent"));
        }
    }
    Ok(())
}

async fn own_account(accounts: &dyn AccountService, id: AccountId) -> AppResult<bool> {
    let list = accounts.list().await?;
    let Some(account) = list
        .iter()
        .find(|a| a.id == id && a.valid && a.kind == AccountKind::Yggdrasil)
    else {
        return Ok(false);
    };
    let servers = accounts.yggdrasil_servers().await?;
    let Some(account_server) = account.server.as_deref().and_then(|s| Url::parse(s).ok()) else {
        return Ok(false);
    };
    Ok(servers.iter().any(|server| {
        server.official && Url::parse(&server.api_root).is_ok_and(|url| url == account_server)
    }))
}
/// Path under the hub API root (`/api/v1`), without the `/chat` prefix.
fn root_endpoint(base: &Url, segments: &[&str]) -> AppResult<Url> {
    let mut url = base.clone();
    {
        let mut path = url.path_segments_mut().map_err(|_| invalid("hubUrl"))?;
        path.pop_if_empty();
        for segment in segments {
            if segment.is_empty() || *segment == "." || *segment == ".." {
                return Err(invalid("pathSegment"));
            }
            path.push(segment);
        }
    }
    Ok(url)
}
fn endpoint(base: &Url, segments: &[&str]) -> AppResult<Url> {
    let mut prefixed = Vec::with_capacity(segments.len() + 1);
    prefixed.push("chat");
    prefixed.extend_from_slice(segments);
    root_endpoint(base, &prefixed)
}
async fn bounded(mut response: Response, max: usize, cancel: &CancelToken) -> AppResult<Vec<u8>> {
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err(invalid("responseTooLarge"));
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! { _ = cancel.cancelled() => return Err(cancelled()), r = response.chunk() => r.map_err(|_| network())? };
        let Some(chunk) = chunk else { break };
        if bytes.len().saturating_add(chunk.len()) > max {
            return Err(invalid("responseTooLarge"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn checked(response: Response, cancel: &CancelToken) -> AppResult<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let code = match status.as_u16() {
        401 => ErrorCode::AuthExpired,
        403 => ErrorCode::PermissionDenied,
        404 => ErrorCode::NotFound,
        409 => ErrorCode::Conflict,
        429 => ErrorCode::RateLimited,
        400 | 413 | 422 => ErrorCode::InvalidInput,
        _ => ErrorCode::Network,
    };
    let mut error = AppError::new(code, "Chat server rejected request");
    let mut server_message = None;
    if let Ok(bytes) = bounded(response, 16384, cancel).await {
        if let Ok(server) = serde_json::from_slice::<AppError>(&bytes) {
            server_message = Some(server.message.clone());
            if let Some(reason) = server.details.as_ref().and_then(|d| d.get("reason")) {
                if [
                    "filtered",
                    "muted",
                    "attachmentsPaused",
                    "guestReadOnly",
                    "profileInvalid",
                    "avatarRejected",
                ]
                .contains(&reason.as_str())
                {
                    error = error.detail("reason", reason);
                }
            }
        }
    }
    if let Some(message) = server_message {
        error = error.detail("serverMessage", message);
    }
    if status.as_u16() == 401 {
        cancel.cancel();
    }
    Err(error)
}
async fn decode<T: DeserializeOwned>(response: Response, cancel: &CancelToken) -> AppResult<T> {
    serde_json::from_slice(&bounded(response, MAX_JSON, cancel).await?)
        .map_err(|_| AppError::new(ErrorCode::Network, "Invalid chat response"))
}
fn value<T: Serialize>(v: T) -> AppResult<Value> {
    serde_json::to_value(v).map_err(|_| AppError::internal("Chat serialization failed"))
}

async fn socket(
    session: Arc<Session>,
    accounts: Arc<dyn AccountService>,
    auth: Arc<dyn AuthProvider>,
    downloader: Arc<dyn Downloader>,
    observer: Arc<dyn SocialObserver>,
) {
    let mut attempt = 0u32;
    while !session.cancel.is_cancelled() {
        let run = socket_once(
            &session,
            accounts.as_ref(),
            auth.as_ref(),
            downloader.as_ref(),
            observer.as_ref(),
        );
        let result = tokio::select! { _ = session.cancel.cancelled() => break, r = run => r };
        if result
            .as_ref()
            .is_err_and(|e| matches!(e.code, ErrorCode::AuthExpired | ErrorCode::AuthRequired))
        {
            session.cancel.cancel();
        }
        observer.on_event(SocialEvent::Connection { connected: false });
        attempt = attempt.saturating_add(1);
        let delay = Duration::from_secs(1u64 << attempt.min(5));
        tokio::select! { _ = session.cancel.cancelled() => break, _ = tokio::time::sleep(delay) => {} }
    }
    observer.on_event(SocialEvent::Connection { connected: false });
    let _ = tokio::fs::remove_dir_all(&session.cache).await;
}
async fn socket_once(
    s: &Session,
    accounts: &dyn AccountService,
    auth: &dyn AuthProvider,
    downloader: &dyn Downloader,
    observer: &dyn SocialObserver,
) -> AppResult<()> {
    if let Some(account) = s.account() {
        if !own_account(accounts, account).await? {
            s.cancel.cancel();
            return Err(expired());
        }
    }
    let token = token_for(s, auth).await?;
    let key = generate_key();
    let handshake = downloader
        .upgrade_client(Route::Auto)
        .get(endpoint(&s.base, &["ws"])?)
        .version(reqwest::Version::HTTP_11)
        .bearer_auth(token.expose())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", &key)
        .send();
    let response = tokio::time::timeout(Duration::from_secs(20), handshake)
        .await
        .map_err(|_| network())?
        .map_err(|_| network())?;
    if response.status().as_u16() != 101 {
        checked(response, &s.cancel).await?;
        return Err(network());
    }
    if response
        .headers()
        .get("sec-websocket-accept")
        .and_then(|h| h.to_str().ok())
        != Some(derive_accept_key(key.as_bytes()).as_str())
    {
        return Err(network());
    }
    let upgraded = tokio::time::timeout(Duration::from_secs(10), response.upgrade())
        .await
        .map_err(|_| network())?
        .map_err(|_| network())?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_JSON))
        .max_frame_size(Some(MAX_JSON));
    let mut ws = WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(config)).await;
    observer.on_event(SocialEvent::Connection { connected: true });
    let mut presence = s.presence.subscribe();
    let mut last_received = tokio::time::Instant::now();
    let initial_status = *presence.borrow_and_update();
    ws.send(Message::Text(
        json!({"type":"presence","status":initial_status})
            .to_string()
            .into(),
    ))
    .await
    .map_err(|_| network())?;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = s.cancel.cancelled() => { let _ = ws.close(None).await; return Ok(()); },
            _ = presence.changed() => {
                let status = *presence.borrow_and_update();
                ws.send(Message::Text(json!({"type":"presence","status":status}).to_string().into())).await.map_err(|_| network())?;
            },
            _ = heartbeat.tick() => {
                if last_received.elapsed() > Duration::from_secs(60) { return Err(network()); }
                if let Some(account) = s.account() {
                    if !own_account(accounts, account).await? { s.cancel.cancel(); return Err(expired()); }
                }
                tokio::time::timeout(Duration::from_secs(10), ws.send(Message::Ping(Vec::new().into()))).await.map_err(|_| network())?.map_err(|_| network())?;
            },
            frame = tokio::time::timeout(Duration::from_secs(60), ws.next()) => {
                last_received = tokio::time::Instant::now();
                match frame.map_err(|_| network())? {
                    Some(Ok(Message::Text(text))) => { let event: SocialEvent = serde_json::from_str(&text).map_err(|_| network())?; if matches!(event, SocialEvent::MessageUpdated { .. }) { let _ = tokio::fs::remove_dir_all(&s.cache).await; } observer.on_event(event); },
                    Some(Ok(Message::Ping(data))) => ws.send(Message::Pong(data)).await.map_err(|_| network())?,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Err(network()),
                    _ => {}
                }
            }
        }
    }
}
impl ChatClient {
    /// A server account on this device replaces any leftover guest session:
    /// the game account is the identity the player expects in chat.
    async fn stored_guest_id(&self) -> AppResult<Option<AccountId>> {
        let own: Vec<AccountId> = self
            .accounts
            .list()
            .await?
            .into_iter()
            .filter(|a| a.kind == AccountKind::Yggdrasil && a.valid)
            .map(|a| a.id)
            .collect();
        for id in own {
            if own_account(self.accounts.as_ref(), id)
                .await
                .unwrap_or(false)
            {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// [`ChatService::connect`] without taking the lifecycle lock.
    async fn connect_locked(&self, account: AccountId) -> AppResult<SocialUser> {
        self.shutdown_current().await;
        *self.me.write().await = None;
        let session = self
            .start_session(Credential::Account(account), &account.to_string())
            .await?;
        let response = self.request(&session, Method::GET, &["me"], None).await?;
        let user: SocialUser = decode(response, &session.cancel).await?;
        self.spawn_socket(session).await;
        *self.me.write().await = Some(user.clone());
        Ok(user)
    }
}

#[async_trait]
impl ChatService for ChatClient {
    async fn connect(&self, account: AccountId) -> AppResult<SocialUser> {
        let _guard = self.lifecycle.lock().await;
        self.shutdown_current().await;
        if !own_account(self.accounts.as_ref(), account).await? {
            return Err(invalid("needsOwnServerAccount"));
        }
        self.connect_locked(account).await
    }
    async fn disconnect(&self) -> AppResult<()> {
        let _guard = self.lifecycle.lock().await;
        self.shutdown_current().await;
        // The online counter tracks launchers, not chat sessions: it keeps
        // running while the app is open.
        self.observer
            .on_event(SocialEvent::Connection { connected: false });
        Ok(())
    }
    async fn server_account(&self) -> AppResult<Option<SocialUser>> {
        let _guard = self.lifecycle.lock().await;
        let Some(id) = self.stored_guest_id().await? else {
            return Ok(None);
        };
        let current = self.me.read().await.clone();
        if let Some(user) = current.filter(|u| !u.guest) {
            return Ok(Some(user));
        }
        self.connect_locked(id).await.map(Some)
    }
    async fn set_presence(&self, status: SocialStatus) -> AppResult<()> {
        let s = self.current_session().await?;
        s.presence.send_replace(status);
        Ok(())
    }
    async fn channels(&self) -> AppResult<Vec<SocialChannel>> {
        self.api(Method::GET, &["channels"], None).await
    }
    async fn history(
        &self,
        channel: &SocialChannelId,
        before: Option<String>,
    ) -> AppResult<SocialHistory> {
        let s = self.current_session().await?;
        let mut url = endpoint(&s.base, &["channels", channel, "messages"])?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", "100");
            if let Some(before) = &before {
                query.append_pair("before", before);
            }
        }
        if s.cancel.is_cancelled() {
            return Err(cancelled());
        }
        let token = self.token(&s).await?;
        let key = format!(
            "{:x}.json",
            Sha256::digest(
                serde_json::to_vec(&(channel, &before)).map_err(|_| invalid("channel"))?
            )
        );
        let response = tokio::select! { _ = s.cancel.cancelled() => return Err(cancelled()), r = self.downloader.client(Route::Auto).get(url).bearer_auth(token.expose()).send() => r };
        let response = match response {
            Ok(r) => r,
            Err(_) => {
                if s.cancel.is_cancelled() {
                    return Err(cancelled());
                }
                let bytes = tokio::fs::read(s.cache.join(&key))
                    .await
                    .map_err(|_| network())?;
                if bytes.len() > MAX_JSON {
                    return Err(network());
                }
                return serde_json::from_slice(&bytes).map_err(|_| network());
            }
        };
        let page: SocialHistory = decode(checked(response, &s.cancel).await?, &s.cancel).await?;

        // Serialize cache mutations with logout so a finished request cannot recreate it.
        let _guard = self.lifecycle.lock().await;
        if s.cancel.is_cancelled() {
            return Err(cancelled());
        }
        tokio::fs::create_dir_all(&s.cache).await?;
        let bytes = serde_json::to_vec(&page)
            .map_err(|_| AppError::internal("History serialization failed"))?;
        let tmp = s.cache.join(format!("{key}.part"));
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(tmp, s.cache.join(key)).await?;
        Ok(page)
    }
    async fn mark_read(&self, channel: &SocialChannelId, message_id: &str) -> AppResult<()> {
        self.empty(
            Method::POST,
            &["channels", channel, "read"],
            Some(json!({"messageId":message_id})),
        )
        .await
    }
    async fn send(
        &self,
        channel: &SocialChannelId,
        content: SocialContent,
    ) -> AppResult<SocialMessage> {
        self.full_session().await?;
        if matches!(content, SocialContent::PackShare { .. }) && channel == "global" {
            return Err(invalid("directOnly"));
        }
        self.api(
            Method::POST,
            &["channels", channel, "messages"],
            Some(value(content)?),
        )
        .await
    }
    async fn send_file(
        &self,
        channel: &SocialChannelId,
        path: &str,
        caption: Option<String>,
        sink: &dyn ProgressSink,
    ) -> AppResult<SocialMessage> {
        sink.check_cancelled()?;
        let s = self.full_session().await?;
        let mut file = tokio::fs::File::open(path).await?;
        let size = file.metadata().await?.len();
        if size > MAX_FILE as u64 {
            return Err(invalid("fileTooLarge"));
        }
        let mut bytes = Vec::with_capacity(size as usize);
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = tokio::select! { _ = sink.cancel_token().cancelled() => return Err(cancelled()), _ = s.cancel.cancelled() => return Err(cancelled()), n = file.read(&mut buf) => n? };
            if n == 0 {
                break;
            }
            if bytes.len() + n > MAX_FILE {
                return Err(invalid("fileTooLarge"));
            }
            bytes.extend_from_slice(&buf[..n]);
            sink.progress(bytes.len() as u64, Some(size), "chat.readFile");
        }
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| invalid("fileName"))?
            .to_owned();
        let part = reqwest::multipart::Part::bytes(bytes).file_name(name);
        if s.cancel.is_cancelled() {
            return Err(cancelled());
        }
        let token = self.token(&s).await?;
        sink.check_cancelled()?;
        sink.progress(0, Some(size), "chat.upload");
        let request = self
            .downloader
            .client(Route::Auto)
            .post(endpoint(&s.base, &["files"])?)
            .bearer_auth(token.expose())
            .multipart(reqwest::multipart::Form::new().part("file", part));
        let response = tokio::select! { _ = sink.cancel_token().cancelled() => return Err(cancelled()), _ = s.cancel.cancelled() => return Err(cancelled()), r = request.send() => r.map_err(|_| network())? };
        let attachment: SocialAttachment = tokio::select! { _ = sink.cancel_token().cancelled() => return Err(cancelled()), r = async { decode(checked(response, &s.cancel).await?, &s.cancel).await } => r? };
        sink.check_cancelled()?;
        if s.cancel.is_cancelled() {
            return Err(cancelled());
        }
        sink.progress(size, Some(size), "chat.upload");
        // Keep the original session for upload + send; never post into a switched account.
        tokio::select! {
            _ = sink.cancel_token().cancelled() => Err(cancelled()),
            result = async {
                let response = self.request(&s, Method::POST, &["channels", channel, "messages"], Some(value(SocialContent::Attachment { attachment, caption })?)).await?;
                decode(response, &s.cancel).await
            } => result
        }
    }
    async fn delete_message(&self, message_id: &str) -> AppResult<()> {
        self.empty(Method::DELETE, &["messages", message_id], None)
            .await
    }
    async fn save_attachment(
        &self,
        attachment_id: &str,
        dest: &str,
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        sink.check_cancelled()?;
        let s = self.current_session().await?;
        let segments = ["files", attachment_id];
        let response = tokio::select! { _ = sink.cancel_token().cancelled() => return Err(cancelled()), r = self.request(&s, Method::GET, &segments, None) => r? };
        let total = response.content_length();
        if total.is_some_and(|n| n > MAX_FILE as u64) {
            return Err(invalid("fileTooLarge"));
        }
        let destination = PathBuf::from(dest);
        let parent = destination.parent().ok_or_else(|| invalid("destination"))?;
        let temporary = parent.join(format!(".chat-{}.part", sink.task_id()));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await?;
        let result: AppResult<()> = async {
            let mut response = response;
            let mut done = 0u64;
            loop {
                let chunk = tokio::select! { _ = sink.cancel_token().cancelled() => return Err(cancelled()), _ = s.cancel.cancelled() => return Err(cancelled()), r = response.chunk() => r.map_err(|_| network())? };
                let Some(chunk) = chunk else { break; };
                done += chunk.len() as u64;
                if done > MAX_FILE as u64 { return Err(invalid("fileTooLarge")); }
                file.write_all(&chunk).await?; sink.progress(done, total, "chat.download");
            }
            sink.check_cancelled()?; if s.cancel.is_cancelled() { return Err(cancelled()); }
            file.sync_all().await?; drop(file);
            tokio::fs::rename(&temporary, destination).await?; Ok(())
        }.await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(temporary).await;
        }
        result
    }
    async fn attachment_preview(&self, attachment_id: &str) -> AppResult<String> {
        let s = self.current_session().await?;
        let mut url = endpoint(&s.base, &["files", attachment_id])?;
        url.query_pairs_mut().append_pair("thumb", "1");
        let token = self.token(&s).await?;
        let response = tokio::select! {
            _ = s.cancel.cancelled() => return Err(cancelled()),
            r = self.downloader.client(Route::Auto).get(url).bearer_auth(token.expose()).send() => r.map_err(|_| network())?
        };
        let response = checked(response, &s.cancel).await?;
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if mime.split(';').next().map(str::trim) != Some("image/webp") {
            return Err(invalid("thumbnailType"));
        }
        let bytes = bounded(response, 256 * 1024, &s.cancel).await?;
        thumbnail_data_url(&bytes)
    }
    async fn open_direct(&self, uuid: &str) -> AppResult<SocialChannel> {
        self.full_session().await?;
        self.api(Method::POST, &["users", uuid, "direct"], None)
            .await
    }
    async fn search_users(&self, query: &str) -> AppResult<Vec<SocialUser>> {
        if query.trim().chars().count() < 2 {
            return Err(invalid("queryTooShort"));
        }
        let s = self.current_session().await?;
        let mut url = endpoint(&s.base, &["users"])?;
        url.query_pairs_mut().append_pair("q", query);
        if s.cancel.is_cancelled() {
            return Err(cancelled());
        }
        let token = self.token(&s).await?;
        let response = tokio::select! { _ = s.cancel.cancelled() => return Err(cancelled()), r = self.downloader.client(Route::Auto).get(url).bearer_auth(token.expose()).send() => r.map_err(|_| network())? };
        decode(checked(response, &s.cancel).await?, &s.cancel).await
    }
    async fn profile(&self, uuid: &str) -> AppResult<SocialUser> {
        self.api(Method::GET, &["users", uuid], None).await
    }
    async fn friends(&self) -> AppResult<Vec<SocialFriend>> {
        self.full_session().await?;
        self.api(Method::GET, &["friends"], None).await
    }
    async fn add_friend(&self, uuid: &str) -> AppResult<SocialFriend> {
        self.full_session().await?;
        self.api(Method::PUT, &["friends", uuid], None).await
    }
    async fn remove_friend(&self, uuid: &str) -> AppResult<()> {
        self.full_session().await?;
        self.empty(Method::DELETE, &["friends", uuid], None).await
    }
    async fn set_blocked(&self, uuid: &str, blocked: bool) -> AppResult<()> {
        self.full_session().await?;
        self.empty(
            if blocked { Method::PUT } else { Method::DELETE },
            &["users", uuid, "block"],
            None,
        )
        .await
    }
    async fn privacy(&self) -> AppResult<SocialPrivacy> {
        self.full_session().await?;
        self.api(Method::GET, &["privacy"], None).await
    }
    async fn set_privacy(&self, privacy: SocialPrivacy) -> AppResult<SocialPrivacy> {
        self.full_session().await?;
        self.api(Method::PUT, &["privacy"], Some(value(privacy)?))
            .await
    }
    async fn report(&self, message_id: &str, reason: Option<String>) -> AppResult<()> {
        self.empty(
            Method::POST,
            &["messages", message_id, "report"],
            Some(json!({"reason":reason})),
        )
        .await
    }
    async fn reports(&self) -> AppResult<Vec<SocialReport>> {
        self.api(Method::GET, &["mod", "reports"], None).await
    }
    async fn resolve_report(&self, report_id: &str) -> AppResult<()> {
        self.empty(Method::DELETE, &["mod", "reports", report_id], None)
            .await
    }
    async fn moderate_delete(&self, message_id: &str) -> AppResult<()> {
        self.delete_message(message_id).await
    }
    async fn moderate_user(
        &self,
        uuid: &str,
        action: ModAction,
        seconds: Option<u64>,
    ) -> AppResult<()> {
        self.empty(
            Method::POST,
            &["mod", "users", uuid],
            Some(json!({"action":action,"seconds":seconds})),
        )
        .await
    }
    async fn set_role(&self, uuid: &str, role: SocialRole) -> AppResult<()> {
        if role == SocialRole::Admin {
            return Err(invalid("role"));
        }
        self.empty(
            Method::PUT,
            &["admin", "roles", uuid],
            Some(json!({"role":role})),
        )
        .await
    }
    async fn server_state(&self) -> AppResult<AdminServerState> {
        self.api(Method::GET, &["admin", "server"], None).await
    }
    async fn migrate_storage(&self, target: StorageTarget) -> AppResult<()> {
        self.empty(
            Method::POST,
            &["admin", "migrate"],
            Some(json!({"target":target})),
        )
        .await
    }
    async fn guest_login(&self, name: &str) -> AppResult<SocialUser> {
        if !valid_guest_name(name) {
            return Err(invalid("guestName"));
        }
        let _guard = self.lifecycle.lock().await;
        // A signed-in account of the own server is the chat identity: signing in
        // as a guest connects that account instead. Only when the account cannot
        // sign in (expired session, hub trouble) the guest session is used.
        if let Some(id) = self.stored_guest_id().await? {
            if let Ok(user) = self.connect_locked(id).await {
                return Ok(user);
            }
        }
        self.shutdown_current().await;
        *self.me.write().await = None;
        let base = hub_base(self.settings.as_ref()).await?;
        let cancel = CancelToken::new();
        let request = self
            .downloader
            .client(Route::Auto)
            .post(endpoint(&base, &["guest"])?)
            .json(&json!({ "name": name }))
            .send();
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(cancelled()),
            r = request => r.map_err(|_| network())?,
        };
        let login: GuestLogin = decode(checked(response, &cancel).await?, &cancel).await?;
        let token = Secret::new(login.token);
        self.store_guest(&base, &token, &login.user).await;
        let session = self
            .start_session(
                Credential::Guest(token),
                &format!("guest:{}", login.user.uuid),
            )
            .await?;
        self.spawn_socket(session).await;
        *self.me.write().await = Some(login.user.clone());
        Ok(login.user)
    }
    async fn update_profile(&self, patch: ProfileUpdate) -> AppResult<SocialUser> {
        validate_profile(&patch)?;
        let s = self.full_session().await?;
        let response = self
            .request(&s, Method::PATCH, &["profile"], Some(value(patch)?))
            .await?;
        decode(response, &s.cancel).await
    }
    async fn set_avatar(&self, path: &str, sink: &dyn ProgressSink) -> AppResult<SocialUser> {
        sink.check_cancelled()?;
        let s = self.full_session().await?;
        let mut file = tokio::fs::File::open(path).await?;
        let size = file.metadata().await?.len();
        if size == 0 {
            return Err(invalid("avatarEmpty"));
        }
        if size > MAX_AVATAR {
            return Err(invalid("avatarTooLarge"));
        }
        let mut bytes = Vec::with_capacity(size as usize);
        let mut buf = [0u8; 16 * 1024];
        loop {
            let n = tokio::select! { _ = sink.cancel_token().cancelled() => return Err(cancelled()), _ = s.cancel.cancelled() => return Err(cancelled()), n = file.read(&mut buf) => n? };
            if n == 0 {
                break;
            }
            if bytes.len() + n > MAX_AVATAR as usize {
                return Err(invalid("avatarTooLarge"));
            }
            bytes.extend_from_slice(&buf[..n]);
            sink.progress(bytes.len() as u64, Some(size), "chat.readFile");
        }
        drop(file);
        if bytes.is_empty() {
            return Err(invalid("avatarEmpty"));
        }
        // Raw image body: the hub's multipart parser rejects file names with
        // non-ASCII characters, and the hub re-encodes the image anyway, so the
        // name and the exact mime type do not matter.
        let mime = match std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            Some("bmp") => "image/bmp",
            _ => "image/png",
        };
        let token = self.token(&s).await?;
        sink.check_cancelled()?;
        sink.progress(0, Some(size), "chat.upload");
        let request = self
            .downloader
            .client(Route::Auto)
            .put(endpoint(&s.base, &["avatar"])?)
            .bearer_auth(token.expose())
            .header(reqwest::header::CONTENT_TYPE, mime)
            .body(bytes);
        let response = tokio::select! { _ = sink.cancel_token().cancelled() => return Err(cancelled()), _ = s.cancel.cancelled() => return Err(cancelled()), r = request.send() => r.map_err(|_| network())? };
        let user: SocialUser = tokio::select! { _ = sink.cancel_token().cancelled() => return Err(cancelled()), r = async { decode(checked(response, &s.cancel).await?, &s.cancel).await } => r? };
        sink.check_cancelled()?;
        sink.progress(size, Some(size), "chat.upload");
        // The stored preview is stale now; the next call fetches the new one.
        self.forget_avatar(&s.base, &user.uuid);
        Ok(user)
    }
    async fn clear_avatar(&self) -> AppResult<SocialUser> {
        let s = self.full_session().await?;
        let response = self.request(&s, Method::DELETE, &["avatar"], None).await?;
        let user: SocialUser = decode(response, &s.cancel).await?;
        self.forget_avatar(&s.base, &user.uuid);
        Ok(user)
    }
    async fn presence(&self) -> AppResult<PresenceSummary> {
        // Public endpoint: no session or account is required.
        let base = hub_base(self.settings.as_ref()).await?;
        let cancel = CancelToken::new();
        let request = self
            .downloader
            .client(Route::Auto)
            .get(root_endpoint(&base, &["presence"])?)
            .send();
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(cancelled()),
            r = request => r.map_err(|_| network())?,
        };
        decode(checked(response, &cancel).await?, &cancel).await
    }
    async fn request_account(&self, login: &str, password: &str, note: &str) -> AppResult<()> {
        // Guests only: the guest session must exist; the hub answers 204, so
        // nothing is decoded here.
        self.current_session().await?;
        self.empty(
            Method::POST,
            &["account-request"],
            Some(json!({ "username": login, "password": password, "note": note })),
        )
        .await
    }
    async fn create_account(&self, request_id: &str) -> AppResult<()> {
        self.empty(
            Method::POST,
            &["admin", "create-account", request_id],
            Some(json!({})),
        )
        .await
    }
    async fn admin_accounts(&self) -> AppResult<Vec<AdminAccount>> {
        self.api(Method::GET, &["admin", "accounts"], None).await
    }
    async fn account_requests(&self) -> AppResult<Vec<AccountRequest>> {
        self.api(Method::GET, &["admin", "requests"], None).await
    }
    async fn issue_invite(&self, request_id: &str) -> AppResult<String> {
        #[derive(serde::Deserialize)]
        struct Issued {
            url: String,
        }
        let issued: Issued = self
            .api(Method::POST, &["admin", "invite", request_id], None)
            .await?;
        Ok(issued.url)
    }
    async fn admin_presence(&self) -> AppResult<AdminPresence> {
        self.api(Method::GET, &["admin", "presence"], None).await
    }
    async fn audit(&self) -> AppResult<Vec<AuditEntry>> {
        self.api(Method::GET, &["admin", "audit"], None).await
    }
    /// Current session from memory; a stored guest token is restored without a
    /// hub round trip, so a guest stays signed in across launcher restarts.
    async fn session(&self) -> AppResult<Option<SocialUser>> {
        let _guard = self.lifecycle.lock().await;
        if let Some(user) = self.me.read().await.clone() {
            if self.live.read().await.is_some() {
                return Ok(Some(user));
            }
        }
        let base = hub_base(self.settings.as_ref()).await?;
        let Some((token, user)) = self.load_guest(&base).await else {
            return Ok(None);
        };
        let session = self
            .start_session(Credential::Guest(token), &format!("guest:{}", user.uuid))
            .await?;
        self.spawn_socket(session).await;
        *self.me.write().await = Some(user.clone());
        Ok(Some(user))
    }
    /// Avatar bytes of a player as a `data:` URL (WebP, bounded), or an error
    /// when the player has no avatar.
    ///
    /// Previews live in a bounded in-memory cache and are never written to disk
    /// or to the keyring; the cache is dropped when the session changes.
    async fn avatar_preview(&self, uuid: &str) -> AppResult<String> {
        let uuid = uuid.trim();
        if uuid.is_empty() || uuid.len() > 128 || uuid.chars().any(char::is_control) {
            return Err(invalid("uuid"));
        }
        let s = self.current_session().await?;
        let key = avatar_key(&s.base, uuid);
        if let Some(cached) = self.cached_avatar(&key) {
            return Ok(cached);
        }
        let url = endpoint(&s.base, &["avatar", uuid])?;
        let data =
            avatar_data_url(&s, self.token(&s).await?, url, self.downloader.as_ref()).await?;
        self.remember_avatar(key, data.clone());
        Ok(data)
    }
    /// Original image as a `data:` URL for the full-screen viewer; only image
    /// types are returned and the size is bounded in Rust.
    async fn attachment_original(&self, id: &str) -> AppResult<String> {
        let s = self.current_session().await?;
        let url = endpoint(&s.base, &["files", id])?;
        image_data_url(&s, self.token(&s).await?, url, self.downloader.as_ref()).await
    }
}

/// Response of `POST /chat/guest`; the token stays in memory for this session.
#[derive(Deserialize)]
struct GuestLogin {
    token: String,
    user: SocialUser,
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
