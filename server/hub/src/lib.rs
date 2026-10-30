//! Hub HTTP API. Routes and shapes follow `server/openapi/hub.yaml`
//! (CONTRACT, lead). Implemented: health, rooms (+WS), VPN access. Mirrors and skins are 501 stubs.

pub mod auth;
pub mod chat;
pub mod curseforge;
pub mod error;
pub mod ratelimit;
pub mod rfc3339;
pub mod rooms;
pub mod vpn;
mod ws;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::{
    ConnectInfo, DefaultBodyLimit, FromRequestParts, Path, Query, State, WebSocketUpgrade,
};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{Identity, TokenValidator};
use crate::error::ApiError;
use crate::ratelimit::RateLimiter;
use crate::rooms::{normalize_code, validate_endpoint_id, AuthMode, CreateRoom, Limits, RoomStore};
use crate::vpn::{VpnConfig, VpnService};

pub const API_PREFIX: &str = "/api/v1";

/// Runtime configuration (from env in `main.rs`).
#[derive(Debug, Clone)]
pub struct Config {
    /// Public iroh-relay URLs returned to clients.
    pub relay_urls: Vec<String>,
    pub limits: Limits,
    /// Take the client IP from `X-Real-IP` (set by the reverse proxy).
    pub trust_proxy: bool,
    /// MineDed VPN backend; `None` = feature off (`/me/vpn` answers 503).
    pub vpn: Option<VpnConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            relay_urls: Vec::new(),
            limits: Limits {
                ttl: Duration::from_secs(6 * 3600),
                max_rooms: 200,
                max_rooms_per_user: 3,
            },
            trust_proxy: false,
            vpn: None,
        }
    }
}

/// Anonymous (`open`, no bearer) rooms per IP at a time (hub.yaml createRoom).
pub const ANON_MAX_ACTIVE_ROOMS: usize = 2;

struct Limiters {
    create: RateLimiter<IpAddr>,
    /// Anonymous creates: 10 per hour per IP.
    anon_create: RateLimiter<IpAddr>,
    join: RateLimiter<IpAddr>,
    ws: RateLimiter<IpAddr>,
    close: RateLimiter<IpAddr>,
    /// `POST /me/vpn` per profile.
    vpn_provision: RateLimiter<String>,
    /// `GET /me/vpn` per profile.
    vpn_get: RateLimiter<String>,
    /// Limits unauthenticated VPN requests before they reach Drasl.
    vpn_request: RateLimiter<IpAddr>,
    /// Launcher heartbeats per IP: several launchers may share one address.
    presence_beat: RateLimiter<IpAddr>,
    /// Guest logins per IP: 5 per hour.
    guest_login: RateLimiter<IpAddr>,
}

#[derive(Clone)]
pub struct AppState {
    cfg: Arc<Config>,
    validator: Arc<dyn TokenValidator>,
    rooms: Arc<RoomStore>,
    limiters: Arc<Limiters>,
    vpn: Option<Arc<VpnService>>,
    ws_connections: Arc<tokio::sync::Semaphore>,
    chat: Option<Arc<chat::Chat>>,
    curseforge: Option<Arc<curseforge::Broker>>,
}

impl AppState {
    /// Enables the restricted public catalogue with a server-only credential.
    pub fn with_curseforge(mut self, broker: curseforge::Broker) -> Self {
        self.curseforge = Some(Arc::new(broker));
        self
    }

    /// Enables durable chat after its database and attachment directory are opened.
    pub fn with_chat(mut self, chat: chat::Chat) -> Self {
        self.chat = Some(Arc::new(chat));
        self
    }

