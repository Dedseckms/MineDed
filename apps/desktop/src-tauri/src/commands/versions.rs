//! Owner: core.

use lc_types::version::{VersionEntry, VersionFilter};
use lc_types::AppResult;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn versions_list(
    state: State<'_, AppState>,
    filter: VersionFilter,
) -> AppResult<Vec<VersionEntry>> {
    state.versions.list(filter).await
}
