//! `RoomService` + `DatagramTransport` on iroh.
//!
//! Topology is a star: every guest holds one QUIC connection to the host
//! (direct after hole punching, otherwise through our iroh-relay). The host
//! forwards chat and voice datagrams between guests.
//!
//! Host admission (in order): hub `peerToken` bound to the guest's
//! TLS-authenticated endpoint id, ban list, room size, password proof with a
//! per-endpoint attempt limit.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use iroh::endpoint::{presets, Builder, Connection, Incoming, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey};
use lc_accounts::Secret;
use lc_http::{Downloader, Route};
use lc_runtime::SettingsStore;
use lc_types::room::{InstanceSig, PathKind, Peer, RoomAuthMode, RoomKind, RoomRole, RoomState};
use lc_types::voice::ChatMessage;
use lc_types::{AppError, AppResult, ErrorCode, InstanceId, RoomId};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::code::{join_link, parse_code};
use crate::contract::{DatagramTransport, RoomObserver, RoomService};
use crate::env::{Identity, RoomEnv};
use crate::hub::{CreateRoomReq, HubClient};
use crate::lan::{self, LanSeen};
use crate::proto::{ControlMsg, StreamOpen, ALPN, PROTOCOL_VERSION};
use crate::wire::{self, MAX_LINE, MAX_OPEN_LINE};

/// Hook to adjust the iroh endpoint builder (tests: trust a self-signed relay,
/// add address lookups). Applied after the defaults.
pub type EndpointTweak = Arc<dyn Fn(Builder) -> Builder + Send + Sync>;

const DEFAULT_DATAGRAM: usize = 1200;
const MAX_CHAT_CHARS: usize = 500;
const MAX_NAME_CHARS: usize = 32;
const PATH_POLL: Duration = Duration::from_secs(2);
const DGRAM_QUEUE: usize = 256;
/// Pending control lines per peer; a peer that falls this far behind is dropped.
const CONTROL_QUEUE: usize = 256;
/// Wrong passwords per endpoint before the host starts refusing it.
const MAX_PASSWORD_FAILS: u32 = 5;
/// First lockout after `MAX_PASSWORD_FAILS`; doubles on every further failure.
const PASSWORD_BACKOFF: Duration = Duration::from_secs(30);
const PASSWORD_BACKOFF_MAX: Duration = Duration::from_secs(3600);

/// Kick / reject reasons on the wire (`ControlMsg::Kick::reason`).
pub mod reasons {
    pub const BAD_PASSWORD: &str = "bad_password";
    pub const BAD_TOKEN: &str = "bad_token";
    pub const BANNED: &str = "banned";
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const ROOM_FULL: &str = "room_full";
    pub const PROTOCOL: &str = "protocol";
    pub const KICKED: &str = "kicked";
    pub const SLOW: &str = "slow";
    pub const CLOSED: &str = "closed";
}

#[derive(Clone)]
pub struct P2pConfig {
    /// Relay URLs used instead of the ones returned by hub (tests, self-hosting).
    /// An empty list disables relays (direct connections only).
    pub relay_override: Option<Vec<String>>,
    /// How long `create` waits for "Open to LAN".
    pub lan_wait: Duration,
    /// Limit for the `Hello` / `Welcome` handshake.
    pub hello_timeout: Duration,
    /// Limit for reaching the host.
    pub connect_timeout: Duration,
    /// Including the host.
    pub max_peers: u32,
    /// Listen for / send LAN multicast announcements (off in tests).
    pub lan_multicast: bool,
    pub endpoint_tweak: Option<EndpointTweak>,
}

impl Default for P2pConfig {
    fn default() -> Self {
        Self {
            relay_override: None,
            lan_wait: Duration::from_secs(120),
            hello_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(30),
            max_peers: 8,
            lan_multicast: true,
            endpoint_tweak: None,
        }
    }
}

impl fmt::Debug for P2pConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("P2pConfig")
            .field("relay_override", &self.relay_override)
            .field("lan_wait", &self.lan_wait)
            .field("hello_timeout", &self.hello_timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("max_peers", &self.max_peers)
            .field("lan_multicast", &self.lan_multicast)
            .field("endpoint_tweak", &self.endpoint_tweak.is_some())
            .finish()
    }
}

struct PeerEntry {
    peer: Peer,
    /// `None` for peers we only know from the host's `Welcome` (guest side).
    conn: Option<Connection>,
    /// Control stream writer (bounded).
    tx: Option<mpsc::Sender<String>>,
    /// Host side, yggdrasil rooms: normalised uuid from the verified peer token.
    ban_uuid: Option<String>,
}

#[derive(Default)]
struct Peers {
    /// Welcome order: host first, then guests by join time (voice speaker index).
    order: Vec<String>,
    map: BTreeMap<String, PeerEntry>,
}

struct HostCtx {
    hub: HubClient,
    host_token: Secret,
    password: Option<Secret>,
    /// LAN port fixed at create time; never re-read while the room lives.
    lan_port: u16,
    /// Endpoint ids and (yggdrasil rooms) profile uuids kicked from this room.
    banned: StdMutex<HashSet<String>>,
    /// Wrong password attempts per endpoint id: (count, last failure).
    pw_fails: StdMutex<HashMap<String, (u32, Instant)>>,
}

pub(crate) struct Session {
    pub(crate) role: RoomRole,
    pub(crate) room_id: RoomId,
    code: String,
    auth_mode: RoomAuthMode,
    sig: InstanceSig,
    expires_at: String,
    password_protected: bool,
    pub(crate) local_addr: Option<SocketAddr>,
    my_id: String,
    my_name: String,
    endpoint: Endpoint,
    cancel: CancellationToken,
    peers: StdMutex<Peers>,
    dgram_tx: mpsc::Sender<(String, Vec<u8>)>,
    dgram_rx: tokio::sync::Mutex<mpsc::Receiver<(String, Vec<u8>)>>,
    host: Option<HostCtx>,
}

