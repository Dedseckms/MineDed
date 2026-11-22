//! Deterministic backpressure and stale connection cleanup regressions.
use super::*;
use lc_http::{DownloaderConfig, HttpDownloader};
use lc_runtime::MemorySettingsStore;

struct Env;
#[async_trait]
impl RoomEnv for Env {
    async fn identity(&self) -> AppResult<Identity> {
        unreachable!()
    }
    async fn instance_sig(&self, _: InstanceId) -> AppResult<InstanceSig> {
        unreachable!()
    }
}

#[derive(Default)]
struct Observer {
    chats: StdMutex<Vec<ChatMessage>>,
}
impl RoomObserver for Observer {
    fn on_state(&self, _: Option<RoomState>) {}
    fn on_peer(&self, _: Peer, _: bool) {}
    fn on_chat(&self, chat: ChatMessage) {
        lock(&self.chats).push(chat);
    }
}

async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .bind()
        .await
        .unwrap()
}

async fn connect(host: &Endpoint, guest: &Endpoint) -> (Connection, Connection) {
    let accept = async { host.accept().await.unwrap().await.unwrap() };
    let (host_conn, guest_conn) = tokio::join!(accept, guest.connect(host.addr(), ALPN));
    (host_conn, guest_conn.unwrap())
}

fn entry(conn: Connection, tx: control::Sender) -> PeerEntry {
    PeerEntry {
        peer: Peer {
            peer_id: conn.remote_id().to_string(),
            name: "guest".into(),
            uuid: None,
            path: PathKind::Direct,
            rtt_ms: None,
            speaking: false,
            muted: false,
        },
        conn: Some(conn),
        tx: Some(tx),
        ban_uuid: None,
    }
}

fn session(endpoint: Endpoint) -> Arc<Session> {
    let my_id = endpoint.id().to_string();
    let (dgram_tx, dgram_rx) = mpsc::channel(DGRAM_QUEUE);
    Arc::new(Session {
        role: RoomRole::Host,
        room_id: RoomId::new(),
        code: "ABCD-1234".into(),
        auth_mode: RoomAuthMode::Open,
        kind: RoomKind::Voice,
        sig: None,
        expires_at: String::new(),
        password_protected: false,
        local_addr: None,
        servers_dat: StdMutex::new(None),
        my_id: my_id.clone(),
        my_name: "Host".into(),
        endpoint,
        cancel: CancellationToken::new(),
        peers: StdMutex::new(Peers {
            order: vec![my_id],
            map: BTreeMap::new(),
        }),
        dgram_tx,
        dgram_rx: tokio::sync::Mutex::new(dgram_rx),
        voice: StdMutex::new((false, false)),
        host: None,
    })
}

