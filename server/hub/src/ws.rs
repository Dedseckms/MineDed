//! Room WebSocket: presence and fallback chat (see `roomSocket` in hub.yaml).
//!
//! A guest is a live room member while its socket is open; closing the last
//! socket, `leave` or a host `kick` frees the slot (joinRoom in hub.yaml).

use std::time::{Duration, Instant, SystemTime};

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::ratelimit::RateLimiter;
use crate::rfc3339;
use crate::rooms::Member;
use crate::AppState;

const PING_EVERY: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_TEXT_CHARS: usize = 500;
const MAX_ID_LEN: usize = 64;
const MAX_REASON_CHARS: usize = 120;
const MAX_FRAME: usize = 4 * 1024;

/// Client messages: `chat`, `ping` (contract) plus `leave` (any member) and
/// `kick` (host only) which free a slot as described in joinRoom.
#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum ClientMsg {
    Chat {
        id: String,
        text: String,
    },
    Ping {
        #[allow(dead_code)]
        #[serde(default)]
        nonce: Value,
    },
    Leave {},
    Kick {
        #[serde(rename = "peerId")]
        peer_id: Uuid,
        #[serde(default)]
        reason: String,
    },
}

fn text(v: &Value) -> Message {
    Message::Text(Utf8Bytes::from(v.to_string()))
}

enum Step {
    Continue,
    Stop,
}

fn handle_client(
    state: &AppState,
    room_id: Uuid,
    me: &Member,
    chat: &RateLimiter<()>,
    t: &str,
) -> Step {
    match serde_json::from_str::<ClientMsg>(t) {
        Ok(ClientMsg::Chat { id, text: body }) => {
            let body = body.trim();
            let ok = !body.is_empty()
                && body.chars().count() <= MAX_TEXT_CHARS
                && id.len() <= MAX_ID_LEN
                && chat.check(&()).is_ok();
            if ok {
                let _ = me.events.send(json!({
                    "t": "chat",
                    "id": id,
                    "fromPeer": me.peer_id,
                    "fromName": me.name,
                    "text": body,
                    "sentAt": rfc3339::format(SystemTime::now()),
                }));
            }
            Step::Continue
        }
        Ok(ClientMsg::Leave {}) => Step::Stop,
        Ok(ClientMsg::Kick { peer_id, reason }) => {
            if me.is_host && state.rooms.remove_peer(room_id, peer_id) {
                let reason: String = reason.chars().take(MAX_REASON_CHARS).collect();
                let _ = me
                    .events
                    .send(json!({ "t": "kick", "peerId": peer_id, "reason": reason }));
            }
            Step::Continue
        }
        Ok(ClientMsg::Ping { .. }) | Err(_) => Step::Continue,
    }
}

pub async fn run(mut socket: WebSocket, state: AppState, room_id: Uuid, me: Member) {
    let mut rx = me.events.subscribe();
    if !state.rooms.socket(room_id, me.peer_id, me.is_host, true) {
        let _ = socket.send(text(&json!({ "t": "closed" }))).await;
        let _ = socket.send(Message::Close(None)).await;
        return;
    }
    let _ = me
        .events
        .send(json!({ "t": "peer_joined", "peerId": me.peer_id, "name": me.name }));
    let hello = json!({ "t": "host_endpoint", "endpointId": me.host_endpoint });
    let chat_limit: RateLimiter<()> = RateLimiter::per_minute(60, 10);
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut last_seen = Instant::now();
    let my_id = Value::String(me.peer_id.to_string());

    if socket.send(text(&hello)).await.is_ok() {
        loop {
            tokio::select! {
                incoming = socket.recv() => {
                    let Some(Ok(msg)) = incoming else { break };
                    last_seen = Instant::now();
                    match msg {
                        Message::Text(t) if t.len() <= MAX_FRAME => {
                            if let Step::Stop = handle_client(&state, room_id, &me, &chat_limit, &t) {
                                break;
                            }
                        }
                        Message::Close(_) => break,
                        Message::Text(_) | Message::Binary(_) => break, // oversized/unsupported
                        _ => {}
                    }
                }
                ev = rx.recv() => {
                    match ev {
                        Ok(v) => {
                            let end = v["t"] == "closed" || (v["t"] == "kick" && v["peerId"] == my_id);
                            if socket.send(text(&v)).await.is_err() || end {
                                break;
                            }
                        }
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => {
                            let _ = socket.send(text(&json!({ "t": "closed" }))).await;
                            break;
                        }
                    }
                }
                _ = ping.tick() => {
                    if last_seen.elapsed() > IDLE_TIMEOUT
                        || socket.send(Message::Ping(Default::default())).await.is_err()
                    {
                        break;
                    }
                }
            }
        }
    }

    state.rooms.socket(room_id, me.peer_id, me.is_host, false);
    let _ = me
        .events
        .send(json!({ "t": "peer_left", "peerId": me.peer_id }));
    let _ = socket.send(Message::Close(None)).await;
}
