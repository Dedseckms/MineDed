//! Owner: net. P2P rooms and text chat.

use lc_types::room::{RoomAuthMode, RoomState};
use lc_types::task::{TaskEvent, TaskKind};
use lc_types::voice::ChatMessage;
use lc_types::{AppResult, InstanceId, TaskId};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::tasks::spawn_task;

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
    instance: Option<InstanceId>,
    password: Option<String>,
) -> AppResult<RoomState> {
    state.rooms.join(&code, instance, password).await
}

#[tauri::command]
#[specta::specta]
pub async fn room_create_voice(
    state: State<'_, AppState>,
    password: Option<String>,
) -> AppResult<RoomState> {
    state.rooms.create_voice(password).await
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

/// Guest of a game room: downloads the host's modpack, imports it as a new
/// instance and attaches the room to it (server-list entry, quick play).
#[tauri::command]
#[specta::specta]
pub async fn room_download_host_pack(
    app: AppHandle,
    state: State<'_, AppState>,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let rooms = state.rooms.clone();
    let packs = state.packs.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::InstanceImport,
        "tasks.instanceImport",
        on_event,
        move |sink| async move {
            let path = rooms.download_host_pack(&*sink).await?;
            let imported = packs.import(&path, &*sink).await;
            let _ = std::fs::remove_file(&path);
            rooms.attach_instance(imported?.id).await.map(|_| ())
        },
    ))
}

/// Guest: binds the joined game room to `instance`.
#[tauri::command]
#[specta::specta]
pub async fn room_attach_instance(
    state: State<'_, AppState>,
    instance: InstanceId,
) -> AppResult<RoomState> {
    state.rooms.attach_instance(instance).await
}