    pub fn new(cfg: Config, validator: Arc<dyn TokenValidator>) -> Self {
        let rooms = Arc::new(RoomStore::new(cfg.limits));
        let vpn = cfg.vpn.clone().map(|v| Arc::new(VpnService::new(v)));
        Self {
            cfg: Arc::new(cfg),
            validator,
            rooms,
            limiters: Arc::new(Limiters {
                create: RateLimiter::per_minute(6, 5),
                anon_create: RateLimiter::per_hour(10, 10),
                join: RateLimiter::per_minute(20, 10),
                ws: RateLimiter::per_minute(60, 20),
                close: RateLimiter::per_minute(30, 10),
                vpn_provision: RateLimiter::per_hour(10, 3),
                vpn_get: RateLimiter::per_minute(30, 10),
                vpn_request: RateLimiter::per_minute(60, 20),
                presence_beat: RateLimiter::per_minute(60, 10),
                guest_login: RateLimiter::per_hour(5, 5),
            }),
            vpn,
            ws_connections: Arc::new(tokio::sync::Semaphore::new(128)),
            chat: None,
            curseforge: None,
        }
    }

    /// Periodic housekeeping: expire rooms, drop idle rate-limit buckets.
    pub fn spawn_reaper(&self) -> tokio::task::JoinHandle<()> {
        let s = self.clone();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(Duration::from_secs(30));
            loop {
                t.tick().await;
                s.rooms.reap();
                s.limiters.create.sweep();
                s.limiters.anon_create.sweep();
                s.limiters.join.sweep();
                s.limiters.ws.sweep();
                s.limiters.close.sweep();
                s.limiters.vpn_provision.sweep();
                s.limiters.vpn_get.sweep();
                s.limiters.vpn_request.sweep();
                s.limiters.presence_beat.sweep();
                s.limiters.guest_login.sweep();
                if let Some(broker) = &s.curseforge {
                    broker.sweep();
                }
                if let Some(chat) = &s.chat {
                    chat.poll_health().await;
                }
            }
        })
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .merge(chat::router(state.clone()))
        .route("/presence", get(presence_summary))
        .route("/presence/beat", post(presence_beat))
        .route(
            "/catalog/curseforge",
            axum::routing::any(curseforge::handle),
        )
        .route(
            "/catalog/curseforge/{*path}",
            axum::routing::any(curseforge::handle),
        )
        .route("/mirrors", get(not_implemented))
        .route("/skins", get(not_implemented).post(not_implemented))
        .route("/skins/{id}", delete(not_implemented))
        .route("/me/skin", post(not_implemented))
        .route("/me/vpn", get(get_vpn).post(provision_vpn))
        .route("/rooms", post(create_room))
        .route("/rooms/{code}/join", post(join_room))
        .route("/rooms/{room_id}", delete(close_room))
        .route("/rooms/{room_id}/ws", get(room_socket))
        .fallback(|| async { ApiError::not_found("no such route") })
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                // Room credentials and personal subscription URLs must not be cached.
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    axum::http::HeaderValue::from_static("no-store"),
                );
                response
            },
        ))
        .with_state(state);
    Router::new().nest(API_PREFIX, api)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn not_implemented() -> ApiError {
    ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "notImplemented",
        "not implemented yet",
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeatRequest {
    client_id: String,
    #[serde(default)]
    kind: String,
}

/// `GET /presence`: how many launchers are on air (any account kind).
async fn presence_summary(
    State(s): State<AppState>,
) -> Result<Json<lc_types::social::PresenceSummary>, ApiError> {
    let chat = s.chat.ok_or_else(chat_unavailable)?;
    Ok(Json(chat.online()))
}

/// `POST /presence/beat`: idempotent heartbeat, one count per client id.
async fn presence_beat(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    body: Result<Bytes, BytesRejection>,
) -> Result<StatusCode, ApiError> {
    let chat = s.chat.ok_or_else(chat_unavailable)?;
    limit(&s.limiters.presence_beat, ip)?;
    let mut req: BeatRequest = parse_json(&request_body(body)?)?;
    req.client_id = req.client_id.trim().to_owned();
    let client_id = Uuid::parse_str(&req.client_id)
        .ok()
        .filter(|id| !id.is_nil())
        .map(|id| id.simple().to_string())
        .ok_or_else(|| ApiError::invalid_input("clientId must be a UUID"))?;
    chat.beat(&client_id, &req.kind)?;
    Ok(StatusCode::NO_CONTENT)
}

fn chat_unavailable() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "network",
        "chat is not configured",
    )
}

