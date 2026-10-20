//! Owner: accounts. Tokens never cross IPC: only `Account` DTOs do.

use lc_types::account::{Account, DeviceCode, YggdrasilServer};
use lc_types::task::{TaskEvent, TaskKind};
use lc_types::{AccountId, AppError, AppResult, ErrorCode, TaskId};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use url::Url;

use crate::state::AppState;
use crate::tasks::spawn_task;

#[tauri::command]
#[specta::specta]
pub async fn account_list(state: State<'_, AppState>) -> AppResult<Vec<Account>> {
    state.accounts.list().await
}

#[tauri::command]
#[specta::specta]
pub async fn account_msa_begin(state: State<'_, AppState>) -> AppResult<DeviceCode> {
    state.accounts.msa_begin().await
}

/// Polls Microsoft until the user confirms the device code.
#[tauri::command]
#[specta::specta]
pub async fn account_msa_complete(
    app: AppHandle,
    state: State<'_, AppState>,
    flow_id: String,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.accounts.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::AccountLogin,
        "tasks.accountLogin",
        on_event,
        move |sink| async move { svc.msa_complete(&flow_id, &*sink).await.map(|_| ()) },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn account_offline_add(state: State<'_, AppState>, name: String) -> AppResult<Account> {
    state.accounts.offline_add(&name).await
}

#[tauri::command]
#[specta::specta]
pub async fn account_yggdrasil_login(
    state: State<'_, AppState>,
    api_root: String,
    username: String,
    password: String,
) -> AppResult<Account> {
    let api_root = Url::parse(&api_root)
        .map_err(|e| AppError::new(ErrorCode::InvalidInput, format!("api_root: {e}")))?;
    state
        .accounts
        .yggdrasil_login(&api_root, &username, &password)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn account_yggdrasil_servers(
    state: State<'_, AppState>,
) -> AppResult<Vec<YggdrasilServer>> {
    state.accounts.yggdrasil_servers().await
}

#[tauri::command]
#[specta::specta]
pub async fn account_remove(state: State<'_, AppState>, id: AccountId) -> AppResult<()> {
    state.accounts.remove(id).await
}

#[tauri::command]
#[specta::specta]
pub async fn account_set_default(state: State<'_, AppState>, id: AccountId) -> AppResult<()> {
    state.accounts.set_default(id).await
}
