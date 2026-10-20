//! Owner: core.

use lc_types::java::JavaRuntime;
use lc_types::task::{TaskEvent, TaskKind};
use lc_types::{AppResult, TaskId};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::tasks::spawn_task;

#[tauri::command]
#[specta::specta]
pub async fn java_list(state: State<'_, AppState>) -> AppResult<Vec<JavaRuntime>> {
    state.java.list().await
}

#[tauri::command]
#[specta::specta]
pub async fn java_detect(state: State<'_, AppState>) -> AppResult<Vec<JavaRuntime>> {
    state.java.detect().await
}

#[tauri::command]
#[specta::specta]
pub async fn java_install(
    app: AppHandle,
    state: State<'_, AppState>,
    major: u32,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let java = state.java.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::JavaInstall,
        "tasks.javaInstall",
        on_event,
        move |sink| async move { java.install(major, &*sink).await.map(|_| ()) },
    ))
}
