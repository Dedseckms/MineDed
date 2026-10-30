//! Headless rooms smoke test (net): the real `AppState` wiring against the
//! live hub. Offline account, voice room, then a game room over a fake LAN
//! world (TCP listener + multicast announcement, like "Open to LAN").
//!
//! `cargo run -p desktop --example rooms_smoke [-- --creds <user:password file>]`
//! Uses a throwaway data dir under the system temp folder.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use desktop_lib::state::AppState;
use lc_core::GameObserver;
use lc_runtime::{CancelToken, JsonSettingsStore, Paths, ProgressSink, SettingsStore};
use lc_types::events::{InstanceLogPayload, InstanceStatePayload};
use lc_types::instance::{InstanceSpec, LoaderRef};
use lc_types::room::RoomAuthMode;
use lc_types::task::TaskEvent;
use lc_types::version::LoaderKind;
use lc_types::TaskId;

struct Obs;

impl lc_chat::SocialObserver for Obs {
    fn on_event(&self, _: lc_types::social::SocialEvent) {}
}

impl GameObserver for Obs {
    fn on_state(&self, _p: InstanceStatePayload) {}
    fn on_log(&self, _p: InstanceLogPayload) {}
}

impl lc_p2p::RoomObserver for Obs {
    fn on_state(&self, state: Option<lc_types::room::RoomState>) {
        println!(
            "  [room] {:?}",
            state.map(|s| (s.code, s.kind, s.peers.len()))
        );
    }
    fn on_peer(&self, _peer: lc_types::room::Peer, _joined: bool) {}
    fn on_chat(&self, _message: lc_types::voice::ChatMessage) {}
}

impl lc_voice::VoiceObserver for Obs {
    fn on_speaking(&self, _payload: lc_types::events::VoiceSpeakingPayload) {}
}

struct NoSink(TaskId, CancelToken);

impl ProgressSink for NoSink {
    fn task_id(&self) -> TaskId {
        self.0
    }
    fn emit(&self, _event: TaskEvent) {}
    fn cancel_token(&self) -> &CancelToken {
        &self.1
    }
}

async fn timed<T>(label: &str, f: impl std::future::Future<Output = T>) -> T {
    let t = Instant::now();
    let r = f.await;
    println!("  {label}: {} ms", t.elapsed().as_millis());
    r
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join("mineup-rooms-smoke");
    let store: Arc<dyn SettingsStore> =
        Arc::new(JsonSettingsStore::new(root.join("settings.json")));
    let settings = store.get().await?;
    println!("hub: {}", settings.hub_url);
    let obs = Arc::new(Obs);
    let state = AppState::real(
        Paths::new(&root),
        store,
        &settings,
        obs.clone(),
        obs.clone(),
        obs.clone(),
        obs,
    )?;
    // `--creds <file>` with `user:password`: a Yggdrasil account on the own
    // server becomes the default (removed again at the end). Never printed.
    let args: Vec<String> = std::env::args().collect();
    let creds = args
        .iter()
        .position(|a| a == "--creds")
        .and_then(|i| args.get(i + 1));
    let mut ygg = None;
    let mut mode = RoomAuthMode::Open;
    if let Some(path) = creds {
        let raw = std::fs::read_to_string(path)?;
        let (user, pass) = raw.trim().split_once(':').ok_or("creds: user:password")?;
        let root = url::Url::parse("https://skins.dedseckms.ru/authlib-injector")?;
        let acc = timed(
            "yggdrasil_login",
            state.accounts.yggdrasil_login(&root, user, pass),
        )
        .await?;
        state.accounts.set_default(acc.id).await?;
        ygg = Some(acc.id);
        mode = RoomAuthMode::Yggdrasil;
    } else if !state
        .accounts
        .list()
        .await?
        .iter()
        .any(|a| a.name == "RoomSmoke")
    {
        state.accounts.offline_add("RoomSmoke").await?;
    }
    let auth = timed("launch_auth", state.auth.launch_auth(None)).await;
    println!(
        "  identity: {:?}",
        auth.map(|a| (a.name, a.user_type)).map_err(|e| e.code)
    );

    let hub = state.downloader.client(lc_http::Route::Direct);
    let r = timed(
        "hub GET /mirrors",
        hub.get(format!("{}/mirrors", settings.hub_url)).send(),
    )
    .await;
    println!("  -> {:?}", r.map(|r| r.status()));

    println!("== voice room ==");
    let r = timed(
        "create_voice",
        tokio::time::timeout(Duration::from_secs(90), state.rooms.create_voice(None)),
    )
    .await;
    report(&r);
    timed("leave", state.rooms.leave()).await?;

    println!("== game room ==");
    let inst = match state.instances.list().await?.into_iter().next() {
        Some(i) => i,
        None => {
            // Creating installs the version; the room only needs the record.
            let spec = InstanceSpec {
                name: "Room smoke".into(),
                mc: "1.21.1".into(),
                loader: LoaderRef {
                    kind: LoaderKind::Vanilla,
                    version: None,
                },
                preset: None,
                icon: None,
            };
            let sink = NoSink(TaskId::new(), CancelToken::new());
            timed("instance create", state.instances.create(spec, &sink)).await?
        }
    };
    // Fake integrated server: accepts TCP, announces itself like the game.
    let lan = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    let port = lan.local_addr()?.port();
    tokio::spawn(async move {
        while let Ok((s, _)) = lan.accept().await {
            drop(s);
        }
    });
    tokio::spawn(async move {
        let Ok(sock) = tokio::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await else {
            return;
        };
        let _ = sock.set_multicast_loop_v4(true);
        let pkt = format!("[MOTD]Smoke - World[/MOTD][AD]{port}[/AD]");
        loop {
            let _ = sock
                .send_to(
                    pkt.as_bytes(),
                    SocketAddrV4::new(Ipv4Addr::new(224, 0, 2, 60), 4445),
                )
                .await;
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    });
    println!("  fake LAN world on port {port}");
    let r = timed(
        "create (game)",
        tokio::time::timeout(
            Duration::from_secs(150),
            state.rooms.create(inst.id, mode, None),
        ),
    )
    .await;
    report(&r);
    timed("leave", state.rooms.leave()).await?;
    if let Some(id) = ygg {
        state.accounts.remove(id).await?;
    }
    Ok(())
}

fn report<T: std::fmt::Debug>(r: &Result<lc_types::AppResult<T>, tokio::time::error::Elapsed>) {
    match r {
        Err(_) => println!("  RESULT: HANG (outer timeout)"),
        Ok(Err(e)) => println!(
            "  RESULT: error {:?}: {} {:?}",
            e.code, e.message, e.details
        ),
        Ok(Ok(_)) => println!("  RESULT: OK"),
    }
}