#[tokio::test]
async fn control_writer_failure_only_closes_its_own_connection() {
    let host = endpoint().await;
    let a = endpoint().await;
    let b = endpoint().await;
    let (failed, remote) = connect(&host, &a).await;
    let (healthy, _healthy_remote) = connect(&host, &b).await;
    let (mut send, _recv) = failed.open_bi().await.unwrap();
    send.write_all(b"hello\n").await.unwrap();
    let (_remote_send, mut remote_recv) = remote.accept_bi().await.unwrap();
    remote_recv.stop(1u32.into()).unwrap();
    let (tx, rx) = control::channel();
    spawn_writer(failed.clone(), send, rx);
    // The write can initially succeed before STOP_SENDING is delivered;
    // continue yielding until the writer observes the peer's stream reset.
    tokio::time::timeout(Duration::from_secs(5), async {
        while failed.close_reason().is_none() {
            let _ = tx.try_send("control\n".into());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(healthy.close_reason().is_none());
    a.close().await;
    b.close().await;
    host.close().await;
}

#[tokio::test]
async fn full_peer_removal_does_not_cascade_to_two_healthy_peers() {
    let host = endpoint().await;
    let observer = Arc::new(Observer::default());
    let service = P2pRooms::new(
        P2pConfig::default(),
        Arc::new(HttpDownloader::new(DownloaderConfig::default()).unwrap()),
        Arc::new(MemorySettingsStore::default()),
        Arc::new(Env),
        observer.clone(),
    );
    let s = session(host.clone());
    service.inner.set_current(Some(s.clone()));
    let mut endpoints = Vec::new();
    let mut connections = Vec::new();
    let mut receivers = Vec::new();
    for _ in 0..3 {
        let guest = endpoint().await;
        let (conn, remote) = connect(&host, &guest).await;
        let id = guest.id().to_string();
        let (tx, rx) = control::channel();
        s.peers().map.insert(id.clone(), entry(conn.clone(), tx));
        s.peers().assign_slot(&id);
        endpoints.push(guest);
        connections.push((conn, remote));
        receivers.push(rx);
    }
    // Pause writers deterministically. Healthy FIFOs can briefly fill too
    // while one network burst is being processed.
    let chat = wire::encode(&ControlMsg::Chat {
        id: "queued".into(),
        text: "chat".into(),
        sent_at: String::new(),
        from: None,
        from_name: None,
    })
    .unwrap();
    for _ in 0..256 {
        s.broadcast(&chat, None);
    }
    for _ in 0..1000 {
        service.host_broadcast_welcome(&s).unwrap();
    }
    assert!(connections.iter().all(|(c, _)| c.close_reason().is_none()));
    // Only the persistently stalled peer exceeds its reliable budget.
    let slow = endpoints[0].id().to_string();
    s.send_to(&slow, &chat);
    assert!(connections[0].0.close_reason().is_some());
    service
        .host_remove_peer(&s, &slow, &connections[0].0)
        .unwrap();
    assert!(!s.cancel.is_cancelled());
    assert_eq!(s.state().peers.len(), 2);
    for (i, rx) in receivers.iter_mut().enumerate().skip(1) {
        assert!(connections[i].0.close_reason().is_none());
        for _ in 0..256 {
            assert_eq!(rx.recv().await.unwrap(), chat);
        }
        let ControlMsg::Welcome { peers, .. } = wire::decode(&rx.recv().await.unwrap()).unwrap()
        else {
            panic!("welcome missing")
        };
        assert!(!peers.contains(&slow));
    }
    let healthy = endpoints[1].id().to_string();
    assert!(service.on_control(&s, &healthy, ControlMsg::Ping { nonce: 7 }));
    assert_eq!(
        wire::decode(&receivers[1].recv().await.unwrap()).unwrap(),
        ControlMsg::Pong { nonce: 7 }
    );
    assert!(service.on_control(&s, &healthy, wire::decode(&chat).unwrap()));
    assert!(matches!(
        wire::decode(&receivers[2].recv().await.unwrap()).unwrap(),
        ControlMsg::Chat { .. }
    ));
    assert_eq!(lock(&observer.chats).len(), 1);

    // Delayed cleanup of the old connection must preserve the replacement.
    let (new_conn, _new_remote) = connect(&host, &endpoints[0]).await;
    let (tx, _rx) = control::channel();
    s.peers()
        .map
        .insert(slow.clone(), entry(new_conn.clone(), tx));
    s.peers().assign_slot(&slow);
    let slot = s.speaker_index(&slow);
    service
        .host_remove_peer(&s, &slow, &connections[0].0)
        .unwrap();
    assert_eq!(s.speaker_index(&slow), slot);
    assert_eq!(s.state().peers.len(), 3);
    assert!(new_conn.close_reason().is_none());
    assert!(!s.cancel.is_cancelled());
    let newer = session(host.clone());
    service.inner.set_current(Some(newer.clone()));
    assert!(!service.inner.take_if(&s));
    assert!(!newer.cancel.is_cancelled());
    for ep in endpoints {
        ep.close().await;
    }
    host.close().await;
}
