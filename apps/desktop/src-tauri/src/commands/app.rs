//! Owner: release. Self-update (tauri-plugin-updater, mirrored manifest).
//!
//! Endpoints, public key and Windows install mode live in `tauri.conf.json`
//! (`plugins.updater`). The plugin tries the endpoints in order (GitHub
//! Releases first, then the owner's mirror) and verifies every package with
//! the minisign public key before installing it.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lc_runtime::ProgressSink;
use lc_types::system::UpdateInfo;
use lc_types::task::{TaskEvent, TaskKind};
use lc_types::{AppError, AppResult, ErrorCode, TaskId};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};
use tauri_specta::Event;

use crate::events::UpdateAvailableEvent;
use crate::state::AppState;
use crate::tasks::spawn_task;

#[tauri::command]
#[specta::specta]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

// Manual check (the user pressed "check for updates"): always queries the
// endpoints, regardless of `Settings.auto_update`. Emits `update://available`
// when a newer version exists. Plain comment: doc comments leak into bindings.ts.
#[tauri::command]
#[specta::specta]
pub async fn update_check(app: AppHandle) -> AppResult<Option<UpdateInfo>> {
    let info = check(&app).await?.map(|u| info_from(&u));
    if let Some(info) = &info {
        let _ = UpdateAvailableEvent(info.clone()).emit(&app);
    }
    Ok(info)
}

#[tauri::command]
#[specta::specta]
pub async fn update_install(
    app: AppHandle,
    state: State<'_, AppState>,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskId> {
    let handle = app.clone();
    Ok(spawn_task(
        &app,
        &state.tasks,
        TaskKind::AppUpdate,
        "tasks.appUpdate",
        on_event,
        move |sink| install(handle, sink),
    ))
}

/// Background check at startup. Does nothing when `Settings.auto_update` is
/// off; otherwise emits `update://available` if a newer version exists.
/// Errors (offline, blocked endpoints) are only logged.
pub fn spawn_auto_check(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let enabled = match app.try_state::<AppState>() {
            Some(state) => match state.settings.get().await {
                Ok(s) => s.auto_update,
                Err(e) => {
                    eprintln!("update: cannot read settings: {e}");
                    return;
                }
            },
            None => return,
        };
        if !enabled {
            return;
        }
        match check(&app).await {
            Ok(Some(update)) => {
                let _ = UpdateAvailableEvent(info_from(&update)).emit(&app);
            }
            Ok(None) => {}
            Err(e) => eprintln!("update: check failed: {e}"),
        }
    });
}

async fn install(app: AppHandle, sink: Arc<crate::tasks::ChannelSink>) -> AppResult<()> {
    sink.progress(0, None, "check");
    let update = check(&app)
        .await?
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "no update available"))?;
    sink.check_cancelled()?;

    let done = AtomicU64::new(0);
    let download = update.download(
        |chunk, total| {
            let now = done.fetch_add(chunk as u64, Ordering::Relaxed) + chunk as u64;
            sink.progress(now, total, "download");
        },
        || {},
    );
    let bytes = tokio::select! {
        r = download => r.map_err(|e| map_err(e, "download"))?,
        _ = sink.cancel_token().cancelled() => {
            return Err(AppError::new(ErrorCode::Cancelled, "cancelled by user"));
        }
    };
    sink.check_cancelled()?;

    sink.progress(0, None, "install");
    // Windows: launches the installer (passive mode) and exits the app; the
    // installer relaunches it. Linux (AppImage/deb): files are replaced in place.
    update.install(bytes).map_err(|e| map_err(e, "install"))?;
    app.restart();
}

/// Returns `Ok(None)` when up to date. All endpoints failing is a `Network` error.
async fn check(app: &AppHandle) -> AppResult<Option<Update>> {
    updater(app)?.check().await.map_err(|e| map_err(e, "check"))
}

fn updater(app: &AppHandle) -> AppResult<Updater> {
    // `UpdaterExt` panics if the plugin is not registered in `lib.rs`; turn
    // that wiring mistake into an error instead of a hung IPC call.
    match catch_unwind(AssertUnwindSafe(|| app.updater())) {
        Ok(r) => r.map_err(|e| map_err(e, "init")),
        Err(_) => Err(AppError::not_implemented(
            "updater plugin is not registered",
        )),
    }
}

fn map_err(e: tauri_plugin_updater::Error, stage: &str) -> AppError {
    use tauri_plugin_updater::Error as E;
    let code = match &e {
        E::Reqwest(_) | E::Network(_) | E::ReleaseNotFound => ErrorCode::Network,
        E::Minisign(_)
        | E::Base64(_)
        | E::SignatureUtf8(_)
        | E::SignedVersionMismatch { .. }
        | E::MissingSignedVersion => ErrorCode::HashMismatch,
        E::AuthenticationFailed => ErrorCode::PermissionDenied,
        E::Io(_) => ErrorCode::Io,
        _ => ErrorCode::Internal,
    };
    AppError::new(code, format!("update {stage} failed: {e}")).detail("stage", stage)
}

fn info_from(u: &Update) -> UpdateInfo {
    info_from_parts(
        &u.version,
        &u.current_version,
        u.body.as_deref(),
        &u.raw_json,
    )
}

/// `pub_date` is read from the raw manifest: it is already RFC 3339 there.
fn info_from_parts(
    version: &str,
    current: &str,
    body: Option<&str>,
    raw: &serde_json::Value,
) -> UpdateInfo {
    UpdateInfo {
        version: version.to_owned(),
        current_version: current.to_owned(),
        notes: body.unwrap_or_default().to_owned(),
        date: raw
            .get("pub_date")
            .and_then(|d| d.as_str())
            .map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_takes_date_from_manifest() {
        let raw = serde_json::json!({
            "version": "0.2.0",
            "notes": "fixes",
            "pub_date": "2026-10-24T10:00:00Z",
            "platforms": {}
        });
        let info = info_from_parts("0.2.0", "0.1.0", Some("fixes"), &raw);
        assert_eq!(info.version, "0.2.0");
        assert_eq!(info.current_version, "0.1.0");
        assert_eq!(info.notes, "fixes");
        assert_eq!(info.date.as_deref(), Some("2026-10-24T10:00:00Z"));
    }

    #[test]
    fn info_without_notes_and_date() {
        let info = info_from_parts("1.0.0", "0.9.0", None, &serde_json::json!({}));
        assert_eq!(info.notes, "");
        assert_eq!(info.date, None);
    }

    #[test]
    fn config_has_signed_mirrored_updater() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(conf["productName"], "MineUP");
        assert_eq!(conf["identifier"], "ru.dedseckms.mineup");
        assert_eq!(conf["bundle"]["createUpdaterArtifacts"], true);
        let up = &conf["plugins"]["updater"];
        assert!(!up["pubkey"].as_str().unwrap_or_default().is_empty());
        let endpoints: Vec<&str> = up["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e.as_str())
            .collect();
        assert_eq!(
            endpoints,
            [
                "https://github.com/Dedseckms/MineUP/releases/latest/download/latest.json",
                "https://relay.dedseckms.ru/updates/latest.json",
            ]
        );
        let targets = conf["bundle"]["targets"].as_array().unwrap();
        for t in ["nsis", "msi", "appimage", "deb"] {
            assert!(targets.iter().any(|v| v == t), "missing bundle target {t}");
        }
    }
}