fn lock<T>(m: &StdMutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Session {
    fn peers(&self) -> MutexGuard<'_, Peers> {
        lock(&self.peers)
    }

    fn state(&self) -> RoomState {
        let peers = self.peers();
        let mut list: Vec<Peer> = peers
            .order
            .iter()
            .filter(|id| **id != self.my_id)
            .filter_map(|id| peers.map.get(id).map(|e| e.peer.clone()))
            .collect();
        // Entries not (yet) in the order list.
        for (id, e) in &peers.map {
            if !peers.order.contains(id) {
                list.push(e.peer.clone());
            }
        }
        RoomState {
            room_id: self.room_id,
            code: self.code.clone(),
            role: self.role,
            auth_mode: self.auth_mode,
            kind: RoomKind::Game,
            sig: Some(self.sig.clone()),
            peers: list,
            expires_at: self.expires_at.clone(),
            link: join_link(&self.code),
            password_protected: self.password_protected,
            local_addr: self.local_addr.map(|a| a.to_string()),
        }
    }

    /// Queues `line` for the given peers (all when `only` is `None`), skipping
    /// `except`. Peers whose queue is full are disconnected.
    fn send_lines(&self, line: &str, only: Option<&str>, except: Option<&str>) {
        let mut slow = Vec::new();
        for (id, e) in &self.peers().map {
            if Some(id.as_str()) == except || only.is_some_and(|o| o != id) {
                continue;
            }
            if let Some(tx) = &e.tx {
                if let Err(mpsc::error::TrySendError::Full(_)) = tx.try_send(line.to_owned()) {
                    if let Some(c) = &e.conn {
                        slow.push(c.clone());
                    }
                }
            }
        }
        for c in slow {
            c.close(3u32.into(), reasons::SLOW.as_bytes());
        }
    }

    fn send_to(&self, peer_id: &str, line: &str) {
        self.send_lines(line, Some(peer_id), None);
    }

    fn broadcast(&self, line: &str, except: Option<&str>) {
        self.send_lines(line, None, except);
    }

    fn connections(&self, except: Option<&str>) -> Vec<Connection> {
        self.peers()
            .map
            .iter()
            .filter(|(id, _)| Some(id.as_str()) != except)
            .filter_map(|(_, e)| e.conn.clone())
            .collect()
    }
}

pub(crate) struct Inner {
    cfg: P2pConfig,
    downloader: Arc<dyn Downloader>,
    settings: Arc<dyn SettingsStore>,
    env: Arc<dyn RoomEnv>,
    observer: Arc<dyn RoomObserver>,
    /// Serialises create / join / leave.
    op: tokio::sync::Mutex<()>,
    session: StdMutex<Option<Arc<Session>>>,
    /// Last LAN port seen in a game log or a local LAN announcement. Only
    /// consulted by `create`.
    lan_port: Arc<watch::Sender<Option<LanSeen>>>,
    /// Endpoint key for the app's lifetime, so a host's ban of this launcher
    /// survives a rejoin.
    secret: SecretKey,
}

impl Inner {
    pub(crate) fn current(&self) -> Option<Arc<Session>> {
        lock(&self.session).clone()
    }

    fn set_current(&self, s: Option<Arc<Session>>) {
        *lock(&self.session) = s;
    }

    /// Takes the session out if it is still `s`.
    fn take_if(&self, s: &Arc<Session>) -> bool {
        let mut guard = lock(&self.session);
        if guard.as_ref().is_some_and(|cur| Arc::ptr_eq(cur, s)) {
            *guard = None;
            true
        } else {
            false
        }
    }
}

/// Rooms over iroh. Cheap to clone.
#[derive(Clone)]
pub struct P2pRooms {
    inner: Arc<Inner>,
}

impl P2pRooms {
    pub fn new(
        cfg: P2pConfig,
        downloader: Arc<dyn Downloader>,
        settings: Arc<dyn SettingsStore>,
        env: Arc<dyn RoomEnv>,
        observer: Arc<dyn RoomObserver>,
    ) -> Self {
        let (lan_port, _) = watch::channel(None);
        Self {
            inner: Arc::new(Inner {
                cfg,
                downloader,
                settings,
                env,
                observer,
                op: tokio::sync::Mutex::new(()),
                session: StdMutex::new(None),
                lan_port: Arc::new(lan_port),
                secret: SecretKey::generate(),
            }),
        }
    }

