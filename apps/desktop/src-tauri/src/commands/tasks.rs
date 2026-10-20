//! Owner: lead.

use lc_types::task::TaskSummary;
use lc_types::{AppResult, TaskId};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn tasks_list(state: State<'_, AppState>) -> AppResult<Vec<TaskSummary>> {
    Ok(state.tasks.list())
}

#[tauri::command]
#[specta::specta]
pub async fn task_cancel(state: State<'_, AppState>, id: TaskId) -> AppResult<()> {
    state.tasks.cancel(id)
}
