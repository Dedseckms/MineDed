//! CONTRACT (lead). `RoomService` is used by `src-tauri`; `DatagramTransport`
//! is consumed by `lc-voice`. The implementation also provides a
//! `LaunchContributor` (quick play target for guests, authlib for the host).

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::room::{Peer, RoomAuthMode, RoomState};
use lc_types::voice::ChatMessage;
use lc_types::{AppResult, InstanceId};

#[async_trait]
pub trait RoomService: Send + Sync {
    /// Registers a room in hub and starts the iroh endpoint without waiting for
    /// "Open to LAN": the LAN port is detected in the background
    /// (`RoomState::world_open`). `password` is checked by the host over P2P,
    /// never sent to hub.
    async fn create(
        &self,
        instance: InstanceId,
        auth_mode: RoomAuthMode,
        password: Option<String>,
    ) -> AppResult<RoomState>;
    /// `code` is a room code or a `mineded://join/<code>` link. Validates
    /// `InstanceSig` against the local instance, connects to the host and opens
    /// a local TCP port (`RoomState::local_addr`) tunnelled to the host's world.
    /// `instance` is required for game rooms and ignored for voice rooms.
    async fn join(
        &self,
        code: &str,
        instance: Option<InstanceId>,
        password: Option<String>,
    ) -> AppResult<RoomState>;
    /// Voice/text-only room (`RoomKind::Voice`) for players on a regular
    /// server: no game tunnel, no instance check. Auth follows the account
    /// (Yggdrasil bearer if available, else anonymous like open rooms).
    async fn create_voice(&self, password: Option<String>) -> AppResult<RoomState>;
    async fn leave(&self) -> AppResult<()>;
    /// Guest of a game room: asks the host for its instance as `.mrpack`
    /// (exported by the host on request and sent over P2P; Modrinth files as
    /// links, local mods/configs as overrides). Returns the downloaded file path
    /// for `PackIo::import`. Game rooms may be joined with `instance: None`
    /// for this; call `attach_instance` before launching.
    async fn download_host_pack(&self, sink: &dyn ProgressSink) -> AppResult<String>;
    /// Guest: binds the joined game room to `instance` (e.g. the imported host
    /// pack) and adds the server-list entry to it.
    async fn attach_instance(&self, instance: InstanceId) -> AppResult<RoomState>;
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
    /// Publishes our own speaking/muted state to the room (`ControlMsg::VoiceState`).
    /// Default: ignored (test doubles).
    async fn set_voice_state(&self, _speaking: bool, _muted: bool) -> AppResult<()> {
        Ok(())
    }
}

/// Room events; implemented in `src-tauri` (emits `room://*`, `chat://message`).
pub trait RoomObserver: Send + Sync {
    fn on_state(&self, state: Option<RoomState>);
    fn on_peer(&self, peer: Peer, joined: bool);
    fn on_chat(&self, message: ChatMessage);
}
