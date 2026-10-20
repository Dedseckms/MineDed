//! CONTRACT (lead).

use std::sync::Arc;

use async_trait::async_trait;
use lc_p2p::DatagramTransport;
use lc_types::events::VoiceSpeakingPayload;
use lc_types::voice::{AudioDevice, VoiceSettings};
use lc_types::AppResult;

#[async_trait]
pub trait VoiceEngine: Send + Sync {
    async fn devices(&self) -> AppResult<Vec<AudioDevice>>;
    async fn settings(&self) -> AppResult<VoiceSettings>;
    /// Applies devices, mode, PTT key, gains. Re-registers the global hotkey.
    async fn set_settings(&self, settings: VoiceSettings) -> AppResult<()>;
    /// Starts capture/playback over the room transport.
    async fn start(&self, transport: Arc<dyn DatagramTransport>) -> AppResult<()>;
    async fn stop(&self) -> AppResult<()>;
    /// Local mute of a remote peer.
    async fn set_peer_muted(&self, peer_id: &str, muted: bool) -> AppResult<()>;
    async fn set_self_muted(&self, muted: bool) -> AppResult<()>;
    /// Plays the microphone back locally (settings screen).
    async fn mic_test(&self, enabled: bool) -> AppResult<()>;
}

/// Implemented in `src-tauri` (emits `voice://speaking`, throttled).
pub trait VoiceObserver: Send + Sync {
    fn on_speaking(&self, payload: VoiceSpeakingPayload);
}
