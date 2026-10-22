//! Owner: net. P2P rooms and text chat.

use lc_types::room::{RoomAuthMode, RoomState};
use lc_types::voice::ChatMessage;
use lc_types::{AppResult, InstanceId};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn room_create(
    state: State<'_, AppState>,
    instance: InstanceId,
    auth_mode: RoomAuthMode,
    password: Option<String>,
) -> AppResult<RoomState> {
    state.rooms.create(instance, auth_mode, password).await
}

#[tauri::command]
#[specta::specta]
pub async fn room_join(
    state: State<'_, AppState>,
    code: String,
    instance: InstanceId,
    password: Option<String>,
) -> AppResult<RoomState> {
    state.rooms.join(&code, instance, password).await
}

#[tauri::command]
#[specta::specta]
pub async fn room_leave(state: State<'_, AppState>) -> AppResult<()> {
    state.rooms.leave().await
}

#[tauri::command]
#[specta::specta]
pub async fn room_state(state: State<'_, AppState>) -> AppResult<Option<RoomState>> {
    state.rooms.state().await
}

#[tauri::command]
#[specta::specta]
pub async fn room_kick(state: State<'_, AppState>, peer_id: String) -> AppResult<()> {
    state.rooms.kick(&peer_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn chat_send(state: State<'_, AppState>, text: String) -> AppResult<ChatMessage> {
    state.rooms.send_chat(&text).await
}
