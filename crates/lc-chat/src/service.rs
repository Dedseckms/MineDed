use std::{path::PathBuf, sync::Arc, time::Duration};

use crate::{ChatService, SocialObserver};
use async_trait::async_trait;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use lc_accounts::{AccountService, AuthProvider};
use lc_http::{Downloader, Route};
use lc_runtime::{CancelToken, Paths, ProgressSink, SettingsStore};
use lc_types::{account::AccountKind, social::*, AccountId, AppError, AppResult, ErrorCode};
use reqwest::{Method, Response};
use serde::{de::DeserializeOwned, Serialize};
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

struct Session {
    account: AccountId,
    base: Url,
    cancel: CancelToken,
    cache: PathBuf,
    presence: watch::Sender<SocialStatus>,
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

/// Authenticated hub client. One account owns each connection and cache namespace.
pub struct ChatClient {
    accounts: Arc<dyn AccountService>,
    auth: Arc<dyn AuthProvider>,
    downloader: Arc<dyn Downloader>,
    paths: Paths,
    settings: Arc<dyn SettingsStore>,
    observer: Arc<dyn SocialObserver>,
    live: RwLock<Option<Live>>,
    lifecycle: Mutex<()>,
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
        Self {
            accounts,
            auth,
            downloader,
            paths,
            settings,
            observer,
            live: RwLock::new(None),
            lifecycle: Mutex::new(()),
        }
    }
    async fn session(&self) -> AppResult<Arc<Session>> {
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
        if !own_account(self.accounts.as_ref(), session.account).await? {
            session.cancel.cancel();
            self.clear_cache(&session).await;
            return Err(expired());
        }
        Ok(session)
    }
    async fn clear_cache(&self, s: &Session) {
        let _ = tokio::fs::remove_dir_all(&s.cache).await;
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
        let token = self.auth.launch_auth(Some(s.account)).await?.access_token;
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
        let s = self.session().await?;
        let r = self.request(&s, method, segments, body).await?;
        decode(r, &s.cancel).await
    }
    async fn empty(&self, method: Method, segments: &[&str], body: Option<Value>) -> AppResult<()> {
        let s = self.session().await?;
        self.request(&s, method, segments, body).await?;
        Ok(())
    }
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
fn endpoint(base: &Url, segments: &[&str]) -> AppResult<Url> {
    let mut url = base.clone();
    {
        let mut path = url.path_segments_mut().map_err(|_| invalid("hubUrl"))?;
        path.pop_if_empty();
        path.push("chat");
        for segment in segments {
            if segment.is_empty() || *segment == "." || *segment == ".." {
                return Err(invalid("pathSegment"));
            }
            path.push(segment);
        }
    }
    Ok(url)
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
    if let Ok(bytes) = bounded(response, 16384, cancel).await {
        if let Ok(server) = serde_json::from_slice::<AppError>(&bytes) {
            if let Some(reason) = server.details.as_ref().and_then(|d| d.get("reason")) {
                if ["filtered", "muted", "attachmentsPaused"].contains(&reason.as_str()) {
                    error = error.detail("reason", reason);
                }
            }
        }
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
    if !own_account(accounts, s.account).await? {
        s.cancel.cancel();
        return Err(expired());
    }
    let token = auth.launch_auth(Some(s.account)).await?.access_token;
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
                if !own_account(accounts, s.account).await? { s.cancel.cancel(); return Err(expired()); }
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

#[async_trait]
impl ChatService for ChatClient {
    async fn connect(&self, account: AccountId) -> AppResult<SocialUser> {
        let _guard = self.lifecycle.lock().await;
        if let Some(old) = self.live.write().await.take() {
            old.session.cancel.cancel();
            self.clear_cache(&old.session).await;
            drop(old);
        }
        if !own_account(self.accounts.as_ref(), account).await? {
            return Err(invalid("needsOwnServerAccount"));
        }
        let settings = self.settings.get().await?;
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
        let cache = self
            .paths
            .cache()
            .join("chat")
            .join(format!(
                "{:x}",
                Sha256::digest(format!("{}:{account}", base).as_bytes())
            ))
            .join(AccountId::new().to_string());
        let session = Arc::new(Session {
            account,
            base,
            cancel: CancelToken::new(),
            cache,
            presence: watch::channel(SocialStatus::Online).0,
        });
        let response = self.request(&session, Method::GET, &["me"], None).await?;
        let user = decode(response, &session.cancel).await?;
        let task = tokio::spawn(socket(
            session.clone(),
            self.accounts.clone(),
            self.auth.clone(),
            self.downloader.clone(),
            self.observer.clone(),
        ));
        *self.live.write().await = Some(Live { session, task });
        Ok(user)
    }
    async fn disconnect(&self) -> AppResult<()> {
        let _guard = self.lifecycle.lock().await;
        if let Some(old) = self.live.write().await.take() {
            old.session.cancel.cancel();
            old.task.abort();
            self.clear_cache(&old.session).await;
        }
        self.observer
            .on_event(SocialEvent::Connection { connected: false });
        Ok(())
    }
    async fn set_presence(&self, status: SocialStatus) -> AppResult<()> {
        let s = self.session().await?;
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
        let s = self.session().await?;
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
        let token = self.auth.launch_auth(Some(s.account)).await?.access_token;
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
        let s = self.session().await?;
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
        let token = self.auth.launch_auth(Some(s.account)).await?.access_token;
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
        let s = self.session().await?;
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
        let s = self.session().await?;
        let mut url = endpoint(&s.base, &["files", attachment_id])?;
        url.query_pairs_mut().append_pair("thumb", "1");
        let token = self.auth.launch_auth(Some(s.account)).await?.access_token;
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
        self.api(Method::POST, &["users", uuid, "direct"], None)
            .await
    }
    async fn search_users(&self, query: &str) -> AppResult<Vec<SocialUser>> {
        if query.trim().chars().count() < 2 {
            return Err(invalid("queryTooShort"));
        }
        let s = self.session().await?;
        let mut url = endpoint(&s.base, &["users"])?;
        url.query_pairs_mut().append_pair("q", query);
        if s.cancel.is_cancelled() {
            return Err(cancelled());
        }
        let token = self.auth.launch_auth(Some(s.account)).await?.access_token;
        let response = tokio::select! { _ = s.cancel.cancelled() => return Err(cancelled()), r = self.downloader.client(Route::Auto).get(url).bearer_auth(token.expose()).send() => r.map_err(|_| network())? };
        decode(checked(response, &s.cancel).await?, &s.cancel).await
    }
    async fn profile(&self, uuid: &str) -> AppResult<SocialUser> {
        self.api(Method::GET, &["users", uuid], None).await
    }
    async fn friends(&self) -> AppResult<Vec<SocialFriend>> {
        self.api(Method::GET, &["friends"], None).await
    }
    async fn add_friend(&self, uuid: &str) -> AppResult<SocialFriend> {
        self.api(Method::PUT, &["friends", uuid], None).await
    }
    async fn remove_friend(&self, uuid: &str) -> AppResult<()> {
        self.empty(Method::DELETE, &["friends", uuid], None).await
    }
    async fn set_blocked(&self, uuid: &str, blocked: bool) -> AppResult<()> {
        self.empty(
            if blocked { Method::PUT } else { Method::DELETE },
            &["users", uuid, "block"],
            None,
        )
        .await
    }
    async fn privacy(&self) -> AppResult<SocialPrivacy> {
        self.api(Method::GET, &["privacy"], None).await
    }
    async fn set_privacy(&self, privacy: SocialPrivacy) -> AppResult<SocialPrivacy> {
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
    // Filled in by net per docs/plans/chat-1.4.md; the hub endpoints are new.
    async fn guest_login(&self, _name: &str) -> AppResult<SocialUser> {
        Err(AppError::not_implemented("ChatService::guest_login"))
    }
    async fn update_profile(&self, _patch: ProfileUpdate) -> AppResult<SocialUser> {
        Err(AppError::not_implemented("ChatService::update_profile"))
    }
    async fn set_avatar(&self, _path: &str, _sink: &dyn ProgressSink) -> AppResult<SocialUser> {
        Err(AppError::not_implemented("ChatService::set_avatar"))
    }
    async fn clear_avatar(&self) -> AppResult<SocialUser> {
        Err(AppError::not_implemented("ChatService::clear_avatar"))
    }
    async fn presence(&self) -> AppResult<PresenceSummary> {
        Ok(PresenceSummary::default())
    }
    async fn admin_presence(&self) -> AppResult<AdminPresence> {
        Err(AppError::not_implemented("ChatService::admin_presence"))
    }
    async fn audit(&self) -> AppResult<Vec<AuditEntry>> {
        Err(AppError::not_implemented("ChatService::audit"))
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
