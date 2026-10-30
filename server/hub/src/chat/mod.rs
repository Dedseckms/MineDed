//! Authenticated social chat API, backed by SQLite WAL and content-addressed files.
mod admin;
mod drasl;
mod files;
mod filter;
mod store;
#[cfg(test)]
mod tests;

use crate::{error::ApiError, ratelimit::RateLimiter, AppState, ClientIp};
use axum::body::to_bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{DefaultBodyLimit, Extension, Request, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use lc_types::social::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, OwnedSemaphorePermit, Semaphore};

/// Durable state. Open once during startup; an unavailable database fails startup.
pub struct Chat {
    store: Mutex<store::Store>,
    events: broadcast::Sender<Notice>,
    presence: Mutex<HashMap<String, (usize, SocialStatus)>>,
    requests: RateLimiter<IpAddr>,
    users: RateLimiter<String>,
    uploads: RateLimiter<String>,
    sockets: Arc<Semaphore>,
    jobs: Arc<Semaphore>,
    upload_jobs: Arc<Semaphore>,
    directory: drasl::Directory,
    root: PathBuf,
    migration: Mutex<Option<f32>>,
    /// Last full refresh of the public player directory.
    users_synced: Mutex<Option<Instant>>,
    disk_warning: std::sync::atomic::AtomicU64,
    last_health: Mutex<Option<AdminServerState>>,
}
#[derive(Clone)]
enum Notice {
    Message(i64, bool),
    Friend(String, String),
    Presence(String),
    Admin,
}
#[derive(Clone)]
struct Caller(String);

impl Chat {
    /// Opens a durable directory, creating schema v1 if needed. No data is deleted.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ApiError> {
        let root = root.as_ref().to_path_buf();
        let store = store::Store::open(&root)?;
        let (events, _) = broadcast::channel(256);
        Ok(Self {
            store: Mutex::new(store),
            events,
            presence: Mutex::new(HashMap::new()),
            requests: RateLimiter::per_minute(180, 60),
            users: RateLimiter::per_minute(120, 40),
            uploads: RateLimiter::per_minute(6, 2),
            sockets: Arc::new(Semaphore::new(128)),
            jobs: Arc::new(Semaphore::new(8)),
            upload_jobs: Arc::new(Semaphore::new(2)),
            directory: drasl::Directory::from_env(),
            root,
            migration: Mutex::new(None),
            users_synced: Mutex::new(None),
            disk_warning: std::sync::atomic::AtomicU64::new(0),
            last_health: Mutex::new(None),
        })
    }
    /// Claims the directory refresh: at most once a minute and never twice at
    /// once. A full catalogue upsert per request would queue the store behind
    /// thousands of writes on a small host.
    fn claim_directory_sync(&self) -> bool {
        const TTL: Duration = Duration::from_secs(60);
        match self.users_synced.lock() {
            Ok(mut last) => {
                if last.is_some_and(|at| at.elapsed() < TTL) {
                    return false;
                }
                *last = Some(Instant::now());
                true
            }
            // A poisoned lock must not switch the catalogue off for good.
            Err(_) => true,
        }
    }
    /// Keeps the claim after a filled refresh, otherwise retries on the next request.
    fn finish_directory_sync(&self, filled: bool) {
        if filled {
            return;
        }
        if let Ok(mut last) = self.users_synced.lock() {
            *last = None;
        }
    }
    async fn run<T: Send + 'static>(
        self: &Arc<Self>,
        f: impl FnOnce(&mut store::Store, &Chat) -> Result<T, ApiError> + Send + 'static,
    ) -> Result<T, ApiError> {
        let permit = self
            .jobs
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::rate_limited(1))?;
        self.run_admitted(permit, f).await
    }
    /// Socket count bounds waiters; internal notices wait instead of rejecting live subscribers.
    async fn run_wait<T: Send + 'static>(
        self: &Arc<Self>,
        f: impl FnOnce(&mut store::Store, &Chat) -> Result<T, ApiError> + Send + 'static,
    ) -> Result<T, ApiError> {
        let permit = self
            .jobs
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| internal())?;
        self.run_admitted(permit, f).await
    }
    async fn run_admitted<T: Send + 'static>(
        self: &Arc<Self>,
        permit: OwnedSemaphorePermit,
        f: impl FnOnce(&mut store::Store, &Chat) -> Result<T, ApiError> + Send + 'static,
    ) -> Result<T, ApiError> {
        let chat = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut store = chat.store.lock().map_err(|_| internal())?;
            f(&mut store, &chat)
        })
        .await
        .map_err(|_| internal())?
    }
    fn sweep(&self) {
        self.requests.sweep();
        self.users.sweep();
        self.uploads.sweep();
    }
    fn disconnect_presence(&self, me: &str) {
        if let Ok(mut presence) = self.presence.lock() {
            if let Some(entry) = presence.get_mut(me) {
                entry.0 = entry.0.saturating_sub(1);
                if entry.0 == 0 {
                    presence.remove(me);
                }
            }
        }
        let _ = self.events.send(Notice::Presence(me.into()));
    }
    pub(crate) async fn poll_health(self: &Arc<Self>) {
        if let Ok(state) = self.run(|db, chat| db.health(chat)).await {
            if let Ok(mut old) = self.last_health.lock() {
                let changed = old.as_ref().is_none_or(|v| {
                    v.attachments_paused != state.attachments_paused
                        || v.last_backup != state.last_backup
                        || v.backup_error != state.backup_error
                        || v.disk_total_bytes != state.disk_total_bytes
                });
                *old = Some(state);
                if changed {
                    let _ = self.events.send(Notice::Admin);
                }
            }
        }
    }
}
fn internal() -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        "chat storage unavailable",
    )
}
fn invalid(message: &str) -> ApiError {
    ApiError::invalid_input(message)
}
fn denied() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "permissionDenied",
        "chat permission denied",
    )
}
fn rejected(reason: &str) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalidInput",
        "message violates chat rules",
    )
    .detail("reason", reason)
}
impl From<rusqlite::Error> for ApiError {
    fn from(_: rusqlite::Error) -> Self {
        internal()
    }
}
impl From<std::io::Error> for ApiError {
    fn from(_: std::io::Error) -> Self {
        internal()
    }
}
impl From<serde_json::Error> for ApiError {
    fn from(_: serde_json::Error) -> Self {
        internal()
    }
}

