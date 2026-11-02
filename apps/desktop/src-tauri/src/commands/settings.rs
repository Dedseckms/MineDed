//! Owner: lead.

use lc_types::settings::Settings;
use lc_types::AppResult;
use tauri::{AppHandle, State};

use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn settings_get(state: State<'_, AppState>) -> AppResult<Settings> {
    state.settings.get().await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_set(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> AppResult<()> {
    // Proxy and window look apply immediately; game dir / parallelism on restart.
    state.downloader.set_manual_proxy(settings.proxy.clone());
    crate::apply_window_effects(&app, &settings);
    crate::commands::app::apply_close_to_tray(&app, settings.close_to_tray);
    state.settings.set(settings).await
}
