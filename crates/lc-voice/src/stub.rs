use std::sync::Arc;

use async_trait::async_trait;
use lc_p2p::DatagramTransport;
use lc_types::voice::{AudioDevice, VoiceMode, VoiceSettings};
use lc_types::{AppError, AppResult};

use crate::contract::VoiceEngine;

/// Placeholder: no devices, voice off.
#[derive(Debug, Default)]
pub struct StubVoice;

#[async_trait]
impl VoiceEngine for StubVoice {
    async fn devices(&self) -> AppResult<Vec<AudioDevice>> {
        Ok(Vec::new())
    }

    async fn settings(&self) -> AppResult<VoiceSettings> {
        Ok(VoiceSettings {
            mode: VoiceMode::Off,
            input_device: None,
            output_device: None,
            ptt_key: None,
            input_gain: 1.0,
            output_volume: 1.0,
            noise_suppression: true,
            peer_volumes: Default::default(),
        })
    }

    async fn set_settings(&self, _settings: VoiceSettings) -> AppResult<()> {
        Err(AppError::not_implemented("VoiceEngine::set_settings"))
    }

    async fn start(&self, _transport: Arc<dyn DatagramTransport>) -> AppResult<()> {
        Err(AppError::not_implemented("VoiceEngine::start"))
    }

    async fn stop(&self) -> AppResult<()> {
        Ok(())
    }

    async fn set_peer_muted(&self, _peer_id: &str, _muted: bool) -> AppResult<()> {
        Err(AppError::not_implemented("VoiceEngine::set_peer_muted"))
    }

    async fn set_peer_volume(&self, _peer_id: &str, _volume: f32) -> AppResult<()> {
        Err(AppError::not_implemented("VoiceEngine::set_peer_volume"))
    }

    async fn set_self_muted(&self, _muted: bool) -> AppResult<()> {
        Err(AppError::not_implemented("VoiceEngine::set_self_muted"))
    }

    async fn mic_test(&self, _enabled: bool) -> AppResult<()> {
        Err(AppError::not_implemented("VoiceEngine::mic_test"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_voice_is_off() {
        assert_eq!(StubVoice.settings().await.unwrap().mode, VoiceMode::Off);
    }
}
