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
        observer,
    )?)
}

pub fn run() {
    let builder = bindings::specta_builder();

    let result = tauri::Builder::default()
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let state = build_state(app.handle())?;
            if let Some(vpn) = state.mihomo.clone() {
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = vpn.restore().await {
                        eprintln!("vpn restore failed: {e}");
                    }
                });
            }
            app.manage(state);
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