/// Client IP: `X-Real-IP` when behind the trusted proxy, else the socket peer.
struct ClientIp(IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, s: &AppState) -> Result<Self, Self::Rejection> {
        if s.cfg.trust_proxy {
            if let Some(ip) = parts
                .headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse().ok())
            {
                return Ok(Self(ip));
            }
        }
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip())
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        Ok(Self(ip))
    }
}

fn limit(rl: &RateLimiter<IpAddr>, ip: IpAddr) -> Result<(), ApiError> {
    rl.check(&ip).map_err(ApiError::rate_limited)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return None;
    }
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer")
        && !token.is_empty()
        && token.len() <= auth::MAX_BEARER_BYTES
        && !token
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control()))
    .then_some(token)
}

async fn require_identity(s: &AppState, headers: &HeaderMap) -> Result<Identity, ApiError> {
    let token = bearer(headers).ok_or_else(ApiError::auth_required)?;
    let who = s
        .validator
        .validate(token)
        .await?
        .ok_or_else(ApiError::auth_expired)?;
    if Uuid::parse_str(&who.user_id)
        .ok()
        .is_none_or(|id| id.is_nil())
    {
        return Err(ApiError::auth_expired());
    }
    Ok(who)
}

fn request_body(body: Result<Bytes, BytesRejection>) -> Result<Bytes, ApiError> {
    body.map_err(|_| ApiError::invalid_input("request body is unavailable or too large"))
}

fn parse_json<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|e| ApiError::invalid_input("invalid JSON body").detail("error", e.to_string()))
}