    pub(crate) fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }

    /// Feed log lines of a running game; picks up the integrated server's
    /// LAN port (server-thread lines only, chat is ignored). The port is
    /// used only by a later `create` for the same instance.
    pub fn feed_game_log(&self, instance: InstanceId, line: &str) {
        if let Some(port) = lan::port_from_log_line(line) {
            self.inner.lan_port.send_replace(Some(LanSeen {
                instance: Some(instance),
                port,
            }));
        }
    }

    /// Sets the host's LAN port explicitly (tests, manual override).
    pub fn set_lan_port(&self, port: Option<u16>) {
        self.inner.lan_port.send_replace(port.map(|port| LanSeen {
            instance: None,
            port,
        }));
    }

    /// Address of the local iroh endpoint while in a room (diagnostics, tests).
    pub fn endpoint_addr(&self) -> Option<EndpointAddr> {
        self.inner.current().map(|s| s.endpoint.addr())
    }

    async fn hub(&self) -> AppResult<HubClient> {
        let settings = self.inner.settings.get().await?;
        HubClient::new(
            self.inner.downloader.client(Route::Direct),
            &settings.hub_url,
        )
    }

    fn emit_state(&self) {
        self.inner
            .observer
            .on_state(self.inner.current().map(|s| s.state()));
    }

    async fn bind_endpoint(
        &self,
        secret: SecretKey,
        relays: &[RelayUrl],
        accept: bool,
    ) -> AppResult<Endpoint> {
        let mode = if relays.is_empty() {
            RelayMode::Disabled
        } else {
            RelayMode::custom(relays.iter().cloned())
        };
        let mut b = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .relay_mode(mode);
        if accept {
            b = b.alpns(vec![ALPN.to_vec()]);
        }
        if let Some(tweak) = &self.inner.cfg.endpoint_tweak {
            b = tweak(b);
        }
        b.bind()
            .await
            .map_err(|e| net_err("cannot start p2p endpoint", e))
    }

    fn relays(&self, from_hub: Vec<String>) -> Vec<RelayUrl> {
        self.inner
            .cfg
            .relay_override
            .clone()
            .unwrap_or(from_hub)
            .iter()
            .filter_map(|u| u.parse::<RelayUrl>().ok())
            .collect()
    }

    /// Waits until a verified LAN port of `instance` (or of an unknown local
    /// game, from a LAN announcement) is known.
    async fn wait_lan_port(&self, instance: InstanceId) -> AppResult<u16> {
        let mut rx = self.inner.lan_port.subscribe();
        let deadline = tokio::time::Instant::now() + self.inner.cfg.lan_wait;
        loop {
            let cur = *rx.borrow_and_update();
            if let Some(seen) = cur.filter(|s| s.instance.is_none_or(|i| i == instance)) {
                if lan::is_local_port_open(seen.port).await {
                    return Ok(seen.port);
                }
                // Stale port from an earlier game session.
                self.inner.lan_port.send_if_modified(|v| {
                    let stale = *v == Some(seen);
                    if stale {
                        *v = None;
                    }
                    stale
                });
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => {
                    return Err(AppError::new(
                        ErrorCode::NotFound,
                        "no LAN world detected; open the world to LAN first",
                    )
                    .detail("hint", "openToLan"));
                }
            }
        }
    }

    // ---------------------------------------------------------------- host

    async fn create_room(
        &self,
        instance: InstanceId,
        auth_mode: RoomAuthMode,
        password: Option<String>,
    ) -> AppResult<RoomState> {
        let _op = self.inner.op.lock().await;
        if self.inner.current().is_some() {
            return Err(AppError::new(ErrorCode::Conflict, "already in a room"));
        }
        let password = password.filter(|p| !p.is_empty()).map(Secret::new);
        let hub = self.hub().await?;
        let me = self.inner.env.identity().await?;
        check_host_auth_mode(auth_mode, &me)?;
        let sig = self.inner.env.instance_sig(instance).await?;

        let cancel = CancellationToken::new();
        if self.inner.cfg.lan_multicast {
            tokio::spawn(lan::listen_announcements(
                self.inner.lan_port.clone(),
                cancel.clone(),
            ));
        }
        let detected = self.wait_lan_port(instance).await;
        // Detection is over: the port is fixed for the room's lifetime.
        cancel.cancel();
        let lan_port = detected?;
        tracing::info!("rooms: hosting LAN world on port {lan_port}");

        let secret = self.inner.secret.clone();
        let my_id = secret.public().to_string();
        let req = CreateRoomReq {
            endpoint_id: my_id.clone(),
            sig: sig.clone(),
            auth_mode,
            max_peers: self.inner.cfg.max_peers,
            password_protected: password.is_some(),
        };
        let resp = hub.create_room(&req, me.hub_token.as_ref()).await?;
        let host_token = Secret::new(resp.host_token);
        let relays = self.relays(resp.relay_urls);
        let endpoint = match self.bind_endpoint(secret, &relays, true).await {
            Ok(ep) => ep,
            Err(e) => {
                let _ = hub.close_room(resp.room_id, &host_token).await;
                return Err(e);
            }
        };

        let (dgram_tx, dgram_rx) = mpsc::channel(DGRAM_QUEUE);
        let session = Arc::new(Session {
            role: RoomRole::Host,
            room_id: resp.room_id,
            code: resp.code,
            auth_mode,
            sig,
            expires_at: resp.expires_at,
            password_protected: password.is_some(),
            local_addr: None,
            my_id: my_id.clone(),
            my_name: clean_name(&me.name),
            endpoint,
            cancel: CancellationToken::new(),
            peers: StdMutex::new(Peers {
                order: vec![my_id],
                map: BTreeMap::new(),
            }),
            dgram_tx,
            dgram_rx: tokio::sync::Mutex::new(dgram_rx),
            host: Some(HostCtx {
                hub,
                host_token,
                password,
                lan_port,
                banned: StdMutex::new(HashSet::new()),
                pw_fails: StdMutex::new(HashMap::new()),
            }),
        });
        self.inner.set_current(Some(session.clone()));
        tokio::spawn(self.clone().host_accept_loop(session.clone()));
        let state = session.state();
        self.inner.observer.on_state(Some(state.clone()));
        Ok(state)
    }

    async fn host_accept_loop(self, s: Arc<Session>) {
        loop {
            let incoming = tokio::select! {
                () = s.cancel.cancelled() => return,
                i = s.endpoint.accept() => match i {
                    Some(i) => i,
                    None => return,
                },
            };
            let this = self.clone();
            let s2 = s.clone();
            tokio::spawn(async move {
                if let Err(e) = this.host_handle(s2, incoming).await {
                    tracing::debug!("rooms: peer handshake failed: {}", e.message);
                }
            });
        }
    }

    async fn host_handle(&self, s: Arc<Session>, incoming: Incoming) -> AppResult<()> {
        let Some(host) = s.host.as_ref() else {
            return Ok(());
        };
        let hello_timeout = self.inner.cfg.hello_timeout;
        let conn = incoming
            .accept()
            .map_err(|e| net_err("incoming", e))?
            .await
            .map_err(|e| net_err("incoming", e))?;
        let remote = conn.remote_id().to_string();
        let (mut send, recv) = tokio::time::timeout(hello_timeout, conn.accept_bi())
            .await
            .map_err(|_| timeout_err("hello"))?
            .map_err(|e| net_err("control stream", e))?;
        let mut reader = BufReader::new(recv);
        let line = tokio::time::timeout(hello_timeout, wire::read_line(&mut reader, MAX_LINE))
            .await
            .map_err(|_| timeout_err("hello"))??
            .ok_or_else(|| wire::proto_err("no hello"))?;
        let ControlMsg::Hello {
            protocol,
            name,
            uuid,
            peer_token,
            password_proof,
        } = wire::decode(&line)?
        else {
            return reject(&conn, &mut send, reasons::PROTOCOL).await;
        };
        if protocol != PROTOCOL_VERSION {
            return reject(&conn, &mut send, reasons::PROTOCOL).await;
        }
        let Some(token_uuid) = wire::verify_peer_token(
            &peer_token,
            host.host_token.expose(),
            s.room_id,
            &remote,
            unix_now(),
        ) else {
            return reject(&conn, &mut send, reasons::BAD_TOKEN).await;
        };
        // Ban identity: the hub-verified uuid from the token (never Hello's),
        // only in yggdrasil rooms where it means something.
        let uuid_key = token_uuid
            .as_deref()
            .filter(|_| s.auth_mode == RoomAuthMode::Yggdrasil)
            .map(normalize_uuid);
        let uuid = match (&uuid_key, token_uuid) {
            (Some(_), Some(verified)) => Some(verified),
            _ => uuid,
        };
        let banned = {
            let b = lock(&host.banned);
            b.contains(&remote) || uuid_key.as_ref().is_some_and(|u| b.contains(u))
        };
        if banned {
            return reject(&conn, &mut send, reasons::BANNED).await;
        }
        let max_guests = usize::try_from(self.inner.cfg.max_peers.saturating_sub(1)).unwrap_or(0);
        let full = {
            let p = s.peers();
            !p.map.contains_key(&remote) && p.map.len() >= max_guests
        };
        if full {
            return reject(&conn, &mut send, reasons::ROOM_FULL).await;
        }
        if let Some(password) = host.password.as_ref() {
            if password_locked(host, &remote) {
                return reject(&conn, &mut send, reasons::RATE_LIMITED).await;
            }
            let ekm = ekm(&conn, s.room_id)?;
            let ok = password_proof
                .as_deref()
                .is_some_and(|p| wire::verify_password_proof(password.expose(), &ekm, p));
            if !ok {
                note_password_failure(host, &remote);
                return reject(&conn, &mut send, reasons::BAD_PASSWORD).await;
            }
            lock(&host.pw_fails).remove(&remote);
        }

        // Accepted: introduce ourselves, then everyone gets a fresh Welcome.
        let host_hello = wire::encode(&ControlMsg::Hello {
            protocol: PROTOCOL_VERSION,
            name: s.my_name.clone(),
            uuid: None,
            peer_token: String::new(),
            password_proof: None,
        })?;
        send.write_all(host_hello.as_bytes())
            .await
            .map_err(|e| net_err("control stream", e))?;
        let (tx, rx) = mpsc::channel(CONTROL_QUEUE);
        spawn_writer(send, rx);
        let peer = Peer {
            peer_id: remote.clone(),
            name: clean_name(&name),
            uuid,
            path: PathKind::Connecting,
            rtt_ms: None,
            speaking: false,
            muted: false,
        };
        let replaced = {
            let mut p = s.peers();
            let old = p.map.insert(
                remote.clone(),
                PeerEntry {
                    peer: peer.clone(),
                    conn: Some(conn.clone()),
                    tx: Some(tx),
                    ban_uuid: uuid_key,
                },
            );
            if !p.order.contains(&remote) {
                p.order.push(remote.clone());
            }
            old
        };
        if let Some(old) = replaced.and_then(|e| e.conn) {
            old.close(0u32.into(), b"replaced");
        }
        self.host_broadcast_welcome(&s)?;
        self.inner.observer.on_peer(peer, true);
        self.emit_state();

        tokio::spawn(
            self.clone()
                .monitor_path(s.clone(), remote.clone(), conn.clone()),
        );
        tokio::spawn(
            self.clone()
                .datagram_loop(s.clone(), remote.clone(), conn.clone()),
        );
        tokio::spawn(host_game_streams(
            conn.clone(),
            host.lan_port,
            s.cancel.clone(),
        ));
        let control = self.clone().control_loop(s.clone(), remote.clone(), reader);
        tokio::select! {
            () = s.cancel.cancelled() => {}
            () = control => { conn.close(0u32.into(), b"bye"); }
            _ = conn.closed() => {}
        }
        self.host_remove_peer(&s, &remote, &conn)?;
        Ok(())
    }

    fn host_broadcast_welcome(&self, s: &Session) -> AppResult<()> {
        let (peers, peer_names) = {
            let p = s.peers();
            let names = p
                .order
                .iter()
                .map(|id| {
                    if *id == s.my_id {
                        s.my_name.clone()
                    } else {
                        p.map
                            .get(id)
                            .map(|e| e.peer.name.clone())
                            .unwrap_or_default()
                    }
                })
                .collect();
            (p.order.clone(), names)
        };
        let line = wire::encode(&ControlMsg::Welcome { peers, peer_names })?;
        s.broadcast(&line, None);
        Ok(())
    }

    fn host_remove_peer(&self, s: &Arc<Session>, id: &str, conn: &Connection) -> AppResult<()> {
        if s.cancel.is_cancelled() {
            return Ok(());
        }
        let removed = {
            let mut p = s.peers();
            let same = p
                .map
                .get(id)
                .and_then(|e| e.conn.as_ref())
                .is_some_and(|c| c.stable_id() == conn.stable_id());
            if same {
                p.order.retain(|x| x != id);
                p.map.remove(id)
            } else {
                None
            }
        };
        if let Some(e) = removed {
            self.host_broadcast_welcome(s)?;
            self.inner.observer.on_peer(e.peer, false);
            self.emit_state();
        }
        Ok(())
    }

    async fn kick_peer(&self, peer_id: &str) -> AppResult<()> {
        let s = self.require_session()?;
        let Some(host) = s.host.as_ref().filter(|_| s.role == RoomRole::Host) else {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "only the host can kick",
            ));
        };
        let line = wire::encode(&ControlMsg::Kick {
            reason: reasons::KICKED.into(),
        })?;
        let entry = {
            let mut p = s.peers();
            p.order.retain(|x| x != peer_id);
            p.map.remove(peer_id)
        };
        let Some(entry) = entry else {
            return Err(
                AppError::new(ErrorCode::NotFound, "no such peer").detail("peerId", peer_id)
            );
        };
        // Banned for the room's lifetime: endpoint id, plus the profile uuid
        // in yggdrasil rooms (a new endpoint id is free to generate).
        {
            let mut b = lock(&host.banned);
            b.insert(peer_id.to_owned());
            if let Some(u) = &entry.ban_uuid {
                b.insert(u.clone());
            }
        }
        if let Some(tx) = &entry.tx {
            let _ = tx.try_send(line);
        }
        if let Some(conn) = entry.conn.clone() {
            tokio::spawn(async move {
                // Give the Kick line a moment to arrive before closing.
                let _ = tokio::time::timeout(Duration::from_millis(500), conn.closed()).await;
                conn.close(2u32.into(), reasons::KICKED.as_bytes());
            });
        }
        self.host_broadcast_welcome(&s)?;
        self.inner.observer.on_peer(entry.peer, false);
        self.emit_state();
        Ok(())
    }

    // --------------------------------------------------------------- guest

    async fn join_room(
        &self,
        code: &str,
        instance: InstanceId,
        password: Option<String>,
    ) -> AppResult<RoomState> {
        let code = parse_code(code)?;
        let _op = self.inner.op.lock().await;
        if self.inner.current().is_some() {
            return Err(AppError::new(ErrorCode::Conflict, "already in a room"));
        }
        let password = password.filter(|p| !p.is_empty()).map(Secret::new);
        let hub = self.hub().await?;
        let me = self.inner.env.identity().await?;
        let local_sig = self.inner.env.instance_sig(instance).await?;

        let secret = self.inner.secret.clone();
        let my_id = secret.public().to_string();
        let resp = hub
            .join_room(
                &code,
                &my_id,
                &clean_name(&me.name),
                me.uuid.as_deref(),
                me.hub_token.as_ref(),
            )
            .await
            .map_err(|e| {
                if e.code == ErrorCode::AuthRequired && me.hub_token.is_none() {
                    e.detail("reason", "yggdrasilAccountRequired")
                } else {
                    e
                }
            })?;
        if resp.auth_mode == RoomAuthMode::Yggdrasil && me.hub_token.is_none() {
            return Err(AppError::new(
                ErrorCode::AuthRequired,
                "this room needs an account on the project's Yggdrasil server",
            )
            .detail("reason", "yggdrasilAccountRequired"));
        }
        check_sig(&resp.sig, &local_sig)?;
        if resp.password_protected && password.is_none() {
            return Err(
                AppError::new(ErrorCode::InvalidInput, "room password required")
                    .detail("reason", "passwordRequired"),
            );
        }
        let host_id: EndpointId = resp
            .host_endpoint_id
            .parse()
            .map_err(|e| net_err("invalid host endpoint id", e))?;
        let relays = self.relays(resp.relay_urls);
        let endpoint = self.bind_endpoint(secret, &relays, false).await?;

        let handshake = self
            .guest_connect(
                &endpoint,
                host_id,
                &relays,
                resp.room_id,
                &me,
                &resp.peer_token,
                password.as_ref(),
            )
            .await;
        let (conn, send, reader, host_name, welcome) = match handshake {
            Ok(v) => v,
            Err(e) => {
                endpoint.close().await;
                return Err(e);
            }
        };

        let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await {
            Ok(l) => l,
            Err(e) => {
                conn.close(0u32.into(), b"bye");
                endpoint.close().await;
                return Err(e.into());
            }
        };
        let local_addr = listener.local_addr()?;

        let host_key = host_id.to_string();
        let (tx, rx) = mpsc::channel(CONTROL_QUEUE);
        spawn_writer(send, rx);
        let (dgram_tx, dgram_rx) = mpsc::channel(DGRAM_QUEUE);
        let mut peers = Peers::default();
        peers.map.insert(
            host_key.clone(),
            PeerEntry {
                peer: Peer {
                    peer_id: host_key.clone(),
                    name: host_name.clone(),
                    uuid: None,
                    path: PathKind::Connecting,
                    rtt_ms: None,
                    speaking: false,
                    muted: false,
                },
                conn: Some(conn.clone()),
                tx: Some(tx),
                ban_uuid: None,
            },
        );
        apply_welcome(&mut peers, welcome.0, welcome.1, &my_id, &host_key);
        let session = Arc::new(Session {
            role: RoomRole::Guest,
            room_id: resp.room_id,
            code,
            auth_mode: resp.auth_mode,
            sig: resp.sig,
            expires_at: String::new(),
            password_protected: resp.password_protected,
            local_addr: Some(local_addr),
            my_id,
            my_name: clean_name(&me.name),
            endpoint,
            cancel: CancellationToken::new(),
            peers: StdMutex::new(peers),
            dgram_tx,
            dgram_rx: tokio::sync::Mutex::new(dgram_rx),
            host: None,
        });
        self.inner.set_current(Some(session.clone()));

        tokio::spawn(guest_tunnel(listener, conn.clone(), session.cancel.clone()));
        if self.inner.cfg.lan_multicast {
            tokio::spawn(lan::announce(
                format!("{host_name} (MineUP)"),
                local_addr.port(),
                session.cancel.clone(),
            ));
        }
        tokio::spawn(
            self.clone()
                .monitor_path(session.clone(), host_key.clone(), conn.clone()),
        );
        tokio::spawn(
            self.clone()
                .datagram_loop(session.clone(), host_key.clone(), conn.clone()),
        );
        tokio::spawn(
            self.clone()
                .guest_lifetime(session.clone(), host_key, conn, reader),
        );

        let state = session.state();
        self.inner.observer.on_state(Some(state.clone()));
        Ok(state)
    }

    /// Connects, sends `Hello` and waits for the host's `Hello` + `Welcome`.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    async fn guest_connect(
        &self,
        endpoint: &Endpoint,
        host_id: EndpointId,
        relays: &[RelayUrl],
        room_id: RoomId,
        me: &Identity,
        peer_token: &str,
        password: Option<&Secret>,
    ) -> AppResult<(
        Connection,
        SendStream,
        BufReader<RecvStream>,
        String,
        (Vec<String>, Vec<String>),
    )> {
        let mut addr = EndpointAddr::new(host_id);
        for r in relays {
            addr = addr.with_relay_url(r.clone());
        }
        let conn =
            tokio::time::timeout(self.inner.cfg.connect_timeout, endpoint.connect(addr, ALPN))
                .await
                .map_err(|_| {
                    AppError::new(ErrorCode::Network, "host is unreachable")
                        .detail("reason", "timeout")
                })?
                .map_err(|e| net_err("host is unreachable", e))?;
        let (mut send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| net_err("control stream", e))?;
        let password_proof = match password {
            Some(p) => Some(wire::password_proof(p.expose(), &ekm(&conn, room_id)?)),
            None => None,
        };
        let hello = wire::encode(&ControlMsg::Hello {
            protocol: PROTOCOL_VERSION,
            name: clean_name(&me.name),
            uuid: me.uuid.clone(),
            peer_token: peer_token.to_owned(),
            password_proof,
        })?;
        send.write_all(hello.as_bytes())
            .await
            .map_err(|e| net_err("control stream", e))?;
        let mut reader = BufReader::new(recv);
        let mut host_name = String::from("Host");
        let handshake = async {
            loop {
                let line = wire::read_line(&mut reader, MAX_LINE).await?;
                let Some(line) = line else {
                    return Err(AppError::new(
                        ErrorCode::Network,
                        "host closed the connection",
                    ));
                };
                match wire::decode(&line)? {
                    ControlMsg::Hello { name, .. } => host_name = clean_name(&name),
                    ControlMsg::Welcome { peers, peer_names } => return Ok((peers, peer_names)),
                    ControlMsg::Kick { reason } => return Err(kick_error(&reason)),
                    _ => {}
                }
            }
        };
        let welcome = match tokio::time::timeout(self.inner.cfg.hello_timeout, handshake).await {
            Ok(Ok(w)) => w,
            Ok(Err(e)) => {
                conn.close(0u32.into(), b"bye");
                return Err(e);
            }
            Err(_) => {
                conn.close(0u32.into(), b"bye");
                return Err(timeout_err("welcome"));
            }
        };
        Ok((conn, send, reader, host_name, welcome))
    }

    async fn guest_lifetime(
        self,
        s: Arc<Session>,
        host_key: String,
        conn: Connection,
        reader: BufReader<RecvStream>,
    ) {
        let control = self.clone().control_loop(s.clone(), host_key, reader);
        tokio::select! {
            () = s.cancel.cancelled() => return,
            () = control => {}
            _ = conn.closed() => {}
        }
        // Host left, closed the room or kicked us.
        if self.inner.take_if(&s) {
            shutdown(&s).await;
            self.inner.observer.on_state(None);
        }
    }

    // -------------------------------------------------------------- shared

    /// Reads control messages from one peer until EOF / error / Bye.
    async fn control_loop(self, s: Arc<Session>, from: String, mut reader: BufReader<RecvStream>) {
        loop {
            let line = match wire::read_line(&mut reader, MAX_LINE).await {
                Ok(Some(l)) => l,
                Ok(None) | Err(_) => return,
            };
            let Ok(msg) = wire::decode(&line) else {
                continue;
            };
            match msg {
                ControlMsg::Ping { nonce } => {
                    if let Ok(l) = wire::encode(&ControlMsg::Pong { nonce }) {
                        s.send_to(&from, &l);
                    }
                }
                ControlMsg::Chat {
                    id,
                    text,
                    sent_at,
                    from: orig,
                    from_name,
                } => {
                    self.on_chat(&s, &from, (orig, from_name), id, text, sent_at);
                }
                ControlMsg::VoiceState {
                    speaking, muted, ..
                } => {
                    if let Some(e) = s.peers().map.get_mut(&from) {
                        e.peer.speaking = speaking;
                        e.peer.muted = muted;
                    }
                    self.emit_state();
                }
                ControlMsg::Welcome { peers, peer_names } if s.role == RoomRole::Guest => {
                    apply_welcome(&mut s.peers(), peers, peer_names, &s.my_id, &from);
                    self.emit_state();
                }
                ControlMsg::Hello { name, .. } if s.role == RoomRole::Guest => {
                    if let Some(e) = s.peers().map.get_mut(&from) {
                        e.peer.name = clean_name(&name);
                    }
                    self.emit_state();
                }
                ControlMsg::Kick { .. } | ControlMsg::Bye => return,
                _ => {}
            }
        }
    }

    fn on_chat(
        &self,
        s: &Session,
        from: &str,
        (orig, orig_name): (Option<String>, Option<String>),
        id: String,
        text: String,
        sent_at: String,
    ) {
        let text: String = text.chars().take(MAX_CHAT_CHARS).collect();
        let conn_name = s
            .peers()
            .map
            .get(from)
            .map(|e| e.peer.name.clone())
            .unwrap_or_default();
        let (from_peer, from_name) = match (s.role, orig) {
            // Guests only talk to the host; it names the original sender.
            (RoomRole::Guest, Some(f)) if f != from => (f, orig_name.unwrap_or_default()),
            // The host trusts only the connection identity.
            _ => (from.to_owned(), conn_name),
        };
        if s.role == RoomRole::Host {
            let fwd = wire::encode(&ControlMsg::Chat {
                id: id.clone(),
                text: text.clone(),
                sent_at: sent_at.clone(),
                from: Some(from_peer.clone()),
                from_name: Some(from_name.clone()),
            });
            if let Ok(line) = fwd {
                s.broadcast(&line, Some(from));
            }
        }
        self.inner.observer.on_chat(ChatMessage {
            id,
            from_peer,
            from_name,
            text,
            sent_at,
            via_relay: false,
        });
    }

    async fn datagram_loop(self, s: Arc<Session>, from: String, conn: Connection) {
        loop {
            let data = tokio::select! {
                () = s.cancel.cancelled() => return,
                d = conn.read_datagram() => match d {
                    Ok(d) => d,
                    Err(_) => return,
                },
            };
            if s.role == RoomRole::Host {
                for other in s.connections(Some(&from)) {
                    let _ = other.send_datagram(data.clone());
                }
            }
            // Voice is lossy anyway: drop when the consumer lags.
            let _ = s.dgram_tx.try_send((from.clone(), data.to_vec()));
        }
    }

    async fn monitor_path(self, s: Arc<Session>, peer_id: String, conn: Connection) {
        loop {
            let (kind, rtt) = path_info(&conn);
            let changed = {
                let mut p = s.peers();
                let mut changed = false;
                let guest = s.role == RoomRole::Guest;
                for (id, e) in p.map.iter_mut() {
                    // Guests reach everyone through the host connection.
                    let mirrors = guest && e.conn.is_none();
                    if (*id == peer_id || mirrors) && significant(&e.peer, kind, rtt) {
                        e.peer.path = kind;
                        e.peer.rtt_ms = rtt;
                        changed = true;
                    }
                }
                changed
            };
            if changed {
                self.emit_state();
            }
            tokio::select! {
                () = s.cancel.cancelled() => return,
                _ = conn.closed() => return,
                () = tokio::time::sleep(PATH_POLL) => {}
            }
        }
    }

    fn require_session(&self) -> AppResult<Arc<Session>> {
        self.inner
            .current()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "not in a room"))
    }

    async fn leave_room(&self) -> AppResult<()> {
        let _op = self.inner.op.lock().await;
        let Some(s) = self.inner.current() else {
            return Ok(());
        };
        if self.inner.take_if(&s) {
            shutdown(&s).await;
            self.inner.observer.on_state(None);
        }
        Ok(())
    }

    async fn chat(&self, text: &str) -> AppResult<ChatMessage> {
        let s = self.require_session()?;
        let text = text.trim();
        if text.is_empty() || text.chars().count() > MAX_CHAT_CHARS {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "chat message is empty or too long",
            )
            .detail("max", MAX_CHAT_CHARS.to_string()));
        }
        let msg = ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            from_peer: s.my_id.clone(),
            from_name: s.my_name.clone(),
            text: text.to_owned(),
            sent_at: chrono::Utc::now().to_rfc3339(),
            via_relay: false,
        };
        let line = wire::encode(&ControlMsg::Chat {
            id: msg.id.clone(),
            text: msg.text.clone(),
            sent_at: msg.sent_at.clone(),
            from: Some(msg.from_peer.clone()),
            from_name: Some(msg.from_name.clone()),
        })?;
        s.broadcast(&line, None);
        Ok(msg)
    }

    fn datagram_limit(s: &Session) -> usize {
        s.connections(None)
            .iter()
            .filter_map(Connection::max_datagram_size)
            .min()
            .unwrap_or(DEFAULT_DATAGRAM)
    }
}

