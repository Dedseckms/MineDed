//! Room WebSocket: presence and fallback chat (see `roomSocket` in hub.yaml).

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
const MAX_FRAME: usize = 4 * 1024;

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
}

fn text(v: &Value) -> Message {
    Message::Text(Utf8Bytes::from(v.to_string()))
}

pub async fn run(mut socket: WebSocket, state: AppState, room_id: Uuid, me: Member) {
    let mut rx = me.events.subscribe();
    if me.is_host {
        state.rooms.host_socket(room_id, true);
    }
    let _ = me
        .events
        .send(json!({ "t": "peer_joined", "peerId": me.peer_id, "name": me.name }));
    let hello = json!({ "t": "host_endpoint", "endpointId": me.host_endpoint });
    let chat_limit: RateLimiter<()> = RateLimiter::per_minute(60, 10);
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut last_seen = Instant::now();

    if socket.send(text(&hello)).await.is_ok() {
        loop {
            tokio::select! {
                incoming = socket.recv() => {
                    let Some(Ok(msg)) = incoming else { break };
                    last_seen = Instant::now();
                    match msg {
                        Message::Text(t) if t.len() <= MAX_FRAME => {
                            if let Ok(ClientMsg::Chat { id, text: body }) = serde_json::from_str(&t) {
                                let body = body.trim();
                                let ok = !body.is_empty()
                                    && body.chars().count() <= MAX_TEXT_CHARS
                                    && id.len() <= MAX_ID_LEN
                                    && chat_limit.check(&()).is_ok();
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
                            let closed = v["t"] == "closed";
                            if socket.send(text(&v)).await.is_err() || closed {
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

    if me.is_host {
        state.rooms.host_socket(room_id, false);
    }
    let _ = me
        .events
        .send(json!({ "t": "peer_left", "peerId": me.peer_id }));
    let _ = socket.send(Message::Close(None)).await;
}
