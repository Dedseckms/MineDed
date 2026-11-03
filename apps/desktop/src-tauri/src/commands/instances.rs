//! Owner: core.

use lc_types::content::ContentKind;
use lc_types::instance::{Instance, InstancePatch, InstanceSpec, LaunchOpts, RunState};
use lc_types::task::{TaskEvent, TaskKind};
use lc_types::{AppError, AppResult, ErrorCode, InstanceId, TaskId};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::tasks::spawn_task;

#[tauri::command]
#[specta::specta]
pub async fn instance_list(state: State<'_, AppState>) -> AppResult<Vec<Instance>> {
    state.instances.list().await
}

#[tauri::command]
#[specta::specta]
pub async fn instance_get(state: State<'_, AppState>, id: InstanceId) -> AppResult<Instance> {
    state.instances.get(id).await
}

#[tauri::command]
#[specta::specta]
pub async fn instance_create(
    app: AppHandle,
    state: State<'_, AppState>,
    spec: InstanceSpec,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.instances.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::InstanceCreate,
        "tasks.instanceCreate",
        on_event,
        move |sink| async move { svc.create(spec, &*sink).await.map(|_| ()) },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn instance_clone(
    app: AppHandle,
    state: State<'_, AppState>,
    id: InstanceId,
    name: String,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.instances.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::InstanceCreate,
        "tasks.instanceClone",
        on_event,
        move |sink| async move { svc.clone_instance(id, &name, &*sink).await.map(|_| ()) },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn instance_update(
    state: State<'_, AppState>,
    id: InstanceId,
    patch: InstancePatch,
) -> AppResult<Instance> {
    state.instances.update(id, patch).await
}

#[tauri::command]
#[specta::specta]
pub async fn instance_delete(state: State<'_, AppState>, id: InstanceId) -> AppResult<()> {
    state.instances.delete(id).await
}

#[tauri::command]
#[specta::specta]
pub async fn instance_repair(
    app: AppHandle,
    state: State<'_, AppState>,
    id: InstanceId,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.instances.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::InstanceRepair,
        "tasks.instanceRepair",
        on_event,
        move |sink| async move { svc.repair(id, &*sink).await },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn instance_game_dir(state: State<'_, AppState>, id: InstanceId) -> AppResult<String> {
    Ok(state.instances.game_dir(id).to_string_lossy().into_owned())
}

/// Opens the instance folder of one content kind in the system file manager,
/// creating it first (`kind` null = the game folder itself).
///
/// Done in Rust on purpose: the WebView has no file-system permissions, and
/// `opener:allow-open-path` cannot create a folder that does not exist yet.
#[tauri::command]
#[specta::specta]
pub async fn instance_open_dir(
    state: State<'_, AppState>,
    id: InstanceId,
    kind: Option<ContentKind>,
) -> AppResult<()> {
    state.instances.get(id).await?;
    let mut dir = state.instances.game_dir(id);
    if let Some(kind) = kind {
        let sub = match kind {
            ContentKind::Mod => "mods",
            ContentKind::ResourcePack => "resourcepacks",
            ContentKind::Shader => "shaderpacks",
            ContentKind::DataPack => "datapacks",
            ContentKind::Modpack => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "modpacks are imported as instances",
                ))
            }
        };
        dir = dir.join(sub);
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| AppError::new(ErrorCode::Io, "cannot create the folder").detail("path", dir.display().to_string()).detail("error", e.to_string()))?;
    tauri_plugin_opener::open_path(dir.to_string_lossy().into_owned(), None::<String>)
        .map_err(|_| AppError::new(ErrorCode::Io, "cannot open the folder"))?;
    Ok(())
}

/// Returns once the JVM is running; state/logs arrive via `instance://*` events.
#[tauri::command]
#[specta::specta]
pub async fn instance_launch(
    app: AppHandle,
    state: State<'_, AppState>,
    id: InstanceId,
    opts: LaunchOpts,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let launcher = state.launcher.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::Launch,
        "tasks.launch",
        on_event,
        move |sink| async move { launcher.launch(id, opts, &*sink).await },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn instance_kill(state: State<'_, AppState>, id: InstanceId) -> AppResult<()> {
    state.launcher.kill(id).await
}

#[tauri::command]
#[specta::specta]
pub async fn instance_state(state: State<'_, AppState>, id: InstanceId) -> AppResult<RunState> {
    Ok(state.launcher.state(id))
}
