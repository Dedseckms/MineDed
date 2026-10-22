//! Owner: voice.

use lc_types::voice::{AudioDevice, VoiceSettings};
use lc_types::AppResult;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn voice_devices(state: State<'_, AppState>) -> AppResult<Vec<AudioDevice>> {
    state.voice.devices().await
}

#[tauri::command]
#[specta::specta]
pub async fn voice_settings(state: State<'_, AppState>) -> AppResult<VoiceSettings> {
    state.voice.settings().await
}

#[tauri::command]
#[specta::specta]
pub async fn voice_set_settings(
    state: State<'_, AppState>,
    settings: VoiceSettings,
) -> AppResult<()> {
    state.voice.set_settings(settings).await
}

/// Joins voice of the current room.
#[tauri::command]
#[specta::specta]
pub async fn voice_start(state: State<'_, AppState>) -> AppResult<()> {
    state.voice.start(state.datagrams.clone()).await
}

#[tauri::command]
#[specta::specta]
pub async fn voice_stop(state: State<'_, AppState>) -> AppResult<()> {
    state.voice.stop().await
}

#[tauri::command]
#[specta::specta]
pub async fn voice_set_peer_muted(
    state: State<'_, AppState>,
    peer_id: String,
    muted: bool,
) -> AppResult<()> {
    state.voice.set_peer_muted(&peer_id, muted).await
}

#[tauri::command]
#[specta::specta]
pub async fn voice_status(state: State<'_, AppState>) -> AppResult<lc_types::voice::VoiceStatus> {
    state.voice.status().await
}

#[tauri::command]
#[specta::specta]
pub async fn voice_set_peer_volume(
    state: State<'_, AppState>,
    peer_id: String,
    volume: f32,
) -> AppResult<()> {
    state.voice.set_peer_volume(&peer_id, volume).await
}

#[tauri::command]
#[specta::specta]
pub async fn voice_set_self_muted(state: State<'_, AppState>, muted: bool) -> AppResult<()> {
    state.voice.set_self_muted(muted).await
}

#[tauri::command]
#[specta::specta]
pub async fn voice_mic_test(state: State<'_, AppState>, enabled: bool) -> AppResult<()> {
    state.voice.mic_test(enabled).await
}
