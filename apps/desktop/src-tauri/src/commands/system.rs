//! Owner: core.

use lc_types::system::{MemoryRecommendation, SystemInfo};
use lc_types::{AppResult, InstanceId};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn system_info(state: State<'_, AppState>) -> AppResult<SystemInfo> {
    state.system.info().await
}

#[tauri::command]
#[specta::specta]
pub async fn ram_recommend(
    state: State<'_, AppState>,
    id: InstanceId,
) -> AppResult<MemoryRecommendation> {
    state.system.recommend_memory(id).await
}
