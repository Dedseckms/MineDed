//! End-to-end rooms tests: mock hub (axum), two in-process iroh endpoints,
//! a TCP echo server standing in for the host's LAN world.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use axum::{Json, Router};
use lc_http::{DownloaderConfig, HttpDownloader};
use lc_p2p::iroh::address_lookup::memory::MemoryLookup;
use lc_p2p::iroh::endpoint::Builder;
use lc_p2p::{
    make_peer_token, proto::VoiceHeader, DatagramTransport, EndpointTweak, Identity, P2pConfig,
    P2pRooms, RoomEnv, RoomObserver, RoomService,
};
use lc_runtime::{LaunchPlan, MemorySettingsStore, SettingsStore};
use lc_types::instance::LaunchOpts;
use lc_types::room::{InstanceSig, PathKind, Peer, RoomAuthMode, RoomKind, RoomRole, RoomState};
use lc_types::settings::Settings;
use lc_types::version::LoaderKind;
use lc_types::voice::ChatMessage;
use lc_types::{AppResult, ErrorCode, InstanceId, RoomId};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ------------------------------------------------------------- mock hub

#[derive(Default)]
struct HubState {
    room: Option<Value>,
    relay_urls: Vec<String>,
    closed: bool,
    /// Issue tokens signed with the wrong key (forgery test).
    forge_tokens: bool,
    /// Last join body (name/uuid checks).
    last_join: Option<Value>,
    create_had_bearer: bool,
    /// Answer `401 authExpired` to creates with a bearer (live hub, v1.0.1).
    reject_bearer: bool,
}

type Hub = Arc<Mutex<HubState>>;

async fn create_room(
    State(hub): State<Hub>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let room_id = RoomId::new();
    let mut h = hub.lock().unwrap();
    h.create_had_bearer = headers.contains_key(axum::http::header::AUTHORIZATION);
    if h.create_had_bearer && h.reject_bearer {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"code": "authExpired", "message": "token is invalid or expired"})),
        );
    }
    let mut room = body.clone();
    room["roomId"] = json!(room_id);
    h.room = Some(room);
    h.closed = false;
    (
        StatusCode::CREATED,
        Json(json!({
            "roomId": room_id,
            "code": "ABCD-1234",
            "hostToken": "host-token",
            "expiresAt": "2026-10-22T12:00:00Z",
            "relayUrls": h.relay_urls,
        })),
    )
}

async fn join_room(
    State(hub): State<Hub>,
    Path(code): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut h = hub.lock().unwrap();
    h.last_join = Some(body.clone());
    let h = &*h;
    let Some(room) = h.room.as_ref().filter(|_| code == "ABCD-1234" && !h.closed) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"code": "notFound", "message": "no such room"})),
        );
    };
    let room_id: RoomId = serde_json::from_value(room["roomId"].clone()).unwrap();
    let key = if h.forge_tokens {
        "not-the-host-token"
    } else {
        "host-token"
    };
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    let token = make_peer_token(
        key,
        room_id,
        body["endpointId"].as_str().unwrap(),
        None,
        exp,
    );
    (
        StatusCode::OK,
        Json(json!({
            "roomId": room["roomId"],
            "peerToken": token,
            "hostEndpointId": room["endpointId"],
            "kind": room.get("kind").cloned().unwrap_or(json!("game")),
            "sig": room["sig"],
            "authMode": room["authMode"],
            "relayUrls": h.relay_urls,
            "passwordProtected": room["passwordProtected"],
        })),
    )
}

async fn close_room(State(hub): State<Hub>, Path(_id): Path<String>) -> StatusCode {
    hub.lock().unwrap().closed = true;
    StatusCode::NO_CONTENT
}

async fn spawn_hub(relay_urls: Vec<String>) -> (String, Hub) {
    let hub: Hub = Arc::new(Mutex::new(HubState {
        relay_urls,
        ..HubState::default()
    }));
    let app = Router::new()
        .route("/api/v1/rooms", post(create_room))
        .route("/api/v1/rooms/{code}/join", post(join_room))
        .route("/api/v1/rooms/{id}", delete(close_room))
        .with_state(hub.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/api/v1"), hub)
}

// ------------------------------------------------------- env / observer

struct FakeEnv {
    name: &'static str,
    mods: &'static str,
}

#[async_trait]
impl RoomEnv for FakeEnv {
    async fn instance_sig(&self, _instance: InstanceId) -> AppResult<InstanceSig> {
        Ok(InstanceSig {
            mc: "1.21.1".into(),
            loader: LoaderKind::Fabric,
            loader_version: Some("0.16.9".into()),
            mods_hash: self.mods.into(),
        })
    }

    async fn identity(&self) -> AppResult<Identity> {
        Ok(Identity {
            name: self.name.into(),
            uuid: None,
            hub_token: None,
        })
    }
}

#[derive(Default)]
struct Rec {
    states: Mutex<Vec<Option<RoomState>>>,
    chats: Mutex<Vec<ChatMessage>>,
    peers: Mutex<Vec<(Peer, bool)>>,
}

impl RoomObserver for Rec {
    fn on_state(&self, state: Option<RoomState>) {
        self.states.lock().unwrap().push(state);
    }
    fn on_peer(&self, peer: Peer, joined: bool) {
        self.peers.lock().unwrap().push((peer, joined));
    }
    fn on_chat(&self, message: ChatMessage) {
        self.chats.lock().unwrap().push(message);
    }
}

async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..400 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

// --------------------------------------------------------------- setup

fn loopback_only(b: Builder) -> Builder {
    b.clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .expect("bind addr")
}

struct Side {
    rooms: P2pRooms,
    rec: Arc<Rec>,
}

async fn side(hub_url: &str, cfg: P2pConfig, env: FakeEnv) -> Side {
    side_env(hub_url, cfg, Arc::new(env)).await
}

async fn side_env(hub_url: &str, cfg: P2pConfig, env: Arc<dyn RoomEnv>) -> Side {
    let settings = Arc::new(MemorySettingsStore::default());
    settings
        .set(Settings {
            hub_url: hub_url.to_owned(),
            ..Settings::default()
        })
        .await
        .unwrap();
    let rec = Arc::new(Rec::default());
    let downloader = Arc::new(HttpDownloader::new(DownloaderConfig::default()).unwrap());
    let rooms = P2pRooms::new(cfg, downloader, settings, env, rec.clone());
    Side { rooms, rec }
}

fn base_cfg(tweak: EndpointTweak, relays: Vec<String>) -> P2pConfig {
    P2pConfig {
        relay_override: Some(relays),
        hello_timeout: Duration::from_secs(10),
        connect_timeout: Duration::from_secs(20),
        lan_multicast: false,
        endpoint_tweak: Some(tweak),
        ..P2pConfig::default()
    }
}

/// Echo server standing in for the integrated server's LAN port.
async fn spawn_echo() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let (mut r, mut w) = s.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    port
}