pub(super) fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/chat/ws", get(socket))
        .route(
            "/chat/files",
            post(files::upload).layer(DefaultBodyLimit::max(files::MAX_UPLOAD + 64 * 1024)),
        )
        .route("/chat/files/{id}", get(files::download))
        .route("/chat/{*path}", any(dispatch))
        .route_layer(middleware::from_fn_with_state(state, authenticate))
}
async fn authenticate(State(s): State<AppState>, mut req: Request, next: Next) -> Response {
    let result = async {
        let chat = s.chat.clone().ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "network",
                "chat is not configured",
            )
        })?;
        let (mut parts, body) = req.into_parts();
        let ClientIp(ip) =
            <ClientIp as axum::extract::FromRequestParts<AppState>>::from_request_parts(
                &mut parts, &s,
            )
            .await
            .map_err(|_| internal())?;
        req = Request::from_parts(parts, body);
        chat.requests.check(&ip).map_err(ApiError::rate_limited)?;
        let who = crate::require_identity(&s, req.headers()).await?;
        chat.users
            .check(&who.user_id)
            .map_err(ApiError::rate_limited)?;
        let info = chat.directory.profile(&who.user_id).await;
        let id = chat
            .run(move |db, _| {
                db.identify(
                    &who,
                    info.as_ref().is_some_and(|v| v.admin),
                    info.as_ref().and_then(|v| v.skin.as_deref()),
                )
            })
            .await?;
        req.extensions_mut().insert(Caller(id));
        chat.sweep();
        Ok::<_, ApiError>(next.run(req).await)
    }
    .await;
    result.unwrap_or_else(IntoResponse::into_response)
}
async fn dispatch(
    State(s): State<AppState>,
    Extension(Caller(me)): Extension<Caller>,
    req: Request,
) -> Result<Response, ApiError> {
    let chat = s.chat.ok_or_else(internal)?;
    let method = req.method().as_str().to_owned();
    let path = req.uri().path().trim_start_matches("/chat/").to_owned();
    let q: serde_json::Map<String, Value> =
        form_urlencoded::parse(req.uri().query().unwrap_or("").as_bytes())
            .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
            .collect();
    let bytes = to_bytes(req.into_body(), 32 * 1024)
        .await
        .map_err(|_| invalid("request body too large"))?;
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).map_err(|_| invalid("invalid JSON body"))?
    };
    if method == "GET" && path == "users" && chat.claim_directory_sync() {
        let players = chat.directory.players().await;
        let filled = !players.is_empty();
        let synced = chat.run(move|db,_|{for player in players {db.db.execute("INSERT INTO users(uuid,name,skin,privacy) VALUES(?1,?2,?3,?4) ON CONFLICT(uuid) DO UPDATE SET name=excluded.name,skin=excluded.skin",rusqlite::params![player.uuid,player.name,player.skin,serde_json::to_string(&json!({"whoCanMessage":"everyone","whoCanInvite":"everyone","whoSeesStatus":"everyone","whoCanAddFriend":"everyone"}))?])?;}Ok(())}).await;
        chat.finish_directory_sync(synced.is_ok() && filled);
        synced?;
    }
    if method == "POST" && path == "admin/migrate" {
        chat.start_migration(me, body).await?;
        return Ok(StatusCode::ACCEPTED.into_response());
    }
    let result = chat
        .run(move |db, chat| {
            db.handle(
                &me,
                &method,
                &path.split('/').collect::<Vec<_>>(),
                &Value::Object(q),
                body,
                chat,
            )
        })
        .await?;
    if result.is_null() {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(Json(result).into_response())
    }
}
async fn socket(
    State(s): State<AppState>,
    Extension(Caller(me)): Extension<Caller>,
    req: Request,
) -> Result<Response, ApiError> {
    let chat = s.chat.clone().ok_or_else(internal)?;
    let token = crate::bearer(req.headers())
        .ok_or_else(ApiError::auth_required)?
        .to_owned();
    let (mut parts, _) = req.into_parts();
    let upgrade =
        <WebSocketUpgrade as axum::extract::FromRequestParts<AppState>>::from_request_parts(
            &mut parts, &s,
        )
        .await
        .map_err(|_| invalid("WebSocket upgrade required"))?;
    let permit = chat
        .sockets
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::rate_limited(5))?;
    {
        let mut presence = chat.presence.lock().map_err(|_| internal())?;
        let entry = presence
            .entry(me.clone())
            .or_insert((0, SocialStatus::Offline));
        if entry.0 >= 3 {
            return Err(ApiError::rate_limited(5));
        }
        entry.0 += 1;
        entry.1 = SocialStatus::Online;
    }
    let failed_chat = chat.clone();
    let failed_me = me.clone();
    Ok(upgrade
        .max_message_size(2048)
        .max_frame_size(2048)
        .on_failed_upgrade(move |_| failed_chat.disconnect_presence(&failed_me))
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            let _ = chat.events.send(Notice::Presence(me.clone()));
            socket_loop(socket, s, chat.clone(), me.clone(), token).await;
            chat.disconnect_presence(&me);
        }))
}
async fn send(socket: &mut WebSocket, event: SocialEvent) -> Result<(), ()> {
    let text = serde_json::to_string(&event).map_err(|_| ())?;
    send_frame(socket, Message::Text(text.into())).await
}
async fn send_frame(socket: &mut WebSocket, frame: Message) -> Result<(), ()> {
    tokio::time::timeout(Duration::from_secs(10), socket.send(frame))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}