#[async_trait]
impl RoomService for P2pRooms {
    async fn create(
        &self,
        instance: InstanceId,
        auth_mode: RoomAuthMode,
        password: Option<String>,
    ) -> AppResult<RoomState> {
        self.create_room(instance, auth_mode, password).await
    }

    async fn join(
        &self,
        code: &str,
        instance: Option<InstanceId>,
        password: Option<String>,
    ) -> AppResult<RoomState> {
        // TODO(net, wave 4): voice rooms join without an instance.
        let instance = instance.ok_or_else(|| {
            AppError::new(
                ErrorCode::InvalidInput,
                "instance is required for game rooms",
            )
        })?;
        self.join_room(code, instance, password).await
    }

    async fn create_voice(&self, _password: Option<String>) -> AppResult<RoomState> {
        Err(AppError::not_implemented("RoomService::create_voice"))
    }

    async fn leave(&self) -> AppResult<()> {
        self.leave_room().await
    }

    async fn state(&self) -> AppResult<Option<RoomState>> {
        Ok(self.inner.current().map(|s| s.state()))
    }

    async fn kick(&self, peer_id: &str) -> AppResult<()> {
        self.kick_peer(peer_id).await
    }

    async fn send_chat(&self, text: &str) -> AppResult<ChatMessage> {
        self.chat(text).await
    }
}

