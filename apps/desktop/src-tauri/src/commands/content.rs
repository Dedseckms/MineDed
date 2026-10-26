//! Owner: mods. Includes modpack import/export and performance presets.

use lc_types::content::{
    ContentFilter, ContentKind, ContentSourceKind, ContentVersion, ContentVersionRef,
    InstalledContent, LocalDepIssue, ResolvePlan, SearchPage, SearchQuery,
};
use lc_types::instance::{PackFormat, PerfPreset};
use lc_types::task::{TaskEvent, TaskKind};
use lc_types::{AppResult, InstanceId, TaskId};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::tasks::spawn_task;

#[tauri::command]
#[specta::specta]
pub async fn content_search(
    state: State<'_, AppState>,
    query: SearchQuery,
) -> AppResult<SearchPage> {
    state.content.search(query).await
}

#[tauri::command]
#[specta::specta]
pub async fn content_versions(
    state: State<'_, AppState>,
    source: ContentSourceKind,
    project_id: String,
    filter: ContentFilter,
) -> AppResult<Vec<ContentVersion>> {
    state.content.versions(source, &project_id, filter).await
}

#[tauri::command]
#[specta::specta]
pub async fn content_resolve(
    state: State<'_, AppState>,
    instance: InstanceId,
    picks: Vec<ContentVersionRef>,
) -> AppResult<ResolvePlan> {
    state.content.resolve(instance, picks).await
}

#[tauri::command]
#[specta::specta]
pub async fn content_install(
    app: AppHandle,
    state: State<'_, AppState>,
    instance: InstanceId,
    plan: ResolvePlan,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.content.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::ContentInstall,
        "tasks.contentInstall",
        on_event,
        move |sink| async move { svc.install(instance, plan, &*sink).await },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn content_list(
    state: State<'_, AppState>,
    instance: InstanceId,
) -> AppResult<Vec<InstalledContent>> {
    state.content.list(instance).await
}

#[tauri::command]
#[specta::specta]
pub async fn content_set_enabled(
    state: State<'_, AppState>,
    instance: InstanceId,
    file_name: String,
    enabled: bool,
) -> AppResult<()> {
    state
        .content
        .set_enabled(instance, &file_name, enabled)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn content_remove(
    state: State<'_, AppState>,
    instance: InstanceId,
    file_name: String,
) -> AppResult<()> {
    state.content.remove(instance, &file_name).await
}

/// Copies local files/folders into the instance (`kind` picks the folder; wrong type → `invalidInput`).
#[tauri::command]
#[specta::specta]
pub async fn content_add_local(
    state: State<'_, AppState>,
    instance: InstanceId,
    kind: ContentKind,
    paths: Vec<String>,
) -> AppResult<Vec<InstalledContent>> {
    state.content.add_local(instance, kind, paths).await
}

/// Dependency problems of local mod jars (`fileNames` empty = every local mod).
#[tauri::command]
#[specta::specta]
pub async fn content_check_local(
    state: State<'_, AppState>,
    instance: InstanceId,
    file_names: Vec<String>,
) -> AppResult<Vec<LocalDepIssue>> {
    state.content.check_local(instance, file_names).await
}

#[tauri::command]
#[specta::specta]
pub async fn content_update_all(
    app: AppHandle,
    state: State<'_, AppState>,
    instance: InstanceId,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.content.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::ContentUpdate,
        "tasks.contentUpdate",
        on_event,
        move |sink| async move { svc.update_all(instance, &*sink).await },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn preset_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    instance: InstanceId,
    preset: PerfPreset,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.presets.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::PresetApply,
        "tasks.presetApply",
        on_event,
        move |sink| async move { svc.apply(instance, preset, &*sink).await },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn instance_import(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.packs.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::InstanceImport,
        "tasks.instanceImport",
        on_event,
        move |sink| async move { svc.import(&path, &*sink).await.map(|_| ()) },
    ))
}

/// Installs a Modrinth modpack as a new instance (`versionId` null = latest; CurseForge is disabled).
#[tauri::command]
#[specta::specta]
pub async fn modpack_install(
    app: AppHandle,
    state: State<'_, AppState>,
    source: ContentSourceKind,
    project_id: String,
    version_id: Option<String>,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.packs.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::InstanceImport,
        "tasks.modpackInstall",
        on_event,
        move |sink| async move {
            svc.install_remote(source, &project_id, version_id.as_deref(), &*sink)
                .await
                .map(|_| ())
        },
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn instance_export(
    app: AppHandle,
    state: State<'_, AppState>,
    id: InstanceId,
    format: PackFormat,
    path: String,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let svc = state.packs.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::InstanceExport,
        "tasks.instanceExport",
        on_event,
        move |sink| async move { svc.export(id, format, &path, &*sink).await },
    ))
}