async fn echo_through(addr: &str, payload: &[u8]) -> Vec<u8> {
    let mut tcp = TcpStream::connect(addr).await.unwrap();
    tcp.write_all(payload).await.unwrap();
    let mut back = vec![0u8; payload.len()];
    tokio::time::timeout(Duration::from_secs(30), tcp.read_exact(&mut back))
        .await
        .expect("echo timed out")
        .unwrap();
    back
}

/// Relay tests: guests can only reach a host that is already registered
/// with its home relay.
async fn wait_home_relay(rooms: &P2pRooms) {
    eventually("host home relay", || {
        rooms
            .endpoint_addr()
            .is_some_and(|a| a.relay_urls().next().is_some())
    })
    .await;
}

/// Host + guest wired for direct connections on loopback (no relay).
async fn direct_pair(password: Option<&str>) -> (Side, Side, Hub, MemoryLookup) {
    let (hub_url, hub) = spawn_hub(vec![]).await;
    let host_tweak: EndpointTweak = Arc::new(loopback_only);
    let lookup = MemoryLookup::new();
    let guest_lookup = lookup.clone();
    let guest_tweak: EndpointTweak =
        Arc::new(move |b| loopback_only(b).address_lookup(guest_lookup.clone()));
    let host = side(
        &hub_url,
        base_cfg(host_tweak, vec![]),
        FakeEnv {
            name: "Host",
            mods: "m1",
        },
    )
    .await;
    let guest = side(
        &hub_url,
        base_cfg(guest_tweak, vec![]),
        FakeEnv {
            name: "Guest",
            mods: "m1",
        },
    )
    .await;
    host.rooms.set_lan_port(Some(spawn_echo().await));
    let st = host
        .rooms
        .create(
            InstanceId::new(),
            RoomAuthMode::Open,
            password.map(str::to_owned),
        )
        .await
        .unwrap();
    assert_eq!(st.role, RoomRole::Host);
    assert_eq!(st.link, "https://mineded.dedseckms.ru/join/ABCD-1234");
    assert_eq!(st.password_protected, password.is_some());
    // Offline host, open room: created anonymously.
    assert!(!hub.lock().unwrap().create_had_bearer);
    lookup.add_endpoint_info(host.rooms.endpoint_addr().unwrap());
    (host, guest, hub, lookup)
}

