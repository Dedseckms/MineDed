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

/// `game` rooms carry an instance signature and a game tunnel; `voice` = chat/voice only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomKind {
    #[default]
    Game,
    Voice,
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
    #[serde(default)]
    pub kind: RoomKind,
    /// Required for `game` rooms; ignored (dropped) for `voice` rooms.
    #[serde(default)]
    pub sig: Option<InstanceSig>,
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
    /// Validates fields and normalizes `sig` (dropped for voice rooms).
    pub fn validate(&mut self) -> Result<(), ApiError> {
        validate_endpoint_id(&self.endpoint_id)?;
        if !(2..=16).contains(&self.max_peers) {
            return Err(ApiError::invalid_input("maxPeers must be 2..=16"));
        }
        if self.kind == RoomKind::Voice {
            self.sig = None;
            return Ok(());
        }
        let Some(s) = &self.sig else {
            return Err(ApiError::invalid_input("sig is required for game rooms"));
        };
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
    /// Last (re)join; a guest without WS stays a member for [`JOIN_GRACE`].
    joined_at: Instant,
    /// Open WebSocket connections.
    sockets: usize,
}

impl Peer {
    fn is_live(&self, now: Instant) -> bool {
        self.sockets > 0 || now.saturating_duration_since(self.joined_at) <= JOIN_GRACE
    }
}

/// A guest that never connects the WS keeps its slot this long after join.
pub const JOIN_GRACE: Duration = Duration::from_secs(60);
/// `peerToken` lifetime (hub.yaml joinRoom).
pub const PEER_TOKEN_TTL_SECS: u64 = 600;

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `peerToken` v2 = `<exp>.<uuid|->.<base64url-nopad(HMAC-SHA256(hostToken,
/// "lc-peer/2|roomId|endpointId|uuid|exp"))>`; the host verifies it offline.
/// `uuid` is the undashed profile UUID of a validated bearer, or `-` (anonymous).
pub fn peer_token(
    host_token: &str,
    room_id: Uuid,
    endpoint_id: &str,
    uuid: &str,
    exp: u64,
) -> String {
    use base64::Engine;
    let msg = format!("lc-peer/2|{room_id}|{endpoint_id}|{uuid}|{exp}");
    let mac = hmac_sha256(host_token.as_bytes(), msg.as_bytes());
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac);
    format!("{exp}.{uuid}.{b64}")
}

