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
    make_peer_token, DatagramTransport, EndpointTweak, Identity, P2pConfig, P2pRooms, RoomEnv,
    RoomObserver, RoomService,
};
use lc_runtime::{LaunchPlan, MemorySettingsStore, SettingsStore};
use lc_types::instance::LaunchOpts;
use lc_types::room::{InstanceSig, PathKind, Peer, RoomAuthMode, RoomRole, RoomState};
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
    let mut room = body.clone();
    room["roomId"] = json!(room_id);
    h.room = Some(room);
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
    for _ in 0..200 {
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
    let rooms = P2pRooms::new(cfg, downloader, settings, Arc::new(env), rec.clone());
    Side { rooms, rec }
}

fn base_cfg(tweak: EndpointTweak, relays: Vec<String>) -> P2pConfig {
    P2pConfig {
        relay_override: Some(relays),
        lan_wait: Duration::from_secs(5),
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
    tokio::time::timeout(Duration::from_secs(10), tcp.read_exact(&mut back))
        .await
        .expect("echo timed out")
        .unwrap();
    back
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
    assert_eq!(st.link, "mineup://join/ABCD-1234");
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
        .join("mineup://join/abcd1234", InstanceId::new(), None)
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
        .join("ABCD-1234", InstanceId::new(), None)
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
        .join("ABCD-1234", InstanceId::new(), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);

    let err = guest
        .rooms
        .join("ABCD-1234", InstanceId::new(), Some("wrong".into()))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied, "{err:?}");
    assert_eq!(err.details.unwrap()["reason"], "bad_password");
    assert!(guest.rooms.state().await.unwrap().is_none());
    assert!(host.rooms.state().await.unwrap().unwrap().peers.is_empty());

    let st = guest
        .rooms
        .join("ABCD-1234", InstanceId::new(), Some("open sesame".into()))
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
            .join("ABCD-1234", InstanceId::new(), Some("guess".into()))
            .await
            .unwrap_err();
        assert_eq!(err.details.unwrap()["reason"], "bad_password");
    }
    let err = guest
        .rooms
        .join("ABCD-1234", InstanceId::new(), Some("open sesame".into()))
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
        .join("ABCD-1234", InstanceId::new(), None)
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
        .join("ABCD-1234", InstanceId::new(), None)
        .await
        .unwrap();
    let local = st.local_addr.unwrap();
    assert_eq!(echo_through(&local, b"secret").await, b"secret");
    guest.rooms.leave().await.unwrap();
    host.rooms.leave().await.unwrap();
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

    // No LAN world yet.
    let err = host
        .rooms
        .create(InstanceId::new(), RoomAuthMode::Open, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

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
        .join("ABCD-1234", InstanceId::new(), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    let err = guest
        .rooms
        .join("ABC!-123", InstanceId::new(), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    let err = guest
        .rooms
        .join("ZZZZ-9999", InstanceId::new(), None)
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

    let st = guest
        .rooms
        .join("ABCD-1234", InstanceId::new(), None)
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