// --------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_tunnel_chat_datagrams_kick() {
    let (host, guest, hub, _lookup) = direct_pair(None).await;

    let st = guest
        .rooms
        .join("mineup://join/abcd1234", Some(InstanceId::new()), None)
        .await
        .unwrap();
    assert_eq!(st.role, RoomRole::Guest);
    let local = st.local_addr.clone().expect("guest local addr");
    assert!(local.starts_with("127.0.0.1:"));
    assert_eq!(st.peers.len(), 1);
    assert_eq!(st.peers[0].name, "Host");

    // Game traffic: two parallel TCP connections through the tunnel.
    let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let (a, b) = tokio::join!(
        echo_through(&local, b"hello world"),
        echo_through(&local, &big)
    );
    assert_eq!(a, b"hello world");
    assert_eq!(b, big);

    // Host sees the guest, path becomes Direct with an RTT.
    eventually("host peer direct", || {
        host.rec
            .states
            .lock()
            .unwrap()
            .last()
            .cloned()
            .flatten()
            .is_some_and(|s| {
                s.peers.len() == 1
                    && s.peers[0].name == "Guest"
                    && s.peers[0].path == PathKind::Direct
                    && s.peers[0].rtt_ms.is_some()
            })
    })
    .await;

    // Chat both ways.
    let sent = guest.rooms.send_chat("  hi host ").await.unwrap();
    assert_eq!(sent.text, "hi host");
    eventually("chat at host", || {
        host.rec
            .chats
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.text == "hi host" && c.from_name == "Guest" && !c.via_relay)
    })
    .await;
    host.rooms.send_chat("hi guest").await.unwrap();
    eventually("chat at guest", || {
        guest
            .rec
            .chats
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.text == "hi guest" && c.from_name == "Host")
    })
    .await;
    assert_eq!(
        guest.rooms.send_chat("   ").await.unwrap_err().code,
        ErrorCode::InvalidInput
    );

    // Voice datagrams.
    assert!(guest.rooms.max_datagram_size() >= 1000);
    guest.rooms.send(None, b"opus-frame").await.unwrap();
    let got = tokio::time::timeout(Duration::from_secs(10), host.rooms.recv())
        .await
        .expect("datagram timed out")
        .expect("room open");
    assert_eq!(got.1, b"opus-frame");
    let guest_id = got.0.clone();
    host.rooms.send(Some(&guest_id), b"back").await.unwrap();
    let got = tokio::time::timeout(Duration::from_secs(10), guest.rooms.recv())
        .await
        .expect("datagram timed out")
        .expect("room open");
    assert_eq!(got.1, b"back");
    assert_eq!(
        guest
            .rooms
            .send(None, &vec![0u8; 100_000])
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );

    // Quick play into the tunnel for guest launches of this room.
    let contributor = guest.rooms.launch_contributor();
    let mut plan = LaunchPlan::default();
    let opts = LaunchOpts {
        room: Some(st.room_id),
        ..LaunchOpts::default()
    };
    let inst: lc_types::instance::Instance = serde_json::from_value(json!({
        "id": InstanceId::new(), "name": "x", "mc": "1.21.1",
        "loader": {"kind": "fabric", "version": "0.16.9"},
        "java": {"path": null, "major": null},
        "memory": {"minMb": 512, "maxMb": 2048, "auto": false},
        "jvmArgs": [], "preset": null, "icon": null,
        "createdAt": "2026-01-01T00:00:00Z", "lastPlayed": null, "playtimeSec": 0
    }))
    .unwrap();
    contributor.contribute(&inst, &opts, &mut plan).unwrap();
    let qp = plan.quick_play.expect("quick play");
    assert_eq!(format!("{}:{}", qp.host, qp.port), local);

    // Kick: guest drops out of the room.
    assert_eq!(
        guest.rooms.kick(&guest_id).await.unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    host.rooms.kick(&guest_id).await.unwrap();
    eventually("guest kicked", || {
        guest
            .rec
            .states
            .lock()
            .unwrap()
            .last()
            .is_some_and(Option::is_none)
    })
    .await;
    assert!(guest.rooms.state().await.unwrap().is_none());
    assert!(guest.rooms.recv().await.is_none());
    assert!(host.rooms.state().await.unwrap().unwrap().peers.is_empty());
    assert_eq!(
        hub.lock().unwrap().last_join.as_ref().unwrap()["name"],
        "Guest"
    );

    // Kicked launchers stay out for the room's lifetime.
    let err = guest
        .rooms
        .join("ABCD-1234", Some(InstanceId::new()), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert_eq!(err.details.unwrap()["reason"], "banned");

    host.rooms.leave().await.unwrap();
    assert!(host.rooms.state().await.unwrap().is_none());
    assert!(hub.lock().unwrap().closed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn password_is_checked_by_host() {
    let (host, guest, _hub, _lookup) = direct_pair(Some("open sesame")).await;

    let err = guest
        .rooms
        .join("ABCD-1234", Some(InstanceId::new()), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);

    let err = guest
        .rooms
        .join("ABCD-1234", Some(InstanceId::new()), Some("wrong".into()))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied, "{err:?}");
    assert_eq!(err.details.unwrap()["reason"], "bad_password");
    assert!(guest.rooms.state().await.unwrap().is_none());
    assert!(host.rooms.state().await.unwrap().unwrap().peers.is_empty());

    let st = guest
        .rooms
        .join(
            "ABCD-1234",
            Some(InstanceId::new()),
            Some("open sesame".into()),
        )
        .await
        .unwrap();
    assert!(st.password_protected);
    let local = st.local_addr.unwrap();
    assert_eq!(echo_through(&local, b"ping").await, b"ping");
    guest.rooms.leave().await.unwrap();
    eventually("host sees guest leave", || {
        host.rec
            .peers
            .lock()
            .unwrap()
            .iter()
            .any(|(_, joined)| !joined)
    })
    .await;

    // Brute force: after 5 wrong passwords even the right one is refused.
    for _ in 0..5 {
        let err = guest
            .rooms
            .join("ABCD-1234", Some(InstanceId::new()), Some("guess".into()))
            .await
            .unwrap_err();
        assert_eq!(err.details.unwrap()["reason"], "bad_password");
    }
    let err = guest
        .rooms
        .join(
            "ABCD-1234",
            Some(InstanceId::new()),
            Some("open sesame".into()),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::RateLimited, "{err:?}");
    host.rooms.leave().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forged_peer_token_is_rejected() {
    let (_host, guest, hub, _lookup) = direct_pair(None).await;
    hub.lock().unwrap().forge_tokens = true;
    let err = guest
        .rooms
        .join("ABCD-1234", Some(InstanceId::new()), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert_eq!(err.details.unwrap()["reason"], "bad_token");
}

/// Regression: a chat line (or anything but the server thread) must not move
/// the tunnel, and the port is locked once the room exists.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_cannot_redirect_tunnel() {
    let (hub_url, _hub) = spawn_hub(vec![]).await;
    let host_tweak: EndpointTweak = Arc::new(loopback_only);
    let lookup = MemoryLookup::new();
    let guest_lookup = lookup.clone();
    let guest_tweak: EndpointTweak =
        Arc::new(move |b| loopback_only(b).address_lookup(guest_lookup.clone()));
    let host = side(
        &hub_url,
        base_cfg(host_tweak, vec![]),
        FakeEnv {
            name: "Host",
            mods: "m1",
        },
    )
    .await;
    let guest = side(
        &hub_url,
        base_cfg(guest_tweak, vec![]),
        FakeEnv {
            name: "Guest",
            mods: "m1",
        },
    )
    .await;
    let world = spawn_echo().await;
    // "Sensitive" local service that must never be reachable.
    let evil_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let evil = evil_l.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = evil_l.accept().await {
            let _ = s.write_all(b"EVIL!!").await;
        }
    });
    let inst = InstanceId::new();
    host.rooms.feed_game_log(
        inst,
        &format!("[12:00:00] [Server thread/INFO]: Started serving on {world}"),
    );
    host.rooms.feed_game_log(
        inst,
        &format!("[12:00:05] [Render thread/INFO]: [CHAT] <Mallory> Started serving on {evil}"),
    );
    host.rooms.feed_game_log(
        inst,
        &format!("[12:00:05] [Server thread/INFO]: <Mallory> Started serving on {evil}"),
    );
    host.rooms
        .create(inst, RoomAuthMode::Open, None)
        .await
        .unwrap();
    lookup.add_endpoint_info(host.rooms.endpoint_addr().unwrap());
    // Even a genuine-looking server line after create changes nothing.
    host.rooms.feed_game_log(
        inst,
        &format!("[12:01:00] [Server thread/INFO]: Started serving on {evil}"),
    );
    host.rooms.set_lan_port(Some(evil));

    let st = guest
        .rooms
        .join("ABCD-1234", Some(InstanceId::new()), None)
        .await
        .unwrap();
    let local = st.local_addr.unwrap();
    assert_eq!(echo_through(&local, b"secret").await, b"secret");
    guest.rooms.leave().await.unwrap();
    host.rooms.leave().await.unwrap();
}

/// `FakeEnv` with a game dir (servers.dat entry).
struct DirEnv(FakeEnv, std::path::PathBuf);

#[async_trait]
impl RoomEnv for DirEnv {
    async fn instance_sig(&self, instance: InstanceId) -> AppResult<InstanceSig> {
        self.0.instance_sig(instance).await
    }
    async fn identity(&self) -> AppResult<Identity> {
        self.0.identity().await
    }
    fn game_dir(&self, _instance: InstanceId) -> Option<std::path::PathBuf> {
        Some(self.1.clone())
    }
}

/// Host env exporting a fixed pack; guest env with its own pack dir.
struct PackEnv {
    base: DirEnv,
    pack: Vec<u8>,
    exports: Arc<std::sync::atomic::AtomicUsize>,
    /// Instance whose mod set differs from the host's.
    bad: InstanceId,
}

#[async_trait]
impl RoomEnv for PackEnv {
    async fn instance_sig(&self, instance: InstanceId) -> AppResult<InstanceSig> {
        let mut sig = self.base.instance_sig(instance).await?;
        if instance == self.bad {
            sig.mods_hash = "other".into();
        }
        Ok(sig)
    }
    async fn identity(&self) -> AppResult<Identity> {
        self.base.identity().await
    }
    fn game_dir(&self, instance: InstanceId) -> Option<std::path::PathBuf> {
        self.base.game_dir(instance)
    }
    async fn export_pack(&self, _instance: InstanceId) -> AppResult<std::path::PathBuf> {
        self.exports
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = self.pack_dir();
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(format!("host-{}.mrpack", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, &self.pack).await?;
        Ok(path)
    }
    fn pack_dir(&self) -> std::path::PathBuf {
        self.base.1.join("packs")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_pack_roundtrip_and_attach() {
    let (hub_url, _hub) = spawn_hub(vec![]).await;
    let host_tweak: EndpointTweak = Arc::new(loopback_only);
    let lookup = MemoryLookup::new();
    let gl = lookup.clone();
    let guest_tweak: EndpointTweak = Arc::new(move |b| loopback_only(b).address_lookup(gl.clone()));
    let root = std::env::temp_dir().join(format!("lc-p2p-pack-{}", uuid::Uuid::new_v4()));
    let pack: Vec<u8> = (0..700_000u32).map(|i| (i % 253) as u8).collect();
    let exports = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let bad = InstanceId::new();
    let env = |name: &'static str, sub: &str| PackEnv {
        base: DirEnv(FakeEnv { name, mods: "m1" }, root.join(sub)),
        pack: pack.clone(),
        exports: exports.clone(),
        bad,
    };
    let host = side_env(
        &hub_url,
        base_cfg(host_tweak, vec![]),
        Arc::new(env("Host", "host")),
    )
    .await;
    let guest = side_env(
        &hub_url,
        base_cfg(guest_tweak, vec![]),
        Arc::new(env("Guest", "guest")),
    )
    .await;
    host.rooms
        .create(InstanceId::new(), RoomAuthMode::Open, None)
        .await
        .unwrap();
    lookup.add_endpoint_info(host.rooms.endpoint_addr().unwrap());

    // Host can't download its own pack.
    let err = host
        .rooms
        .download_host_pack(&lc_runtime::NullSink::default())
        .await
        .unwrap_err();
    assert_eq!(err.details.unwrap()["reason"], "notGameGuest");

    // Joined without an instance: no server-list entry yet.
    let st = guest.rooms.join("ABCD-1234", None, None).await.unwrap();
    let local = st.local_addr.unwrap();
    let dat = root.join("guest").join("servers.dat");
    assert!(!dat.exists());

    // Two downloads, one export (cached per room).
    for _ in 0..2 {
        let path = guest
            .rooms
            .download_host_pack(&lc_runtime::NullSink::default())
            .await
            .unwrap();
        assert!(path.ends_with(".mrpack"));
        assert_eq!(std::fs::read(&path).unwrap(), pack);
        std::fs::remove_file(&path).unwrap();
    }
    assert_eq!(exports.load(std::sync::atomic::Ordering::SeqCst), 1);

    // A mismatching instance is refused; a matching one gets the
    // server-list entry.
    let err = guest.rooms.attach_instance(bad).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    guest
        .rooms
        .attach_instance(InstanceId::new())
        .await
        .unwrap();
    let bytes = std::fs::read(&dat).unwrap();
    let needle = format!("127.0.0.1:{}", local.rsplit(':').next().unwrap());
    assert!(bytes.windows(needle.len()).any(|w| w == needle.as_bytes()));

    guest.rooms.leave().await.unwrap();
    host.rooms.leave().await.unwrap();
    // The host's cached export is gone with the room.
    let left = std::fs::read_dir(root.join("host").join("packs"))
        .unwrap()
        .count();
    assert_eq!(left, 0);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn join_without_instance_host_cannot_export() {
    let (host, guest, _hub, _lookup) = direct_pair(None).await;
    let st = guest.rooms.join("ABCD-1234", None, None).await.unwrap();
    assert!(st.local_addr.is_some());
    // The host's env can't export: clear error instead of a hang.
    let err = guest
        .rooms
        .download_host_pack(&lc_runtime::NullSink::default())
        .await
        .unwrap_err();
    assert_eq!(err.details.unwrap()["reason"], "hostExportFailed");
    guest
        .rooms
        .attach_instance(InstanceId::new())
        .await
        .unwrap();
    guest.rooms.leave().await.unwrap();
    host.rooms.leave().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn room_before_world_open_and_server_list() {
    let (hub_url, _hub) = spawn_hub(vec![]).await;
    let host_tweak: EndpointTweak = Arc::new(loopback_only);
    let lookup = MemoryLookup::new();
    let gl = lookup.clone();
    let guest_tweak: EndpointTweak = Arc::new(move |b| loopback_only(b).address_lookup(gl.clone()));
    let host = side(
        &hub_url,
        base_cfg(host_tweak, vec![]),
        FakeEnv {
            name: "Host",
            mods: "m1",
        },
    )
    .await;
    let dir = std::env::temp_dir().join(format!("lc-p2p-room-{}", uuid::Uuid::new_v4()));
    let guest = side_env(
        &hub_url,
        base_cfg(guest_tweak, vec![]),
        Arc::new(DirEnv(
            FakeEnv {
                name: "Guest",
                mods: "m1",
            },
            dir.clone(),
        )),
    )
    .await;
    let inst = InstanceId::new();
    let t = std::time::Instant::now();
    let st = host
        .rooms
        .create(inst, RoomAuthMode::Open, None)
        .await
        .unwrap();
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "create must not wait for LAN"
    );
    assert_eq!(st.world_open, Some(false));
    lookup.add_endpoint_info(host.rooms.endpoint_addr().unwrap());

    let st = guest
        .rooms
        .join("ABCD-1234", Some(inst), None)
        .await
        .unwrap();
    assert_eq!(st.world_open, None);
    let local = st.local_addr.unwrap();
    let dat = std::fs::read(dir.join("servers.dat")).unwrap();
    let needle = format!("127.0.0.1:{}", local.rsplit(':').next().unwrap());
    assert!(dat.windows(needle.len()).any(|w| w == needle.as_bytes()));
    assert!(dat.windows(7).any(|w| w == b"MineDed"));

    // World not open: the game connection is closed, nothing is echoed.
    let mut tcp = TcpStream::connect(&local).await.unwrap();
    let _ = tcp.write_all(b"x").await;
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_secs(10), tcp.read(&mut buf))
        .await
        .expect("closed")
        .unwrap_or(0);
    assert_eq!(n, 0);

    // Opened to LAN later: detected from the game log.
    let port = spawn_echo().await;
    host.rooms.feed_game_log(
        inst,
        &format!("[12:00:00] [Render thread/INFO]: Started serving on {port}"),
    );
    eventually("world open", || {
        host.rec
            .states
            .lock()
            .unwrap()
            .last()
            .and_then(|s| s.as_ref())
            .is_some_and(|s| s.world_open == Some(true))
    })
    .await;
    assert_eq!(echo_through(&local, b"late").await, b"late");

    guest.rooms.leave().await.unwrap();
    let dat = std::fs::read(dir.join("servers.dat")).unwrap();
    assert!(!dat.windows(7).any(|w| w == b"MineDed"));
    host.rooms.leave().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn instance_mismatch_and_bad_codes() {
    let (hub_url, _hub) = spawn_hub(vec![]).await;
    let tweak: EndpointTweak = Arc::new(loopback_only);
    let host = side(
        &hub_url,
        base_cfg(tweak.clone(), vec![]),
        FakeEnv {
            name: "Host",
            mods: "m1",
        },
    )
    .await;
    let guest = side(
        &hub_url,
        base_cfg(tweak, vec![]),
        FakeEnv {
            name: "Guest",
            mods: "other",
        },
    )
    .await;

    // No LAN world yet: the room exists, the world is marked closed.
    let st = host
        .rooms
        .create(InstanceId::new(), RoomAuthMode::Open, None)
        .await
        .unwrap();
    assert_eq!(st.world_open, Some(false));
    host.rooms.leave().await.unwrap();

    // Port from the game log of the hosted instance.
    let port = spawn_echo().await;
    let inst = InstanceId::new();
    host.rooms.feed_game_log(
        inst,
        &format!("[12:00:00] [Server thread/INFO]: Started serving on {port}"),
    );
    // An offline host cannot run a yggdrasil (online-mode) room.
    let err = host
        .rooms
        .create(inst, RoomAuthMode::Yggdrasil, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(err.details.unwrap()["reason"], "authModeMismatch");
    host.rooms
        .create(inst, RoomAuthMode::Open, None)
        .await
        .unwrap();
    assert_eq!(
        host.rooms
            .create(InstanceId::new(), RoomAuthMode::Open, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );

    let err = guest
        .rooms
        .join("ABCD-1234", Some(InstanceId::new()), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    let err = guest
        .rooms
        .join("ABC!-123", Some(InstanceId::new()), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    let err = guest
        .rooms
        .join("ZZZZ-9999", Some(InstanceId::new()), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    host.rooms.leave().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tunnel_via_local_relay() {
    use lc_p2p::iroh::tls::CaTlsConfig;

    let (_map, relay_url, _relay) = lc_p2p::iroh::test_utils::run_relay_server().await.unwrap();
    let relay = relay_url.to_string();
    let (hub_url, _hub) = spawn_hub(vec![relay.clone()]).await;
    let host_tweak: EndpointTweak =
        Arc::new(|b: Builder| loopback_only(b).ca_tls_config(CaTlsConfig::insecure_skip_verify()));
    // The guest has no IP transport at all: everything must go via the relay.
    let guest_tweak: EndpointTweak = Arc::new(|b: Builder| {
        b.clear_ip_transports()
            .ca_tls_config(CaTlsConfig::insecure_skip_verify())
    });
    let mut host_cfg = base_cfg(host_tweak, vec![]);
    host_cfg.relay_override = None; // take relay URLs from hub
    let mut guest_cfg = base_cfg(guest_tweak, vec![]);
    guest_cfg.relay_override = None;
    let host = side(
        &hub_url,
        host_cfg,
        FakeEnv {
            name: "Host",
            mods: "m1",
        },
    )
    .await;
    let guest = side(
        &hub_url,
        guest_cfg,
        FakeEnv {
            name: "Guest",
            mods: "m1",
        },
    )
    .await;
    host.rooms.set_lan_port(Some(spawn_echo().await));
    host.rooms
        .create(InstanceId::new(), RoomAuthMode::Open, None)
        .await
        .unwrap();
    wait_home_relay(&host.rooms).await;

    let st = guest
        .rooms
        .join("ABCD-1234", Some(InstanceId::new()), None)
        .await
        .unwrap();
    let local = st.local_addr.unwrap();
    assert_eq!(echo_through(&local, b"via relay").await, b"via relay");
    eventually("guest path relay", || {
        guest
            .rec
            .states
            .lock()
            .unwrap()
            .last()
            .cloned()
            .flatten()
            .is_some_and(|s| {
                s.peers
                    .first()
                    .is_some_and(|p| p.path == PathKind::Relay && p.rtt_ms.is_some())
            })
    })
    .await;
    guest.rooms.leave().await.unwrap();
    host.rooms.leave().await.unwrap();
}

// ---------------------------------------------------------- voice rooms

/// Online (Yggdrasil) account: voice rooms are registered with a bearer.
struct OnlineEnv;

#[async_trait]
impl RoomEnv for OnlineEnv {
    async fn instance_sig(&self, _instance: InstanceId) -> AppResult<InstanceSig> {
        Err(lc_types::AppError::internal(
            "voice rooms never look at instances",
        ))
    }

    async fn identity(&self) -> AppResult<Identity> {
        Ok(Identity {
            name: "Online".into(),
            uuid: Some("0123456789abcdef0123456789abcdef".into()),
            hub_token: Some(lc_accounts::Secret::new("access-token")),
        })
    }
}

fn last_state(rec: &Rec) -> Option<RoomState> {
    rec.states.lock().unwrap().last().cloned().flatten()
}

async fn recv_dgram(side: &Side) -> (String, Vec<u8>) {
    tokio::time::timeout(Duration::from_secs(10), side.rooms.recv())
        .await
        .expect("datagram timed out")
        .expect("room open")
}

fn frame(speaker: u8, seq: u16, opus: &[u8]) -> Vec<u8> {
    VoiceHeader {
        speaker,
        seq,
        timestamp_ms: u32::from(seq) * 20,
    }
    .encode(opus)
}

/// Voice host + two guests; `relay` = guests have no IP transport at all.
async fn voice_trio(relay: bool, password: Option<&str>) -> (Side, Side, Side, Hub) {
    use lc_p2p::iroh::tls::CaTlsConfig;

    let mut relay_urls = vec![];
    if relay {
        let (_map, url, server) = lc_p2p::iroh::test_utils::run_relay_server().await.unwrap();
        // Keep the relay alive for the whole test process.
        std::mem::forget(server);
        relay_urls.push(url.to_string());
    }
    let (hub_url, hub) = spawn_hub(relay_urls).await;
    let lookup = MemoryLookup::new();
    let host_tweak: EndpointTweak =
        Arc::new(|b: Builder| loopback_only(b).ca_tls_config(CaTlsConfig::insecure_skip_verify()));
    let guest_tweak: EndpointTweak = if relay {
        Arc::new(|b: Builder| {
            b.clear_ip_transports()
                .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        })
    } else {
        let l = lookup.clone();
        Arc::new(move |b| loopback_only(b).address_lookup(l.clone()))
    };
    let cfg = |tweak: EndpointTweak| {
        let mut c = base_cfg(tweak, vec![]);
        c.relay_override = None;
        c
    };
    let host = side(
        &hub_url,
        cfg(host_tweak),
        FakeEnv {
            name: "Host",
            mods: "m1",
        },
    )
    .await;
    let a = side(
        &hub_url,
        cfg(guest_tweak.clone()),
        FakeEnv {
            name: "Alice",
            mods: "x",
        },
    )
    .await;
    let b = side(
        &hub_url,
        cfg(guest_tweak),
        FakeEnv {
            name: "Bob",
            mods: "y",
        },
    )
    .await;

    // No LAN world and no instance needed.
    let st = host
        .rooms
        .create_voice(password.map(str::to_owned))
        .await
        .unwrap();
    assert_eq!(st.kind, RoomKind::Voice);
    assert_eq!(st.role, RoomRole::Host);
    assert!(st.sig.is_none() && st.local_addr.is_none());
    assert_eq!(st.auth_mode, RoomAuthMode::Open);
    {
        let h = hub.lock().unwrap();
        let body = h.room.as_ref().unwrap();
        assert_eq!(body["kind"], "voice");
        assert!(body.get("sig").is_none());
        assert!(!h.create_had_bearer);
    }
    if relay {
        wait_home_relay(&host.rooms).await;
    } else {
        lookup.add_endpoint_info(host.rooms.endpoint_addr().unwrap());
    }
    for g in [&a, &b] {
        let st = g
            .rooms
            .join("ABCD-1234", None, password.map(str::to_owned))
            .await
            .unwrap();
        assert_eq!(st.kind, RoomKind::Voice);
        assert!(st.sig.is_none() && st.local_addr.is_none());
    }
    eventually("everyone sees everyone", || {
        [&host, &a, &b]
            .iter()
            .all(|s| last_state(&s.rec).is_some_and(|st| st.peers.len() == 2))
    })
    .await;
    (host, a, b, hub)
}

async fn exchange_among_three(host: &Side, a: &Side, b: &Side) {
    let id = |s: &Side| s.rooms.endpoint_addr().unwrap().id.to_string();
    let (host_id, a_id, b_id) = (id(host), id(a), id(b));
    let limit = a.rooms.max_datagram_size();
    assert!((900..=1024).contains(&limit), "{limit}");
    assert_eq!(
        a.rooms
            .send(None, &vec![1u8; limit + 1])
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );

    // Alice lies about her speaker index; the host stamps the real one (1).
    a.rooms.send(None, &frame(9, 1, b"alice")).await.unwrap();
    let (from, data) = recv_dgram(host).await;
    assert_eq!(from, a_id);
    let (h, opus) = VoiceHeader::decode(&data).unwrap();
    assert_eq!((h.speaker, opus), (1, &b"alice"[..]));
    // Bob gets it through the host, attributed to Alice.
    let (from, data) = recv_dgram(b).await;
    assert_eq!(from, a_id);
    assert_eq!(VoiceHeader::decode(&data).unwrap().0.speaker, 1);

    // Full-size frame survives forwarding.
    let big = frame(2, 2, &vec![7u8; limit - VoiceHeader::LEN]);
    b.rooms.send(None, &big).await.unwrap();
    let (from, data) = recv_dgram(a).await;
    assert_eq!((from.as_str(), data.len()), (b_id.as_str(), limit));
    assert_eq!(recv_dgram(host).await.0, b_id);

    // Host broadcast reaches both guests as the host (index 0).
    host.rooms.send(None, &frame(5, 3, b"host")).await.unwrap();
    for g in [a, b] {
        let (from, data) = recv_dgram(g).await;
        assert_eq!(from, host_id);
        assert_eq!(VoiceHeader::decode(&data).unwrap().0.speaker, 0);
    }
    // Unicast host -> Bob; Alice can't address Bob directly.
    host.rooms.send(Some(&b_id), b"only bob").await.unwrap();
    assert_eq!(recv_dgram(b).await.1, b"only bob");
    assert_eq!(
        a.rooms.send(Some(&b_id), b"x").await.unwrap_err().code,
        ErrorCode::NotFound
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn voice_room_direct_three_peers() {
    let (host, a, b, hub) = voice_trio(false, Some("pw")).await;
    exchange_among_three(&host, &a, &b).await;

    // Speaking / muted flags travel over the control stream.
    let a_id = a.rooms.endpoint_addr().unwrap().id.to_string();
    a.rooms.set_voice_state(true, false).await.unwrap();
    eventually("host sees alice speaking", || {
        last_state(&host.rec).is_some_and(|s| {
            s.peers
                .iter()
                .any(|p| p.peer_id == a_id && p.speaking && !p.muted)
        })
    })
    .await;
    // Relayed by the host to the other guest, attributed to Alice.
    eventually("bob sees alice speaking", || {
        last_state(&b.rec).is_some_and(|s| {
            s.peers
                .iter()
                .any(|p| p.peer_id == a_id && p.speaking && !p.muted)
        })
    })
    .await;
    host.rooms.set_voice_state(false, true).await.unwrap();
    eventually("guests see host muted", || {
        [&a, &b].iter().all(|g| {
            last_state(&g.rec).is_some_and(|s| s.peers.iter().any(|p| p.name == "Host" && p.muted))
        })
    })
    .await;

    // Chat works like in game rooms.
    a.rooms.send_chat("hello voice").await.unwrap();
    eventually("chat forwarded to bob", || {
        b.rec
            .chats
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.text == "hello voice" && c.from_name == "Alice")
    })
    .await;

    // Kick bans as usual.
    host.rooms.kick(&a_id).await.unwrap();
    eventually("alice out", || {
        a.rec
            .states
            .lock()
            .unwrap()
            .last()
            .is_some_and(Option::is_none)
    })
    .await;
    let err = a
        .rooms
        .join("ABCD-1234", None, Some("pw".into()))
        .await
        .unwrap_err();
    assert_eq!(err.details.unwrap()["reason"], "banned");

    b.rooms.leave().await.unwrap();
    host.rooms.leave().await.unwrap();
    assert!(hub.lock().unwrap().closed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn voice_room_via_relay_three_peers() {
    let (host, a, b, _hub) = voice_trio(true, None).await;
    eventually("guests on relay", || {
        last_state(&a.rec).is_some_and(|s| {
            s.peers
                .iter()
                .any(|p| p.name == "Host" && p.path == PathKind::Relay)
        })
    })
    .await;
    exchange_among_three(&host, &a, &b).await;
    a.rooms.leave().await.unwrap();
    b.rooms.leave().await.unwrap();
    host.rooms.leave().await.unwrap();
}

/// Regression: a late joiner over relay must end up with the room's current
/// voice states (sent right after its `Welcome`), and speaker indexes are
/// never reused within a room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn voice_late_joiner_gets_states_and_fresh_index() {
    let (host, a, b, _hub) = voice_trio(true, None).await;
    let id = |s: &Side| s.rooms.endpoint_addr().unwrap().id.to_string();
    let a_id = id(&a);
    a.rooms.set_voice_state(true, false).await.unwrap();
    host.rooms.set_voice_state(false, true).await.unwrap();
    eventually("host sees alice speaking", || {
        last_state(&host.rec)
            .is_some_and(|s| s.peers.iter().any(|p| p.peer_id == a_id && p.speaking))
    })
    .await;

    let mut last_index = 1u8;
    for round in 0..30 {
        b.rooms.leave().await.unwrap();
        eventually("host drops bob", || {
            last_state(&host.rec).is_some_and(|s| s.peers.len() == 1)
        })
        .await;
        b.rooms.join("ABCD-1234", None, None).await.unwrap();
        let b_id = id(&b);
        eventually(
            &format!("round {round}: bob sees current voice states"),
            || {
                last_state(&b.rec).is_some_and(|s| {
                    s.peers
                        .iter()
                        .any(|p| p.peer_id == a_id && p.speaking && !p.muted)
                        && s.peers.iter().any(|p| p.name == "Host" && p.muted)
                })
            },
        )
        .await;
        eventually("alice sees bob", || {
            last_state(&a.rec).is_some_and(|s| s.peers.iter().any(|p| p.peer_id == b_id))
        })
        .await;

        // Alice keeps her slot; Bob gets a fresh one on every rejoin.
        a.rooms.send(None, &frame(0, 1, b"a")).await.unwrap();
        let (from, data) = recv_dgram(&b).await;
        assert_eq!(from, a_id);
        assert_eq!(VoiceHeader::decode(&data).unwrap().0.speaker, 1);
        let _ = recv_dgram(&host).await;
        b.rooms.send(None, &frame(0, 1, b"b")).await.unwrap();
        let (from, data) = recv_dgram(&host).await;
        assert_eq!(from, b_id);
        let idx = VoiceHeader::decode(&data).unwrap().0.speaker;
        assert!(idx > last_index, "round {round}: index {idx} reused");
        last_index = idx;
        let (from, _) = recv_dgram(&a).await;
        assert_eq!(from, b_id);
    }
    b.rooms.leave().await.unwrap();
    a.rooms.leave().await.unwrap();
    host.rooms.leave().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn voice_room_password_and_online_host() {
    let (hub_url, hub) = spawn_hub(vec![]).await;
    let settings = Arc::new(MemorySettingsStore::default());
    settings
        .set(Settings {
            hub_url: hub_url.clone(),
            ..Settings::default()
        })
        .await
        .unwrap();
    let downloader = Arc::new(HttpDownloader::new(DownloaderConfig::default()).unwrap());
    let host = P2pRooms::new(
        base_cfg(Arc::new(loopback_only), vec![]),
        downloader,
        settings,
        Arc::new(OnlineEnv),
        Arc::new(Rec::default()),
    );
    let st = host.create_voice(None).await.unwrap();
    assert_eq!(st.auth_mode, RoomAuthMode::Yggdrasil);
    assert_eq!(st.kind, RoomKind::Voice);
    assert!(hub.lock().unwrap().create_had_bearer);
    assert_eq!(
        host.create_voice(None).await.unwrap_err().code,
        ErrorCode::Conflict
    );

    // Offline guest: the room needs a Yggdrasil account.
    let guest = side(
        &hub_url,
        base_cfg(Arc::new(loopback_only), vec![]),
        FakeEnv {
            name: "Guest",
            mods: "m1",
        },
    )
    .await;
    let err = guest.rooms.join("ABCD-1234", None, None).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthRequired);
    host.leave().await.unwrap();
}

// ------------------------------------------- v1.0.1 room-hang regressions

struct TestEnv {
    token: Option<&'static str>,
    delay: Duration,
}

#[async_trait]
impl RoomEnv for TestEnv {
    async fn instance_sig(&self, _instance: InstanceId) -> AppResult<InstanceSig> {
        Ok(InstanceSig {
            mc: "1.21.1".into(),
            loader: LoaderKind::Vanilla,
            loader_version: None,
            mods_hash: "m".into(),
        })
    }

    async fn identity(&self) -> AppResult<Identity> {
        tokio::time::sleep(self.delay).await;
        Ok(Identity {
            name: "Steve".into(),
            uuid: None,
            hub_token: self.token.map(lc_accounts::Secret::new),
        })
    }
}

fn env(token: Option<&'static str>, delay: Duration) -> Arc<dyn RoomEnv> {
    Arc::new(TestEnv { token, delay })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn share_pack_reuses_only_exact_host_instance() {
    let (hub_url, hub) = spawn_hub(vec![]).await;
    let host = side_env(
        &hub_url,
        base_cfg(Arc::new(loopback_only), vec![]),
        env(None, Duration::ZERO),
    )
    .await;
    let instance = InstanceId::new();
    let initial = host
        .rooms
        .create(instance, RoomAuthMode::Open, Some("private".into()))
        .await
        .unwrap();
    let shared = host.rooms.share_pack(instance).await.unwrap();
    assert_eq!(shared.room_id, initial.room_id);
    assert_eq!(shared.auth_mode, RoomAuthMode::Open);
    assert!(shared.password_protected);
    assert_eq!(shared.world_open, Some(false));
    assert_eq!(
        host.rooms
            .share_pack(InstanceId::new())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        host.rooms.state().await.unwrap().unwrap().room_id,
        initial.room_id
    );
    assert!(!hub.lock().unwrap().closed);
    host.rooms.leave().await.unwrap();

    let voice = host.rooms.create_voice(None).await.unwrap();
    assert_eq!(
        host.rooms.share_pack(instance).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        host.rooms.state().await.unwrap().unwrap().room_id,
        voice.room_id
    );
    host.rooms.leave().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn share_pack_preserves_guest_room() {
    let (host, guest, hub, _lookup) = direct_pair(None).await;
    let initial = guest.rooms.join("ABCD-1234", None, None).await.unwrap();
    assert_eq!(
        guest
            .rooms
            .share_pack(InstanceId::new())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let state = guest.rooms.state().await.unwrap().unwrap();
    assert_eq!(state.room_id, initial.room_id);
    assert_eq!(state.role, RoomRole::Guest);
    assert!(!hub.lock().unwrap().closed);
    guest.rooms.leave().await.unwrap();
    host.rooms.leave().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn share_pack_creates_yggdrasil_room_before_lan_only_with_account() {
    let (hub_url, hub) = spawn_hub(vec![]).await;
    let host = side_env(
        &hub_url,
        base_cfg(Arc::new(loopback_only), vec![]),
        env(Some("ygg"), Duration::ZERO),
    )
    .await;
    let instance = InstanceId::new();
    let state = host.rooms.share_pack(instance).await.unwrap();
    assert_eq!(state.kind, RoomKind::Game);
    assert_eq!(state.role, RoomRole::Host);
    assert_eq!(state.auth_mode, RoomAuthMode::Yggdrasil);
    assert_eq!(state.world_open, Some(false));
    assert!(!state.password_protected);
    assert!(hub.lock().unwrap().create_had_bearer);
    assert_eq!(
        host.rooms.share_pack(instance).await.unwrap().room_id,
        state.room_id
    );
    host.rooms.leave().await.unwrap();

    let offline = side_env(
        &hub_url,
        base_cfg(Arc::new(loopback_only), vec![]),
        env(None, Duration::ZERO),
    )
    .await;
    let err = offline.rooms.share_pack(instance).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(err.details.unwrap()["reason"], "authModeMismatch");
    assert!(offline.rooms.state().await.unwrap().is_none());
    assert!(hub.lock().unwrap().closed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn share_pack_serializes_with_room_creation() {
    let (hub_url, hub) = spawn_hub(vec![]).await;
    let host = side_env(
        &hub_url,
        base_cfg(Arc::new(loopback_only), vec![]),
        env(Some("ygg"), Duration::from_millis(100)),
    )
    .await;
    let instance = InstanceId::new();
    let (shared, created) = tokio::join!(
        host.rooms.share_pack(instance),
        host.rooms.create_voice(None),
    );
    let shared = shared.unwrap();
    assert_eq!(created.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(
        host.rooms.state().await.unwrap().unwrap().room_id,
        shared.room_id
    );
    assert!(!hub.lock().unwrap().closed);
    host.rooms.leave().await.unwrap();

    let (created, shared) = tokio::join!(
        host.rooms.create_voice(None),
        host.rooms.share_pack(instance),
    );
    let created = created.unwrap();
    assert_eq!(shared.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(
        host.rooms.state().await.unwrap().unwrap().room_id,
        created.room_id
    );
    assert!(!hub.lock().unwrap().closed);
    host.rooms.leave().await.unwrap();
}

/// Hub refuses a Yggdrasil bearer: the voice room still opens (open mode).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn voice_room_survives_rejected_bearer() {
    let (hub_url, hub) = spawn_hub(vec![]).await;
    hub.lock().unwrap().reject_bearer = true;
    let cfg = base_cfg(Arc::new(loopback_only), vec![]);
    let host = side_env(&hub_url, cfg, env(Some("ygg"), Duration::ZERO)).await;
    let st = host.rooms.create_voice(None).await.unwrap();
    assert_eq!(st.kind, RoomKind::Voice);
    assert_eq!(st.auth_mode, RoomAuthMode::Open);
    assert!(!hub.lock().unwrap().create_had_bearer);
    host.rooms.leave().await.unwrap();

    // A game room can't fall back (the world runs in online mode): clear error.
    host.rooms.set_lan_port(Some(spawn_echo().await));
    let err = host
        .rooms
        .create(InstanceId::new(), RoomAuthMode::Yggdrasil, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthExpired);
    assert_eq!(err.details.unwrap()["source"], "hub");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_identity_times_out() {
    let (hub_url, _hub) = spawn_hub(vec![]).await;
    let cfg = P2pConfig {
        identity_timeout: Duration::from_millis(200),
        ..base_cfg(Arc::new(loopback_only), vec![])
    };
    let host = side_env(&hub_url, cfg, env(None, Duration::from_secs(30))).await;
    let t = std::time::Instant::now();
    let err = host.rooms.create_voice(None).await.unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(5));
    assert_eq!(err.code, ErrorCode::Network);
    let d = err.details.unwrap();
    assert_eq!(d["step"], "identity");
    assert_eq!(d["reason"], "timeout");
    assert!(host.rooms.state().await.unwrap().is_none());
}

/// Relay down: an error instead of a room nobody can reach; hub room closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unreachable_relay_fails_fast() {
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("http://127.0.0.1:{}", dead.local_addr().unwrap().port());
    drop(dead);
    let (hub_url, hub) = spawn_hub(vec![]).await;
    let cfg = P2pConfig {
        online_timeout: Duration::from_secs(2),
        ..base_cfg(Arc::new(loopback_only), vec![relay])
    };
    let host = side_env(&hub_url, cfg, env(None, Duration::ZERO)).await;
    let t = std::time::Instant::now();
    let err = host.rooms.create_voice(None).await.unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(10));
    assert_eq!(err.code, ErrorCode::Network);
    assert_eq!(err.details.unwrap()["reason"], "relayUnreachable");
    assert!(hub.lock().unwrap().closed);
    assert!(host.rooms.state().await.unwrap().is_none());
}
