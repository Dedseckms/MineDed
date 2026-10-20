//! Owner: mods.

use lc_types::version::{LoaderKind, LoaderVersion};
use lc_types::AppResult;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn loader_versions(
    state: State<'_, AppState>,
    loader: LoaderKind,
    mc: String,
) -> AppResult<Vec<LoaderVersion>> {
    state.loaders.get(loader)?.versions(&mc).await
}
