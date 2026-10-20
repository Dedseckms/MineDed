//! CONTRACT (lead). `RoomService` is used by `src-tauri`; `DatagramTransport`
//! is consumed by `lc-voice`. The implementation also provides a
//! `LaunchContributor` (quick play target for guests, authlib for the host).

use async_trait::async_trait;
use lc_types::room::{Peer, RoomAuthMode, RoomState};
use lc_types::voice::ChatMessage;
use lc_types::{AppResult, InstanceId};

#[async_trait]
pub trait RoomService: Send + Sync {
    /// Registers a room in hub, starts the iroh endpoint, waits for the LAN port.
    async fn create(&self, instance: InstanceId, auth_mode: RoomAuthMode) -> AppResult<RoomState>;
    /// Validates `InstanceSig` against the local instance, connects to the host.
    async fn join(&self, code: &str, instance: InstanceId) -> AppResult<RoomState>;
    async fn leave(&self) -> AppResult<()>;
    async fn state(&self) -> AppResult<Option<RoomState>>;
    /// Host only.
    async fn kick(&self, peer_id: &str) -> AppResult<()>;
    /// Over the P2P control stream, hub WebSocket as fallback.
    async fn send_chat(&self, text: &str) -> AppResult<ChatMessage>;
}

/// Unreliable datagrams to room peers (QUIC datagrams), used for voice.
#[async_trait]
pub trait DatagramTransport: Send + Sync {
    /// `None` = broadcast to all peers (host forwards when > mesh limit).
    async fn send(&self, to: Option<&str>, payload: &[u8]) -> AppResult<()>;
    /// Next datagram: (peer_id, payload). `None` once the room is closed.
    async fn recv(&self) -> Option<(String, Vec<u8>)>;
    /// Max payload size for the current path.
    fn max_datagram_size(&self) -> usize;
}

/// Room events; implemented in `src-tauri` (emits `room://*`, `chat://message`).
pub trait RoomObserver: Send + Sync {
    fn on_state(&self, state: Option<RoomState>);
    fn on_peer(&self, peer: Peer, joined: bool);
    fn on_chat(&self, message: ChatMessage);
}