#[async_trait]
impl DatagramTransport for P2pRooms {
    async fn send(&self, to: Option<&str>, payload: &[u8]) -> AppResult<()> {
        let s = self.require_session()?;
        let limit = Self::datagram_limit(&s);
        if payload.len() > limit {
            return Err(AppError::new(ErrorCode::InvalidInput, "datagram too large")
                .detail("max", limit.to_string()));
        }
        match to {
            Some(id) => {
                let conn = s.peers().map.get(id).and_then(|e| e.conn.clone());
                let conn = conn.ok_or_else(|| {
                    AppError::new(ErrorCode::NotFound, "peer is not directly connected")
                        .detail("peerId", id)
                })?;
                conn.send_datagram(payload.to_vec().into())
                    .map_err(|e| net_err("datagram", e))
            }
            None => {
                for conn in s.connections(None) {
                    let _ = conn.send_datagram(payload.to_vec().into());
                }
                Ok(())
            }
        }
    }

    async fn recv(&self) -> Option<(String, Vec<u8>)> {
        let s = self.inner.current()?;
        let mut rx = s.dgram_rx.lock().await;
        tokio::select! {
            () = s.cancel.cancelled() => None,
            m = rx.recv() => m,
        }
    }

    fn max_datagram_size(&self) -> usize {
        self.inner
            .current()
            .map(|s| Self::datagram_limit(&s))
            .unwrap_or(DEFAULT_DATAGRAM)
    }
}

