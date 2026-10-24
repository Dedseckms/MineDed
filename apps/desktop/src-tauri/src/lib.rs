//! Tauri shell. Wiring (this file, `state.rs`, `tasks.rs`, `events.rs`,
//! `observers.rs`, `bindings.rs`) is owned by the lead; each `commands/<domain>.rs`
//! is owned by the role listed at its top.

use std::sync::Arc;

use lc_runtime::{JsonSettingsStore, Paths, SettingsStore};
use tauri::Manager;

pub mod bindings;
pub mod commands;
pub mod events;
pub mod observers;
pub mod state;
pub mod tasks;

/// Builds the real service container: settings in the config dir, game data
/// in `settings.gameDir` or the app data dir.
fn build_state(app: &tauri::AppHandle) -> Result<state::AppState, Box<dyn std::error::Error>> {
    let config_dir = app.path().app_config_dir()?;
    let data_dir = app.path().app_data_dir()?;
    let settings_store: Arc<dyn SettingsStore> =
        Arc::new(JsonSettingsStore::new(config_dir.join("settings.json")));
    let settings = tauri::async_runtime::block_on(settings_store.get())?;
    let root = settings
        .game_dir
        .as_deref()
        .filter(|d| !d.trim().is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or(data_dir);
    let observer = Arc::new(observers::EventObserver::new(app.clone()));
    Ok(state::AppState::real(
        Paths::new(root),
        settings_store,
        &settings,
        observer.clone(),
        observer.clone(),
        observer,
    )?)
}

/// Keeps the non-blocking log writer alive for the app's lifetime.
struct LogGuard(#[allow(dead_code)] Option<tracing_appender::non_blocking::WorkerGuard>);

/// Daily-rotated `mineup.log` in the OS log dir (`%LOCALAPPDATA%\ru.dedseckms.mineup\logs`
/// on Windows). `RUST_LOG` overrides the default filter. Never fails the app.
fn init_logging(app: &tauri::AppHandle) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::{fmt, EnvFilter};
    let dir = app.path().app_log_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(dir, "mineup.log"));
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,lc_=debug,desktop=debug"));
    fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .ok()?;
    Some(guard)
}

pub fn run() {
    let builder = bindings::specta_builder();

    let result = tauri::Builder::default()
        // Must be first: a second launch only focuses the running window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            let log_guard = init_logging(app.handle());
            app.manage(LogGuard(log_guard));
            builder.mount_events(app);
            let state = build_state(app.handle())?;
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "MineUP started");
            if let Some(vpn) = state.mihomo.clone() {
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = vpn.restore().await {
                        tracing::warn!("vpn restore failed: {e}");
                    }
                });
            }
            app.manage(state);
            commands::app::spawn_auto_check(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!());

    match result {
        Ok(app) => app.run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(vpn) = handle.state::<state::AppState>().mihomo.as_ref() {
                    vpn.kill_now();
                }
            }
        }),
        Err(e) => {
            eprintln!("fatal: failed to run tauri application: {e}");
            std::process::exit(1);
        }
    }
}