/// Token-segment form of a profile id: undashed lowercase UUID, else `-`.
pub fn profile_uuid(user_id: Option<&str>) -> String {
    user_id
        .and_then(|u| Uuid::parse_str(u).ok())
        .map(|u| u.simple().to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    // HMAC accepts keys of any length, so `new_from_slice` never fails.
    Hmac::<sha2::Sha256>::new_from_slice(key)
        .map(|mut m| {
            m.update(msg);
            m.finalize().into_bytes().to_vec()
        })
        .unwrap_or_default()
}

fn token_exp(token: &str) -> Option<u64> {
    token.split_once('.')?.0.parse().ok()
}

#[derive(Debug)]
pub struct Room {
    pub id: Uuid,
    pub code: String,
    pub owner: String,
    pub host: Peer,
    pub kind: RoomKind,
    pub sig: Option<InstanceSig>,
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
    pub kind: RoomKind,
    pub sig: Option<InstanceSig>,
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
        self.create_limited(req, who, self.limits.max_rooms_per_user)
    }

    /// Like [`Self::create`] with a custom cap of active rooms for `who`.
    pub fn create_limited(
        &self,
        req: CreateRoom,
        who: &Identity,
        max_owned: usize,
    ) -> Result<Created, ApiError> {
        let mut inner = self.lock();
        if inner.rooms.len() >= self.limits.max_rooms {
            return Err(ApiError::rate_limited(60).detail("limit", "maxRooms"));
        }
        let owned = inner
            .rooms
            .values()
            .filter(|r| r.owner == who.user_id)
            .count();
        if owned >= max_owned {
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
            joined_at: Instant::now(),
            sockets: 0,
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
                kind: req.kind,
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

    /// Joins by code. Only live members count toward `maxPeers` (see [`Peer::is_live`]).
    /// Re-joining with the same endpoint keeps the slot and issues a fresh token.
    /// `uuid` = [`profile_uuid`] of the caller (`-` for anonymous guests).
    pub fn join(
        &self,
        code: &str,
        endpoint_id: String,
        name: String,
        uuid: &str,
    ) -> Result<Joined, ApiError> {
        self.join_full(code, endpoint_id, name, uuid, Instant::now(), unix_now())
    }

    #[cfg(test)]
    fn join_at(
        &self,
        code: &str,
        endpoint_id: String,
        name: String,
        now: Instant,
        unix: u64,
    ) -> Result<Joined, ApiError> {
        self.join_full(code, endpoint_id, name, "-", now, unix)
    }

    fn join_full(
        &self,
        code: &str,
        endpoint_id: String,
        name: String,
        uuid: &str,
        now: Instant,
        unix: u64,
    ) -> Result<Joined, ApiError> {
        let mut inner = self.lock();
        let id = *inner
            .by_code
            .get(code)
            .ok_or_else(|| ApiError::not_found("room not found"))?;
        let room = inner
            .rooms
            .get_mut(&id)
            .ok_or_else(|| ApiError::not_found("room not found"))?;
        // Stale guests (joined, never connected, grace over) free their slot.
        room.peers
            .retain(|p| p.is_live(now) || p.endpoint_id == endpoint_id);
        let token = peer_token(
            &room.host.token,
            room.id,
            &endpoint_id,
            uuid,
            unix + PEER_TOKEN_TTL_SECS,
        );
        match room.peers.iter_mut().find(|p| p.endpoint_id == endpoint_id) {
            Some(p) => {
                p.token = token.clone();
                p.joined_at = now;
                p.name = name;
            }
            None => {
                if room.peers.len() + 1 >= usize::from(room.max_peers) {
                    return Err(
                        ApiError::new(StatusCode::CONFLICT, "conflict", "room is full")
                            .detail("maxPeers", room.max_peers.to_string()),
                    );
                }
                room.peers.push(Peer {
                    peer_id: Uuid::new_v4(),
                    token: token.clone(),
                    endpoint_id,
                    name,
                    joined_at: now,
                    sockets: 0,
                });
            }
        }
        Ok(Joined {
            room_id: room.id,
            peer_token: token,
            host_endpoint_id: room.host.endpoint_id.clone(),
            kind: room.kind,
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
            if token_exp(token)? < unix_now() {
                return None;
            }
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

    /// Tracks a member's WS. Returns `false` if the member is gone (left/kicked/closed).
    /// When a guest's last socket closes, the guest leaves and its slot is freed.
    pub fn socket(&self, room_id: Uuid, peer_id: Uuid, is_host: bool, connected: bool) -> bool {
        let mut inner = self.lock();
        let Some(r) = inner.rooms.get_mut(&room_id) else {
            return false;
        };
        if is_host {
            if connected {
                r.host_sockets += 1;
            } else {
                r.host_sockets = r.host_sockets.saturating_sub(1);
            }
            return true;
        }
        let Some(i) = r.peers.iter().position(|p| p.peer_id == peer_id) else {
            return false;
        };
        if connected {
            r.peers[i].sockets += 1;
        } else {
            r.peers[i].sockets = r.peers[i].sockets.saturating_sub(1);
            if r.peers[i].sockets == 0 {
                r.peers.remove(i);
            }
        }
        true
    }

    /// Removes a guest (leave or host kick). Returns `true` if it was a member.
    pub fn remove_peer(&self, room_id: Uuid, peer_id: Uuid) -> bool {
        let mut inner = self.lock();
        let Some(r) = inner.rooms.get_mut(&room_id) else {
            return false;
        };
        let before = r.peers.len();
        r.peers.retain(|p| p.peer_id != peer_id);
        r.peers.len() != before
    }

    /// Live members including the host (for tests and diagnostics).
    pub fn live_members(&self, room_id: Uuid) -> usize {
        let now = Instant::now();
        self.lock()
            .rooms
            .get(&room_id)
            .map(|r| 1 + r.peers.iter().filter(|p| p.is_live(now)).count())
            .unwrap_or(0)
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

        // Re-join keeps the slot; a second guest does not fit (maxPeers=2 incl. host).
        s.join(&a.code, "g1".into(), "G".into(), "-").unwrap();
        s.join(&a.code, "g1".into(), "G".into(), "-").unwrap();
        let full = s.join(&a.code, "g2".into(), "G".into(), "-").err().unwrap();
        assert_eq!(full.status, StatusCode::CONFLICT);

        // Host connected keeps the room alive past its deadline.
        let host = s.member(a.room_id, &a.host_token).unwrap();
        assert!(s.socket(a.room_id, host.peer_id, true, true));
        assert_eq!(s.reap_at(Instant::now() + Duration::from_secs(61)), 2);
        assert_eq!(s.len(), 1);
        s.socket(a.room_id, host.peer_id, true, false);
        assert_eq!(s.reap_at(Instant::now() + Duration::from_secs(200)), 1);
        assert!(s.is_empty());
    }

    #[test]
    fn peer_token_format_and_binding() {
        let s = RoomStore::new(limits());
        let a = s.create(req(), &who("a")).unwrap();
        let uuid = profile_uuid(Some("069a79f4-44e9-4726-a5be-fca90e38aaf5"));
        assert_eq!(uuid, "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(profile_uuid(Some("token:abc")), "-");
        assert_eq!(profile_uuid(None), "-");
        let j = s.join(&a.code, "guest1".into(), "G".into(), &uuid).unwrap();
        let parts: Vec<&str> = j.peer_token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let exp: u64 = parts[0].parse().unwrap();
        let now = unix_now();
        assert!(exp >= now + PEER_TOKEN_TTL_SECS - 5 && exp <= now + PEER_TOKEN_TTL_SECS + 5);
        assert_eq!(parts[1], uuid);
        let mac = parts[2];
        assert_eq!(mac.len(), 43); // 32 bytes, base64url without padding
        assert!(mac
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_eq!(
            j.peer_token,
            peer_token(&a.host_token, a.room_id, "guest1", &uuid, exp)
        );
        assert_ne!(
            j.peer_token,
            peer_token(&a.host_token, a.room_id, "guest2", &uuid, exp)
        );
        // Anonymous guests carry `-`; the MAC covers the uuid segment.
        let anon = s.join(&a.code, "guest1".into(), "G".into(), "-").unwrap();
        assert_eq!(anon.peer_token.split('.').nth(1), Some("-"));
        assert_ne!(
            peer_token(&a.host_token, a.room_id, "guest1", "-", exp),
            peer_token(&a.host_token, a.room_id, "guest1", &uuid, exp)
        );
        // RFC 4231 test case 2.
        let kat: String = hmac_sha256(b"Jefe", b"what do ya want for nothing?")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            kat,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Re-join replaced the token: only the latest one opens the WS.
        assert!(s.member(a.room_id, &anon.peer_token).is_some());
        assert!(s.member(a.room_id, &j.peer_token).is_none());
        assert!(s.member(a.room_id, "1.AAAA").is_none());
    }

    #[test]
    fn only_live_members_count() {
        let s = RoomStore::new(limits());
        let a = s.create(req(), &who("a")).unwrap(); // maxPeers 2 = host + 1 guest
        let t0 = Instant::now();
        let u = unix_now();
        // Join/leave x10 through the WS lifecycle never hits 409.
        for i in 0..10 {
            let j = s
                .join_at(&a.code, format!("g{i}"), "G".into(), t0, u)
                .unwrap();
            let m = s.member(a.room_id, &j.peer_token).unwrap();
            assert!(s.socket(a.room_id, m.peer_id, false, true));
            assert_eq!(s.live_members(a.room_id), 2);
            assert!(s.socket(a.room_id, m.peer_id, false, false)); // WS close frees the slot
            assert_eq!(s.live_members(a.room_id), 1);
        }
        // Leave/kick frees the slot too.
        let j = s.join_at(&a.code, "k".into(), "G".into(), t0, u).unwrap();
        let m = s.member(a.room_id, &j.peer_token).unwrap();
        assert!(s.remove_peer(a.room_id, m.peer_id));
        assert!(s.member(a.room_id, &j.peer_token).is_none());
        // Joined without WS: holds the slot for the grace period only.
        s.join_at(&a.code, "idle".into(), "G".into(), t0, u)
            .unwrap();
        let busy = s.join_at(
            &a.code,
            "x".into(),
            "G".into(),
            t0 + Duration::from_secs(30),
            u,
        );
        assert_eq!(busy.err().unwrap().status, StatusCode::CONFLICT);
        let later = t0 + JOIN_GRACE + Duration::from_secs(1);
        assert!(s.join_at(&a.code, "x".into(), "G".into(), later, u).is_ok());
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
