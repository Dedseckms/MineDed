//! Hub HTTP API. Routes and shapes follow `server/openapi/hub.yaml`
//! (CONTRACT, lead). Implemented: health, rooms (+WS). Mirrors and skins are 501 stubs.

pub mod auth;
pub mod error;
pub mod ratelimit;
pub mod rfc3339;
pub mod rooms;
mod ws;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::{ConnectInfo, FromRequestParts, Path, Query, State, WebSocketUpgrade};
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

pub const API_PREFIX: &str = "/api/v1";

/// Runtime configuration (from env in `main.rs`).
#[derive(Debug, Clone)]
pub struct Config {
    /// Public iroh-relay URLs returned to clients.
    pub relay_urls: Vec<String>,
    pub limits: Limits,
    /// Take the client IP from `X-Real-IP` (set by the reverse proxy).
    pub trust_proxy: bool,
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
}

#[derive(Clone)]
pub struct AppState {
    cfg: Arc<Config>,
    validator: Arc<dyn TokenValidator>,
    rooms: Arc<RoomStore>,
    limiters: Arc<Limiters>,
}

impl AppState {
    pub fn new(cfg: Config, validator: Arc<dyn TokenValidator>) -> Self {
        let rooms = Arc::new(RoomStore::new(cfg.limits));
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
            }),
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
            }
        })
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/mirrors", get(not_implemented))
        .route("/skins", get(not_implemented).post(not_implemented))
        .route("/skins/{id}", delete(not_implemented))
        .route("/me/skin", post(not_implemented))
        .route("/rooms", post(create_room))
        .route("/rooms/{code}/join", post(join_room))
        .route("/rooms/{room_id}", delete(close_room))
        .route("/rooms/{room_id}/ws", get(room_socket))
        .fallback(|| async { ApiError::not_found("no such route") })
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
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

async fn require_identity(s: &AppState, headers: &HeaderMap) -> Result<Identity, ApiError> {
    let token = bearer(headers).ok_or_else(ApiError::auth_required)?;
    s.validator
        .validate(token)
        .await?
        .ok_or_else(ApiError::auth_expired)
}

fn parse_json<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|e| ApiError::invalid_input("invalid JSON body").detail("error", e.to_string()))
}

async fn create_room(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    limit(&s.limiters.create, ip)?;
    let req: CreateRoom = parse_json(&body)?;
    req.validate()?;
    let c = match (bearer(&headers), req.auth_mode) {
        (Some(_), _) | (None, AuthMode::Yggdrasil) => {
            let who = require_identity(&s, &headers).await?;
            s.rooms.create(req, &who)?
        }
        // Offline host: anonymous `open` room, strict per-IP limits.
        (None, AuthMode::Open) => {
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
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    limit(&s.limiters.join, ip)?;
    let req: JoinReq = parse_json(&body)?;
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
    let who = match (mode, bearer(&headers)) {
        (AuthMode::Yggdrasil, None) => return Err(ApiError::auth_required()),
        (AuthMode::Yggdrasil, Some(t)) => Some(
            s.validator
                .validate(t)
                .await?
                .ok_or_else(ApiError::auth_expired)?,
        ),
        (AuthMode::Open, Some(t)) => s.validator.validate(t).await.ok().flatten(),
        (AuthMode::Open, None) => None,
    };
    let uuid = rooms::profile_uuid(who.as_ref().map(|w| w.user_id.as_str()));
    let name = who.map(|w| w.name).or(body_name).unwrap_or_else(|| {
        let tail: String = req.endpoint_id.chars().take(4).collect();
        format!("Guest-{tail}")
    });
    let j = s.rooms.join(&code, req.endpoint_id, name, &uuid)?;
    Ok(Json(json!({
        "roomId": j.room_id,
        "peerToken": j.peer_token,
        "hostEndpointId": j.host_endpoint_id,
        "sig": j.sig,
        "authMode": j.auth_mode,
        "relayUrls": s.cfg.relay_urls,
        "passwordProtected": j.password_protected,
    })))
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

#[derive(Deserialize)]
struct WsQuery {
    token: Option<String>,
}

async fn room_socket(
    State(s): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(room_id): Path<String>,
    Query(q): Query<WsQuery>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    limit(&s.limiters.ws, ip)?;
    let token = q
        .token
        .filter(|t| !t.is_empty())
        .ok_or_else(ApiError::auth_required)?;
    let room_id = Uuid::parse_str(&room_id).map_err(|_| ApiError::auth_expired())?;
    let me = s
        .rooms
        .member(room_id, &token)
        .ok_or_else(ApiError::auth_expired)?;
    let upgrade = upgrade.map_err(|_| ApiError::invalid_input("WebSocket upgrade required"))?;
    Ok(upgrade
        .max_message_size(16 * 1024)
        .on_upgrade(move |socket| ws::run(socket, s, room_id, me)))
}

#[cfg(test)]
mod tests;