// ------------------------------------------------------------------ helpers

/// The integrated server's online mode follows the host account: a Yggdrasil
/// host runs online mode (offline guests get "Invalid session"), an offline
/// host runs offline mode (nobody is verified).
fn check_host_auth_mode(mode: RoomAuthMode, me: &Identity) -> AppResult<()> {
    let online_host = me.hub_token.is_some();
    match (mode, online_host) {
        (RoomAuthMode::Yggdrasil, false) => Err(AppError::new(
            ErrorCode::InvalidInput,
            "a yggdrasil room needs the host to play with a Yggdrasil account",
        )
        .detail("reason", "authModeMismatch")
        .detail("expected", "open")),
        (RoomAuthMode::Open, true) => Err(AppError::new(
            ErrorCode::InvalidInput,
            "the host plays with a Yggdrasil account, so the world runs in online mode; \
             offline guests would fail with \"Invalid session\"",
        )
        .detail("reason", "authModeMismatch")
        .detail("expected", "yggdrasil")),
        _ => Ok(()),
    }
}

fn password_locked(host: &HostCtx, remote: &str) -> bool {
    let fails = lock(&host.pw_fails);
    let Some((count, last)) = fails.get(remote) else {
        return false;
    };
    if *count < MAX_PASSWORD_FAILS {
        return false;
    }
    let exp = (*count - MAX_PASSWORD_FAILS).min(7);
    let lockout = PASSWORD_BACKOFF
        .saturating_mul(1u32 << exp)
        .min(PASSWORD_BACKOFF_MAX);
    last.elapsed() < lockout
}

