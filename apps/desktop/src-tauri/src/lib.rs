//! Tauri shell. Wiring (this file, `state.rs`, `tasks.rs`, `events.rs`,
//! `observers.rs`, `bindings.rs`) is owned by the lead; each `commands/<domain>.rs`
//! is owned by the role listed at its top.

pub mod bindings;
pub mod commands;
pub mod events;
pub mod observers;
pub mod state;
pub mod tasks;

pub fn run() {
    let builder = bindings::specta_builder();

    let result = tauri::Builder::default()
        .manage(state::AppState::stubs())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            Ok(())
        })
        .run(tauri::generate_context!());

    if let Err(e) = result {
        eprintln!("fatal: failed to run tauri application: {e}");
        std::process::exit(1);
    }
}
