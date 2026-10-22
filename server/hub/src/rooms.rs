//! In-memory room registry with TTL. Rooms are lost on restart by design
//! (hosts re-create them; P2P sessions keep running over iroh).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::auth::Identity;
use crate::error::ApiError;

/// Room code alphabet: 32 symbols without 0/O/1/I. `byte & 31` is unbiased.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const EVENTS_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    Yggdrasil,
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSig {
    pub mc: String,
    pub loader: Loader,
    #[serde(default)]
    pub loader_version: Option<String>,
    pub mods_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Loader {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoom {
    pub endpoint_id: String,
    pub sig: InstanceSig,
    pub auth_mode: AuthMode,
    #[serde(default = "default_max_peers")]
    pub max_peers: u8,
    #[serde(default)]
    pub password_protected: bool,
}

fn default_max_peers() -> u8 {
    8
}

impl CreateRoom {
    pub fn validate(&self) -> Result<(), ApiError> {
        validate_endpoint_id(&self.endpoint_id)?;
        if !(2..=16).contains(&self.max_peers) {
            return Err(ApiError::invalid_input("maxPeers must be 2..=16"));
        }
        let s = &self.sig;
        let lv = s.loader_version.as_deref().unwrap_or("");
        if s.mc.is_empty() || s.mc.len() > 64 || s.mods_hash.len() > 128 || lv.len() > 64 {
            return Err(ApiError::invalid_input("sig fields are empty or too long"));
        }
        Ok(())
    }
}

pub fn validate_endpoint_id(id: &str) -> Result<(), ApiError> {
    if id.is_empty() || id.len() > 128 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(ApiError::invalid_input(
            "endpointId must be 1..=128 alphanumeric chars",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Peer {
    pub peer_id: Uuid,
    pub token: String,
    pub endpoint_id: String,
    pub name: String,
}

#[derive(Debug)]
pub struct Room {
    pub id: Uuid,
    pub code: String,
    pub owner: String,
    pub host: Peer,
    pub sig: InstanceSig,
    pub auth_mode: AuthMode,
    pub max_peers: u8,
    pub password_protected: bool,
    pub expires_at: SystemTime,
    deadline: Instant,
    /// Open WebSocket connections of the host.
    pub host_sockets: usize,
    pub peers: Vec<Peer>,
    pub events: broadcast::Sender<Value>,
}

/// Who a WS token belongs to.
#[derive(Debug, Clone)]
pub struct Member {
    pub peer_id: Uuid,
    pub name: String,
    pub is_host: bool,
    pub host_endpoint: String,
    pub events: broadcast::Sender<Value>,
}

pub struct Created {
    pub room_id: Uuid,
    pub code: String,
    pub host_token: String,
    pub expires_at: SystemTime,
}

pub struct Joined {
    pub room_id: Uuid,
    pub peer_token: String,
    pub host_endpoint_id: String,
    pub sig: InstanceSig,
    pub auth_mode: AuthMode,
    pub password_protected: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub ttl: Duration,
    pub max_rooms: usize,
    pub max_rooms_per_user: usize,
}

pub struct RoomStore {
    limits: Limits,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    rooms: HashMap<Uuid, Room>,
    by_code: HashMap<String, Uuid>,
}

fn new_token(prefix: &str) -> String {
    format!(
        "{prefix}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn new_code() -> String {
    let b = Uuid::new_v4().into_bytes();
    let c: String = b
        .iter()
        .take(8)
        .map(|x| CODE_ALPHABET[(x & 31) as usize] as char)
        .collect();
    format!("{}-{}", &c[..4], &c[4..])
}

/// Accepts `abcd-efgh`, `ABCDEFGH`, surrounding spaces.
pub fn normalize_code(code: &str) -> Option<String> {
    let c: String = code
        .trim()
        .chars()
        .filter(|c| *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (c.len() == 8 && c.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| format!("{}-{}", &c[..4], &c[4..]))
}

impl RoomStore {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn create(&self, req: CreateRoom, who: &Identity) -> Result<Created, ApiError> {
        let mut inner = self.lock();
        if inner.rooms.len() >= self.limits.max_rooms {
            return Err(ApiError::rate_limited(60).detail("limit", "maxRooms"));
        }
        let owned = inner
            .rooms
            .values()
            .filter(|r| r.owner == who.user_id)
            .count();
        if owned >= self.limits.max_rooms_per_user {
            return Err(ApiError::rate_limited(60).detail("limit", "maxRoomsPerUser"));
        }
        let mut code = new_code();
        while inner.by_code.contains_key(&code) {
            code = new_code();
        }
        let id = Uuid::new_v4();
        let host = Peer {
            peer_id: Uuid::new_v4(),
            token: new_token("rh_"),
            endpoint_id: req.endpoint_id,
            name: who.name.clone(),
        };
        let expires_at = SystemTime::now() + self.limits.ttl;
        let created = Created {
            room_id: id,
            code: code.clone(),
            host_token: host.token.clone(),
            expires_at,
        };
        let (events, _) = broadcast::channel(EVENTS_CAPACITY);
        inner.by_code.insert(code.clone(), id);
        inner.rooms.insert(
            id,
            Room {
                id,
                code,
                owner: who.user_id.clone(),
                host,
                sig: req.sig,
                auth_mode: req.auth_mode,
                max_peers: req.max_peers,
                password_protected: req.password_protected,
                expires_at,
                deadline: Instant::now() + self.limits.ttl,
                host_sockets: 0,
                peers: Vec::new(),
                events,
            },
        );
        Ok(created)
    }

    pub fn auth_mode_by_code(&self, code: &str) -> Option<AuthMode> {
        let inner = self.lock();
        let id = inner.by_code.get(code)?;
        inner.rooms.get(id).map(|r| r.auth_mode)
    }

    /// Joins by code. Re-joining with the same endpoint returns the same token.
    pub fn join(&self, code: &str, endpoint_id: String, name: String) -> Result<Joined, ApiError> {
        let mut inner = self.lock();
        let id = *inner
            .by_code
            .get(code)
            .ok_or_else(|| ApiError::not_found("room not found"))?;
        let room = inner
            .rooms
            .get_mut(&id)
            .ok_or_else(|| ApiError::not_found("room not found"))?;
        let token = match room.peers.iter().find(|p| p.endpoint_id == endpoint_id) {
            Some(p) => p.token.clone(),
            None => {
                if room.peers.len() + 1 >= usize::from(room.max_peers) {
                    return Err(
                        ApiError::new(StatusCode::CONFLICT, "conflict", "room is full")
                            .detail("maxPeers", room.max_peers.to_string()),
                    );
                }
                let peer = Peer {
                    peer_id: Uuid::new_v4(),
                    token: new_token("rp_"),
                    endpoint_id,
                    name,
                };
                let t = peer.token.clone();
                room.peers.push(peer);
                t
            }
        };
        Ok(Joined {
            room_id: room.id,
            peer_token: token,
            host_endpoint_id: room.host.endpoint_id.clone(),
            sig: room.sig.clone(),
            auth_mode: room.auth_mode,
            password_protected: room.password_protected,
        })
    }

    pub fn host_token_matches(&self, room_id: Uuid, token: &str) -> bool {
        self.lock()
            .rooms
            .get(&room_id)
            .is_some_and(|r| r.host.token == token)
    }

    /// `Ok(true)` closed, `Ok(false)` no such room, `Err` = 403 (not the owner).
    pub fn close(
        &self,
        room_id: Uuid,
        user_id: Option<&str>,
        host_token: Option<&str>,
    ) -> Result<bool, ApiError> {
        let mut inner = self.lock();
        let Some(room) = inner.rooms.get(&room_id) else {
            return Ok(false);
        };
        let allowed = host_token.is_some_and(|t| t == room.host.token)
            || user_id.is_some_and(|u| u == room.owner);
        if !allowed {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "permissionDenied",
                "not the room host",
            ));
        }
        if let Some(room) = inner.rooms.remove(&room_id) {
            inner.by_code.remove(&room.code);
            let _ = room.events.send(json!({ "t": "closed" }));
        }
        Ok(true)
    }

    pub fn member(&self, room_id: Uuid, token: &str) -> Option<Member> {
        let inner = self.lock();
        let room = inner.rooms.get(&room_id)?;
        let (p, is_host) = if room.host.token == token {
            (&room.host, true)
        } else {
            (room.peers.iter().find(|p| p.token == token)?, false)
        };
        Some(Member {
            peer_id: p.peer_id,
            name: p.name.clone(),
            is_host,
            host_endpoint: room.host.endpoint_id.clone(),
            events: room.events.clone(),
        })
    }

    pub fn host_socket(&self, room_id: Uuid, connected: bool) {
        let mut inner = self.lock();
        if let Some(r) = inner.rooms.get_mut(&room_id) {
            if connected {
                r.host_sockets += 1;
            } else {
                r.host_sockets = r.host_sockets.saturating_sub(1);
            }
        }
    }

    /// Removes expired rooms. A room whose host is connected over WS is kept alive.
    pub fn reap(&self) -> usize {
        self.reap_at(Instant::now())
    }

    fn reap_at(&self, now: Instant) -> usize {
        let ttl = self.limits.ttl;
        let mut inner = self.lock();
        let mut expired = Vec::new();
        for r in inner.rooms.values_mut() {
            if r.deadline <= now {
                if r.host_sockets > 0 {
                    r.deadline = now + ttl;
                    r.expires_at = SystemTime::now() + ttl;
                } else {
                    expired.push(r.id);
                }
            }
        }
        for id in &expired {
            if let Some(room) = inner.rooms.remove(id) {
                inner.by_code.remove(&room.code);
                let _ = room.events.send(json!({ "t": "closed" }));
            }
        }
        expired.len()
    }

    pub fn len(&self) -> usize {
        self.lock().rooms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            ttl: Duration::from_secs(60),
            max_rooms: 3,
            max_rooms_per_user: 2,
        }
    }

    fn req() -> CreateRoom {
        serde_json::from_value(json!({
            "endpointId": "abc123",
            "sig": { "mc": "1.21.1", "loader": "neoForge", "modsHash": "h" },
            "authMode": "open",
            "maxPeers": 2
        }))
        .unwrap()
    }

    fn who(u: &str) -> Identity {
        Identity {
            user_id: u.into(),
            name: u.into(),
        }
    }

    #[test]
    fn code_format_and_normalize() {
        for _ in 0..100 {
            let c = new_code();
            assert_eq!(c.len(), 9);
            assert_eq!(
                normalize_code(&c.to_lowercase().replace('-', "")),
                Some(c.clone())
            );
        }
        assert_eq!(normalize_code("abc"), None);
        assert_eq!(normalize_code("ABCD-EF!H"), None);
    }

    #[test]
    fn limits_full_rejoin_and_ttl() {
        let s = RoomStore::new(limits());
        let a = s.create(req(), &who("a")).unwrap();
        s.create(req(), &who("a")).unwrap();
        assert_eq!(
            s.create(req(), &who("a")).err().unwrap().status,
            StatusCode::TOO_MANY_REQUESTS
        );
        s.create(req(), &who("b")).unwrap();
        assert_eq!(
            s.create(req(), &who("c")).err().unwrap().status,
            StatusCode::TOO_MANY_REQUESTS
        );

        let j1 = s.join(&a.code, "g1".into(), "G".into()).unwrap();
        let j2 = s.join(&a.code, "g1".into(), "G".into()).unwrap();
        assert_eq!(j1.peer_token, j2.peer_token);
        let full = s.join(&a.code, "g2".into(), "G".into()).err().unwrap();
        assert_eq!(full.status, StatusCode::CONFLICT);

        // Host connected keeps the room alive past its deadline.
        s.host_socket(a.room_id, true);
        assert_eq!(s.reap_at(Instant::now() + Duration::from_secs(61)), 2);
        assert_eq!(s.len(), 1);
        s.host_socket(a.room_id, false);
        assert_eq!(s.reap_at(Instant::now() + Duration::from_secs(200)), 1);
        assert!(s.is_empty());
    }

    #[test]
    fn close_requires_owner_or_host_token() {
        let s = RoomStore::new(limits());
        let a = s.create(req(), &who("a")).unwrap();
        let mut rx = s
            .member(a.room_id, &a.host_token)
            .unwrap()
            .events
            .subscribe();
        assert!(s.close(a.room_id, Some("b"), Some("nope")).is_err());
        assert!(s.close(a.room_id, None, Some(&a.host_token)).unwrap());
        assert_eq!(rx.try_recv().unwrap()["t"], "closed");
        assert!(!s.close(a.room_id, Some("a"), None).unwrap());
    }
}