async fn create_room(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, ApiError> {
    limit(&s.limiters.create, ip)?;
    let mut req: CreateRoom = parse_json(&request_body(body)?)?;
    req.validate()?;
    let c = match (headers.contains_key(header::AUTHORIZATION), req.auth_mode) {
        (true, _) | (false, AuthMode::Yggdrasil) => {
            let who = require_identity(&s, &headers).await?;
            s.rooms.create(req, &who)?
        }
        // Offline host: anonymous `open` room, strict per-IP limits.
        (false, AuthMode::Open) => {
            limit(&s.limiters.anon_create, ip)?;
            let who = Identity {
                user_id: format!("ip:{ip}"),
                name: "Host".into(),
            };
            s.rooms.create_limited(req, &who, ANON_MAX_ACTIVE_ROOMS)?
        }
    };
    let body = json!({
        "roomId": c.room_id,
        "code": c.code,
        "hostToken": c.host_token,
        "expiresAt": rfc3339::format(c.expires_at),
        "relayUrls": s.cfg.relay_urls,
    });
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinReq {
    endpoint_id: String,
    /// Display name (offline guests); ignored when the bearer is valid.
    #[serde(default)]
    name: Option<String>,
    /// Profile UUID; accepted for symmetry, the token's profile wins when authenticated.
    #[serde(default)]
    #[allow(dead_code)]
    uuid: Option<String>,
}

const MAX_NAME_CHARS: usize = 16;

async fn join_room(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(code): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<Value>, ApiError> {
    limit(&s.limiters.join, ip)?;
    let req: JoinReq = parse_json(&request_body(body)?)?;
    validate_endpoint_id(&req.endpoint_id)?;
    let body_name = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if body_name
        .is_some_and(|n| n.chars().count() > MAX_NAME_CHARS || n.chars().any(char::is_control))
    {
        return Err(ApiError::invalid_input(
            "name must be 1..=16 printable chars",
        ));
    }
    let body_name = body_name.map(str::to_owned);
    let code = normalize_code(&code).ok_or_else(|| ApiError::not_found("room not found"))?;
    let mode = s
        .rooms
        .auth_mode_by_code(&code)
        .ok_or_else(|| ApiError::not_found("room not found"))?;
    // Auth is optional for open rooms (offline-account friends), required otherwise.
    let who = match (mode, headers.contains_key(header::AUTHORIZATION)) {
        (AuthMode::Yggdrasil, _) | (AuthMode::Open, true) => {
            Some(require_identity(&s, &headers).await?)
        }
        (AuthMode::Open, false) => None,
    };
    let uuid = rooms::profile_uuid(who.as_ref().map(|w| w.user_id.as_str()));
    let name = who.map(|w| w.name).or(body_name).unwrap_or_else(|| {
        let tail: String = req.endpoint_id.chars().take(4).collect();
        format!("Guest-{tail}")
    });
    let j = s.rooms.join(&code, req.endpoint_id, name, &uuid)?;
    let mut out = json!({
        "roomId": j.room_id,
        "peerToken": j.peer_token,
        "hostEndpointId": j.host_endpoint_id,
        "kind": j.kind,
        "authMode": j.auth_mode,
        "relayUrls": s.cfg.relay_urls,
        "passwordProtected": j.password_protected,
    });
    // `sig` is absent (not null) for voice rooms.
    if let (Some(sig), Some(o)) = (j.sig, out.as_object_mut()) {
        o.insert("sig".into(), json!(sig));
    }
    Ok(Json(out))
}

async fn close_room(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(room_id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    limit(&s.limiters.close, ip)?;
    let token = bearer(&headers).ok_or_else(ApiError::auth_required)?;
    let room_id =
        Uuid::parse_str(&room_id).map_err(|_| ApiError::invalid_input("roomId must be a UUID"))?;
    // The host may authenticate with its `hostToken` or with its Yggdrasil token.
    if s.rooms.host_token_matches(room_id, token) {
        s.rooms.close(room_id, None, Some(token))?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let who = s
        .validator
        .validate(token)
        .await?
        .ok_or_else(ApiError::auth_expired)?;
    // Unknown/already closed room: idempotent 204.
    s.rooms.close(room_id, Some(&who.user_id), None)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Caller's VPN client key (`mc-<uuid>`) and the backend, after auth and rate limiting.
async fn vpn_caller(
    s: &AppState,
    headers: &HeaderMap,
    rl: &RateLimiter<String>,
) -> Result<(Arc<VpnService>, String), ApiError> {
    let who = require_identity(s, headers).await?;
    let key = vpn::client_key(&who.user_id).ok_or_else(|| {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "authExpired",
            "token has no Yggdrasil profile",
        )
    })?;
    rl.check(&key).map_err(ApiError::rate_limited)?;
    let svc = s.vpn.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "network",
            "VPN is not configured",
        )
    })?;
    Ok((svc, key))
}

async fn get_vpn(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    limit(&s.limiters.vpn_request, ip)?;
    let (svc, key) = vpn_caller(&s, &headers, &s.limiters.vpn_get).await?;
    svc.get(&key)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("no VPN access for this profile"))
}

async fn provision_vpn(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    limit(&s.limiters.vpn_request, ip)?;
    let (svc, key) = vpn_caller(&s, &headers, &s.limiters.vpn_provision).await?;
    svc.provision(&key).await.map(Json)
}

#[derive(Deserialize)]
struct WsQuery {
    token: Option<String>,
}

async fn room_socket(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(room_id): Path<String>,
    query: Result<Query<WsQuery>, axum::extract::rejection::QueryRejection>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    limit(&s.limiters.ws, ip)?;
    let Query(q) = query.map_err(|_| ApiError::auth_required())?;
    let token = q
        .token
        .filter(|t| !t.is_empty() && t.len() <= 256)
        .ok_or_else(ApiError::auth_required)?;
    let room_id = Uuid::parse_str(&room_id).map_err(|_| ApiError::auth_expired())?;
    let me = s.rooms.websocket_member(room_id, &token)?;
    let permit = s
        .ws_connections
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::rate_limited(5))?;
    let upgrade = upgrade.map_err(|_| ApiError::invalid_input("WebSocket upgrade required"))?;
    Ok(upgrade
        .max_message_size(16 * 1024)
        .max_frame_size(16 * 1024)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            ws::run(socket, s, room_id, me).await;
        }))
}

#[cfg(test)]
mod tests;