fn note_password_failure(host: &HostCtx, remote: &str) {
    let mut fails = lock(&host.pw_fails);
    let e = fails
        .entry(remote.to_owned())
        .or_insert((0, Instant::now()));
    e.0 = e.0.saturating_add(1);
    e.1 = Instant::now();
}

fn normalize_uuid(u: &str) -> String {
    let compact: String = u
        .chars()
        .filter(|c| *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    format!("uuid:{compact}")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn shutdown(s: &Session) {
    s.cancel.cancel();
    let conns = {
        let mut p = s.peers();
        let conns: Vec<Connection> = p.map.values().filter_map(|e| e.conn.clone()).collect();
        p.map.clear();
        p.order.clear();
        conns
    };
    for c in conns {
        c.close(1u32.into(), reasons::CLOSED.as_bytes());
    }
    if let Some(h) = &s.host {
        let close = h.hub.close_room(s.room_id, &h.host_token);
        if let Ok(Err(e)) = tokio::time::timeout(Duration::from_secs(5), close).await {
            tracing::debug!("rooms: hub close failed: {}", e.message);
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(3), s.endpoint.close()).await;
}

async fn reject(conn: &Connection, send: &mut SendStream, reason: &str) -> AppResult<()> {
    if let Ok(line) = wire::encode(&ControlMsg::Kick {
        reason: reason.to_owned(),
    }) {
        let _ = send.write_all(line.as_bytes()).await;
        let _ = send.finish();
    }
    // Let the guest read the reason and close first.
    let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
    conn.close(1u32.into(), reason.as_bytes());
    Err(AppError::new(ErrorCode::PermissionDenied, "peer rejected").detail("reason", reason))
}

fn kick_error(reason: &str) -> AppError {
    let (code, msg) = match reason {
        reasons::BAD_PASSWORD => (ErrorCode::PermissionDenied, "wrong room password"),
        reasons::RATE_LIMITED => (ErrorCode::RateLimited, "too many wrong passwords"),
        reasons::BAD_TOKEN => (ErrorCode::PermissionDenied, "room ticket rejected by host"),
        reasons::BANNED => (
            ErrorCode::PermissionDenied,
            "you were kicked from this room",
        ),
        reasons::ROOM_FULL => (ErrorCode::Conflict, "room is full"),
        reasons::PROTOCOL => (ErrorCode::Conflict, "incompatible launcher version"),
        _ => (ErrorCode::PermissionDenied, "rejected by host"),
    };
    AppError::new(code, msg).detail("reason", reason)
}

fn check_sig(remote: &InstanceSig, local: &InstanceSig) -> AppResult<()> {
    if remote == local {
        return Ok(());
    }
    let mut e = AppError::new(ErrorCode::Conflict, "instance does not match the room");
    if remote.mc != local.mc {
        e = e
            .detail("expectedMc", &remote.mc)
            .detail("actualMc", &local.mc);
    }
    if remote.loader != local.loader || remote.loader_version != local.loader_version {
        e = e
            .detail(
                "expectedLoader",
                format!(
                    "{:?} {}",
                    remote.loader,
                    remote.loader_version.as_deref().unwrap_or("")
                ),
            )
            .detail(
                "actualLoader",
                format!(
                    "{:?} {}",
                    local.loader,
                    local.loader_version.as_deref().unwrap_or("")
                ),
            );
    }
    if remote.mods_hash != local.mods_hash {
        e = e.detail("mods", "mismatch");
    }
    Err(e)
}

fn apply_welcome(
    p: &mut Peers,
    order: Vec<String>,
    names: Vec<String>,
    my_id: &str,
    host_key: &str,
) {
    p.map
        .retain(|id, e| id == host_key || e.conn.is_some() || order.contains(id));
    let (path, rtt) = p
        .map
        .get(host_key)
        .map(|e| (e.peer.path, e.peer.rtt_ms))
        .unwrap_or((PathKind::Connecting, None));
    for (i, id) in order.iter().enumerate() {
        if id == my_id {
            continue;
        }
        let given = names.get(i).map(|n| clean_name(n));
        if let Some(e) = p.map.get_mut(id) {
            if let Some(name) = given {
                e.peer.name = name;
            }
            continue;
        }
        let name = given.unwrap_or_else(|| id.chars().take(8).collect());
        p.map.insert(
            id.clone(),
            PeerEntry {
                peer: Peer {
                    peer_id: id.clone(),
                    name,
                    uuid: None,
                    path,
                    rtt_ms: rtt,
                    speaking: false,
                    muted: false,
                },
                conn: None,
                tx: None,
                ban_uuid: None,
            },
        );
    }
    p.order = order;
}

fn spawn_writer(mut send: SendStream, mut rx: mpsc::Receiver<String>) {
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if send.write_all(line.as_bytes()).await.is_err() {
                return;
            }
        }
        let _ = send.finish();
    });
}

/// Host: every bi-stream after the control stream is one game connection to
/// the LAN port fixed at create time.
async fn host_game_streams(conn: Connection, lan_port: u16, cancel: CancellationToken) {
    loop {
        let (send, recv) = tokio::select! {
            () = cancel.cancelled() => return,
            s = conn.accept_bi() => match s {
                Ok(s) => s,
                Err(_) => return,
            },
        };
        tokio::spawn(serve_game_stream(send, recv, lan_port));
    }
}

