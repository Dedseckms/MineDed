//! Owner: lead.

use lc_types::settings::Settings;
use lc_types::AppResult;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn settings_get(state: State<'_, AppState>) -> AppResult<Settings> {
    state.settings.get().await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_set(state: State<'_, AppState>, settings: Settings) -> AppResult<()> {
    state.settings.set(settings).await
}
