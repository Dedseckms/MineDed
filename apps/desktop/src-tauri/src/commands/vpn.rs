//! Owner: net.

use lc_types::task::{TaskEvent, TaskKind};
use lc_types::vpn::{VpnMode, VpnNode, VpnState};
use lc_types::{AppResult, TaskId};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::tasks::spawn_task;

#[tauri::command]
#[specta::specta]
pub async fn vpn_state(state: State<'_, AppState>) -> AppResult<VpnState> {
    state.vpn.state().await
}

#[tauri::command]
#[specta::specta]
pub async fn vpn_set_subscription(state: State<'_, AppState>, urls: Vec<String>) -> AppResult<()> {
    state.vpn.set_subscription(urls).await
}

#[tauri::command]
#[specta::specta]
pub async fn vpn_refresh(
    app: AppHandle,
    state: State<'_, AppState>,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let vpn = state.vpn.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::VpnUpdate,
        "tasks.vpnUpdate",
        on_event,
        move |sink| async move { vpn.refresh(&*sink).await },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn vpn_set_mode(state: State<'_, AppState>, mode: VpnMode) -> AppResult<VpnState> {
    state.vpn.set_mode(mode).await
}

#[tauri::command]
#[specta::specta]
pub async fn vpn_nodes(state: State<'_, AppState>) -> AppResult<Vec<VpnNode>> {
    state.vpn.nodes().await
}

#[tauri::command]
#[specta::specta]
pub async fn vpn_select(state: State<'_, AppState>, node: String) -> AppResult<()> {
    state.vpn.select(&node).await
}

#[tauri::command]
#[specta::specta]
pub async fn vpn_test_latency(state: State<'_, AppState>) -> AppResult<Vec<VpnNode>> {
    state.vpn.test_latency().await
}