async fn serve_game_stream(mut send: SendStream, mut recv: RecvStream, port: u16) {
    let open = match wire::read_line_unbuffered(&mut recv, MAX_OPEN_LINE).await {
        Ok(l) => l,
        Err(_) => return,
    };
    if !matches!(
        serde_json::from_str::<StreamOpen>(&open),
        Ok(StreamOpen::Game)
    ) {
        let _ = send.reset(1u32.into());
        return;
    }
    match TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
        Ok(tcp) => pipe(tcp, send, recv).await,
        Err(_) => {
            let _ = send.reset(1u32.into());
        }
    }
}

/// Guest: each local TCP connection becomes one bi-stream to the host.
async fn guest_tunnel(listener: TcpListener, conn: Connection, cancel: CancellationToken) {
    loop {
        let tcp = tokio::select! {
            () = cancel.cancelled() => return,
            a = listener.accept() => match a {
                Ok((tcp, _)) => tcp,
                Err(_) => continue,
            },
        };
        let conn = conn.clone();
        tokio::spawn(async move {
            let Ok((mut send, recv)) = conn.open_bi().await else {
                return;
            };
            let Ok(open) = wire::encode(&StreamOpen::Game) else {
                return;
            };
            if send.write_all(open.as_bytes()).await.is_err() {
                return;
            }
            pipe(tcp, send, recv).await;
        });
    }
}

async fn pipe(tcp: TcpStream, mut send: SendStream, mut recv: RecvStream) {
    let _ = tcp.set_nodelay(true);
    let (mut r, mut w) = tcp.into_split();
    let up = async {
        let _ = tokio::io::copy(&mut r, &mut send).await;
        let _ = send.finish();
    };
    let down = async {
        let _ = tokio::io::copy(&mut recv, &mut w).await;
        let _ = w.shutdown().await;
    };
    tokio::join!(up, down);
}

fn path_info(conn: &Connection) -> (PathKind, Option<u32>) {
    let paths = conn.paths();
    let selected = paths.iter().find(|p| p.is_selected());
    match selected {
        Some(p) => {
            let kind = if p.is_relay() {
                PathKind::Relay
            } else {
                PathKind::Direct
            };
            let rtt = u32::try_from(p.rtt().as_millis()).unwrap_or(u32::MAX);
            (kind, Some(rtt))
        }
        None => (PathKind::Connecting, None),
    }
}

fn significant(peer: &Peer, kind: PathKind, rtt: Option<u32>) -> bool {
    if peer.path != kind {
        return true;
    }
    match (peer.rtt_ms, rtt) {
        (Some(a), Some(b)) => a.abs_diff(b) >= 5,
        (a, b) => a.is_some() != b.is_some(),
    }
}

fn ekm(conn: &Connection, room_id: RoomId) -> AppResult<[u8; 32]> {
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, wire::EKM_LABEL, room_id.0.as_bytes())
        .map_err(|_| AppError::internal("tls keying material export failed"))?;
    Ok(out)
}

fn clean_name(name: &str) -> String {
    let n: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    let n = n.trim();
    if n.is_empty() {
        "Player".into()
    } else {
        n.to_owned()
    }
}

fn net_err(what: &str, e: impl fmt::Display) -> AppError {
    AppError::new(ErrorCode::Network, format!("{what}: {e}"))
}

fn timeout_err(stage: &str) -> AppError {
    AppError::new(ErrorCode::Network, format!("p2p {stage} timed out")).detail("reason", "timeout")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_types::version::LoaderKind;

    fn sig(mods: &str) -> InstanceSig {
        InstanceSig {
            mc: "1.21.1".into(),
            loader: LoaderKind::Fabric,
            loader_version: Some("0.16.0".into()),
            mods_hash: mods.into(),
        }
    }

    #[test]
    fn sig_mismatch_is_conflict_with_details() {
        assert!(check_sig(&sig("a"), &sig("a")).is_ok());
        let e = check_sig(&sig("a"), &sig("b")).unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        assert_eq!(e.details.unwrap()["mods"], "mismatch");
    }

    #[test]
    fn kick_reasons_map_to_codes() {
        assert_eq!(
            kick_error(reasons::BAD_PASSWORD).code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            kick_error(reasons::RATE_LIMITED).code,
            ErrorCode::RateLimited
        );
        assert_eq!(kick_error(reasons::ROOM_FULL).code, ErrorCode::Conflict);
    }

    #[test]
    fn host_auth_mode_must_match_account() {
        let offline = Identity {
            name: "a".into(),
            uuid: None,
            hub_token: None,
        };
        let online = Identity {
            hub_token: Some(Secret::new("t")),
            ..offline.clone()
        };
        assert!(check_host_auth_mode(RoomAuthMode::Open, &offline).is_ok());
        assert!(check_host_auth_mode(RoomAuthMode::Yggdrasil, &online).is_ok());
        let e = check_host_auth_mode(RoomAuthMode::Yggdrasil, &offline).unwrap_err();
        assert_eq!(e.details.unwrap()["expected"], "open");
        let e = check_host_auth_mode(RoomAuthMode::Open, &online).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
        assert_eq!(e.details.unwrap()["expected"], "yggdrasil");
    }

    #[test]
    fn uuids_normalize() {
        assert_eq!(
            normalize_uuid("ABCD-ef"),
            normalize_uuid("abcdef"),
            "dashes and case are ignored"
        );
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  \u{7}Steve\n "), "Steve");
        assert_eq!(clean_name(""), "Player");
        assert_eq!(clean_name(&"x".repeat(100)).len(), MAX_NAME_CHARS);
    }

    #[test]
    fn welcome_names_are_applied() {
        let mut p = Peers::default();
        apply_welcome(
            &mut p,
            vec!["host".into(), "me".into(), "g2".into()],
            vec!["Hosty".into(), "Me".into(), "Alex".into()],
            "me",
            "host",
        );
        assert_eq!(p.map["host"].peer.name, "Hosty");
        assert_eq!(p.map["g2"].peer.name, "Alex");
        assert!(!p.map.contains_key("me"));
        apply_welcome(
            &mut p,
            vec!["host".into(), "me".into()],
            vec![],
            "me",
            "host",
        );
        assert!(!p.map.contains_key("g2"));
    }

    #[test]
    fn rtt_jitter_is_ignored() {
        let mut p = Peer {
            peer_id: "a".into(),
            name: "a".into(),
            uuid: None,
            path: PathKind::Direct,
            rtt_ms: Some(40),
            speaking: false,
            muted: false,
        };
        assert!(!significant(&p, PathKind::Direct, Some(42)));
        assert!(significant(&p, PathKind::Direct, Some(60)));
        assert!(significant(&p, PathKind::Relay, Some(40)));
        p.rtt_ms = None;
        assert!(significant(&p, PathKind::Direct, Some(40)));
    }
}