async fn socket_loop(
    mut socket: WebSocket,
    s: AppState,
    chat: Arc<Chat>,
    me: String,
    token: String,
) {
    let mut events = chat.events.subscribe();
    let mut auth = tokio::time::interval(Duration::from_secs(60));
    let mut last_seen = tokio::time::Instant::now();
    auth.tick().await;
    if send(&mut socket, SocialEvent::Connection { connected: true })
        .await
        .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            _=auth.tick()=>{
                if last_seen.elapsed()>Duration::from_secs(120){break;}
                let who=match s.validator.validate(&token).await{Ok(Some(who)) if store::uuid(&who.user_id).ok().as_deref()==Some(&me)=>who,_=>break};
                let info=chat.directory.profile(&who.user_id).await;
                if chat.run_wait(move|db,_|db.identify(&who,info.as_ref().is_some_and(|v|v.admin),info.as_ref().and_then(|v|v.skin.as_deref()))).await.is_err(){break;}
                if send_frame(&mut socket,Message::Ping(Vec::new().into())).await.is_err(){break;}
            }
            incoming=socket.recv()=>match incoming{
                Some(Ok(Message::Text(text)))=>{
                    last_seen=tokio::time::Instant::now();
                    if chat.users.check(&me).is_err(){break;}
                    let Ok(v)=serde_json::from_str::<Value>(&text)else{break;};
                    if v["type"]!="presence"{continue;}
                    let Ok(status)=serde_json::from_value::<SocialStatus>(v["status"].clone())else{break;};
                    if let Ok(mut presence)=chat.presence.lock(){if let Some(entry)=presence.get_mut(&me){entry.1=status;}}
                    let _=chat.events.send(Notice::Presence(me.clone()));
                }
                Some(Ok(Message::Ping(bytes)))=>{last_seen=tokio::time::Instant::now();if send_frame(&mut socket,Message::Pong(bytes)).await.is_err(){break;}}
                Some(Ok(Message::Pong(_)))=>{last_seen=tokio::time::Instant::now();},
                _=>break,
            },
            event=events.recv()=>{
                let Ok(event)=event else{break;};
                let viewer=me.clone();
                let event=chat.run_wait(move|db,chat|{
                    let e=match event{
                        Notice::Message(id,updated)=>match db.message(&viewer,id,chat){Ok(message)=>Some(if updated{SocialEvent::MessageUpdated{message}}else{SocialEvent::Message{message}}),Err(e)if e.status==StatusCode::FORBIDDEN=>None,Err(e)=>return Err(e)},
                        Notice::Presence(id)=>{
                            let user=db.user(&viewer,&id,chat)?;
                            user.status.map(|status|SocialEvent::Presence{uuid:id,status})
                        }
                        Notice::Friend(a,b) if a==viewer||b==viewer=>{let peer=if a==viewer{b}else{a};Some(match db.friend(&viewer,&peer,chat){Ok(friend)=>SocialEvent::Friend{friend},Err(_)=>SocialEvent::FriendRemoved{uuid:peer}})},
                        Notice::Admin if db.role(&viewer)?==SocialRole::Admin=>Some(SocialEvent::Admin{state:db.health(chat)?}),
                        _=>None,
                    };Ok(e)
                }).await;
                match event{Ok(Some(e))=>if send(&mut socket,e).await.is_err(){break;},Ok(None)=>{},Err(_)=>break}
            }
        }
    }
}
