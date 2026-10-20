//! Owner: release. Self-update (tauri-plugin-updater, mirrored manifest).

use lc_types::system::UpdateInfo;
use lc_types::task::{TaskEvent, TaskKind};
use lc_types::{AppError, AppResult, TaskId};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::tasks::spawn_task;

#[tauri::command]
#[specta::specta]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
#[specta::specta]
pub async fn update_check() -> AppResult<Option<UpdateInfo>> {
    Err(AppError::not_implemented("update_check"))
}

#[tauri::command]
#[specta::specta]
pub async fn update_install(
    app: AppHandle,
    state: State<'_, AppState>,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::AppUpdate,
        "tasks.appUpdate",
        on_event,
        |_sink| async { Err(AppError::not_implemented("update_install")) },
    ))
}
