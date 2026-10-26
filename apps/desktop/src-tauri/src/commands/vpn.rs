//! Owner: net.

use lc_types::account::AccountKind;
use lc_types::task::{TaskEvent, TaskKind};
use lc_types::vpn::{VpnAccess, VpnApp, VpnMode, VpnNode, VpnSource, VpnState};
use lc_types::{AccountId, AppError, AppResult, ErrorCode, TaskId};
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
pub async fn vpn_import_subscription_file(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<()> {
    state.vpn.import_subscription_file(&path).await
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

/// Switches between the MineDed and the user's own subscription (both kept);
/// reconnects a running VPN.
#[tauri::command]
#[specta::specta]
pub async fn vpn_set_source(state: State<'_, AppState>, source: VpnSource) -> AppResult<VpnState> {
    state.vpn.set_source(source).await
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

fn same_api_root(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches('/').to_ascii_lowercase();
    norm(a) == norm(b)
}

/// MineDed VPN (hub `/me/vpn`) for an own-server Yggdrasil account.
/// `provision` creates the key when missing; `null` = no key yet.
#[tauri::command]
#[specta::specta]
pub async fn vpn_mineded_access(
    state: State<'_, AppState>,
    account_id: AccountId,
    provision: bool,
) -> AppResult<Option<VpnAccess>> {
    let needs_own = || {
        AppError::new(
            ErrorCode::InvalidInput,
            "MineDed VPN needs an own-server account",
        )
        .detail("reason", "needsOwnServerAccount")
    };
    let accounts = state.accounts.list().await?;
    let account = accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "account not found"))?;
    if account.kind != AccountKind::Yggdrasil {
        return Err(needs_own());
    }
    let servers = state.accounts.yggdrasil_servers().await?;
    let own = account.server.as_deref().is_some_and(|root| {
        servers
            .iter()
            .any(|s| s.official && same_api_root(&s.api_root, root))
    });
    if !own {
        return Err(needs_own());
    }
    let auth = state.auth.launch_auth(Some(account_id)).await?;
    state
        .vpn
        .mineded_access(auth.access_token.expose(), provision)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn vpn_apps(state: State<'_, AppState>) -> AppResult<Vec<VpnApp>> {
    state.vpn.apps().await
}

#[tauri::command]
#[specta::specta]
pub async fn vpn_set_apps(state: State<'_, AppState>, apps: Vec<VpnApp>) -> AppResult<()> {
    state.vpn.set_apps(apps).await
}

#[tauri::command]
#[specta::specta]
pub async fn vpn_running_apps(state: State<'_, AppState>) -> AppResult<Vec<VpnApp>> {
    state.vpn.running_apps().await
}

#[cfg(test)]
mod tests {
    #[test]
    fn api_roots_compare_loosely() {
        assert!(super::same_api_root(
            "https://mc.example.ru/authserver/",
            " https://MC.example.ru/authserver"
        ));
        assert!(!super::same_api_root(
            "https://ely.by/api",
            "https://mc.example.ru/api"
        ));
    }
}
