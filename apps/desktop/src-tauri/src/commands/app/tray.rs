//! System tray (owner: release). Closing the window hides it instead of
//! quitting, so the mihomo sidecar, P2P rooms and voice keep running. Only an
//! explicit "Quit" ends the process, and it stops those services first.
//!
//! The "close to tray" toggle is stored in `tray.json` next to `settings.json`
//! until `Settings::close_to_tray` lands in the contract (see CONTRACT REQUEST
//! in the release report); [`TrayPrefs`] is the only place that changes then.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use lc_types::settings::Language;
use lc_types::vpn::{VpnMode, VpnState};
use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::state::AppState;

/// Tray icon id; also used to fetch the handle back for tooltip updates.
const TRAY_ID: &str = "main";
/// Toggle file, next to `settings.json` in the app config dir.
const FILE: &str = "tray.json";
const ID_OPEN: &str = "tray-open";
const ID_HIDE: &str = "tray-hide";
const ID_VPN: &str = "tray-vpn";
const ID_CLOSE_TO_TRAY: &str = "tray-close-to-tray";
const ID_QUIT: &str = "tray-quit";
/// Poll period of the menu watcher. Light mode relaxes it: the tray is a static
/// icon, only the labels need refreshing.
const POLL: Duration = Duration::from_secs(3);
const POLL_LIGHT: Duration = Duration::from_secs(15);
/// A dead sidecar must never keep the process alive.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const LEAVE_TIMEOUT: Duration = Duration::from_secs(2);

/// Persisted tray preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrayPrefs {
    /// Close button hides the window while a tray icon is available.
    pub close_to_tray: bool,
}

impl Default for TrayPrefs {
    fn default() -> Self {
        Self {
            close_to_tray: true,
        }
    }
}

/// `tray.json`: tolerant reader, the file is only a preference and a corrupt
/// one must not break startup.
#[derive(Debug)]
struct TrayStore {
    path: PathBuf,
    prefs: Mutex<TrayPrefs>,
}

impl TrayStore {
    fn load(path: PathBuf) -> Self {
        let prefs = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            path,
            prefs: Mutex::new(prefs),
        }
    }

    fn get(&self) -> TrayPrefs {
        self.prefs.lock().map(|p| *p).unwrap_or_default()
    }

    fn set(&self, prefs: TrayPrefs) -> std::io::Result<()> {
        if let Ok(mut current) = self.prefs.lock() {
            *current = prefs;
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&prefs)?;
        std::fs::write(&self.path, text)
    }
}

/// Menu entry texts; the tray is native, so it is localized in Rust from
/// `Settings::language`.
struct Labels {
    open: &'static str,
    hide: &'static str,
    vpn_on: &'static str,
    vpn_off: &'static str,
    close_to_tray: &'static str,
    quit: &'static str,
    tooltip: &'static str,
    tooltip_vpn: &'static str,
}

fn labels(language: Language) -> Labels {
    match language {
        Language::Ru => Labels {
            open: "Открыть MineDed",
            hide: "Свернуть в трей",
            vpn_on: "VPN: включён — выключить",
            vpn_off: "VPN: выключен — включить",
            close_to_tray: "Закрывать в трей",
            quit: "Выход",
            tooltip: "MineDed",
            tooltip_vpn: "MineDed — VPN включён",
        },
        Language::En => Labels {
            open: "Open MineDed",
            hide: "Minimize to tray",
            vpn_on: "VPN: on — turn off",
            vpn_off: "VPN: off — turn on",
            close_to_tray: "Close to tray",
            quit: "Quit",
            tooltip: "MineDed",
            tooltip_vpn: "MineDed — VPN on",
        },
    }
}

/// Menu items kept for text updates (VPN status, language, toggle state).
struct Items {
    open: MenuItem<Wry>,
    hide: MenuItem<Wry>,
    vpn: MenuItem<Wry>,
    close_to_tray: CheckMenuItem<Wry>,
    quit: MenuItem<Wry>,
}

/// Managed tray state. Present only when the platform actually gave us an icon.
struct TrayState {
    prefs: TrayStore,
    /// False on Linux without AppIndicator/StatusNotifier: there is no icon to
    /// restore from, so the close button keeps quitting.
    available: bool,
    /// Last mode the user ran; the "VPN: on" item restores it.
    last_mode: Mutex<VpnMode>,
    items: Items,
    /// Last applied `(language, vpn running)`, to skip redundant menu updates.
    applied: Mutex<Option<(Language, bool)>>,
}

/// Hides instead of closing only when the user asked for it and an icon is
/// really there (a hidden window with no tray icon would be lost for good).
fn hides_on_close(prefs: TrayPrefs, available: bool) -> bool {
    prefs.close_to_tray && available
}

/// Builds the tray. Never fails the app: on a platform without a tray a warning
/// is logged and closing the window behaves exactly as before.
pub fn setup(app: &AppHandle) {
    let store = TrayStore::load(config_path(app));
    let texts = labels(current_language(app));
    let items = match build_items(app, &texts, store.get()) {
        Ok(items) => items,
        Err(e) => {
            tracing::warn!("tray: cannot build the menu: {e}");
            install_close_handler(app);
            return;
        }
    };
    let available = match build_tray(app, &items, &texts) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("tray: unavailable, the close button quits as before: {e}");
            false
        }
    };
    app.manage(TrayState {
        prefs: store,
        available,
        last_mode: Mutex::new(VpnMode::LauncherOnly),
        items,
        applied: Mutex::new(None),
    });
    app.on_menu_event(on_menu_event);
    install_close_handler(app);
    spawn_watcher(app);
}

fn config_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join(FILE))
        .unwrap_or_else(|_| PathBuf::from(FILE))
}

fn current_language(app: &AppHandle) -> Language {
    let Some(state) = app.try_state::<AppState>() else {
        return Language::Ru;
    };
    tauri::async_runtime::block_on(state.settings.get())
        .map(|s| s.language)
        .unwrap_or(Language::Ru)
}

async fn current_light_mode(app: &AppHandle) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    state
        .settings
        .get()
        .await
        .map(|s| s.light_mode)
        .unwrap_or(false)
}

fn build_items(app: &AppHandle, texts: &Labels, prefs: TrayPrefs) -> tauri::Result<Items> {
    Ok(Items {
        open: MenuItem::with_id(app, ID_OPEN, texts.open, true, None::<&str>)?,
        hide: MenuItem::with_id(app, ID_HIDE, texts.hide, true, None::<&str>)?,
        vpn: MenuItem::with_id(app, ID_VPN, texts.vpn_off, true, None::<&str>)?,
        close_to_tray: CheckMenuItem::with_id(
            app,
            ID_CLOSE_TO_TRAY,
            texts.close_to_tray,
            true,
            prefs.close_to_tray,
            None::<&str>,
        )?,
        quit: MenuItem::with_id(app, ID_QUIT, texts.quit, true, None::<&str>)?,
    })
}

fn build_tray(app: &AppHandle, items: &Items, texts: &Labels) -> tauri::Result<()> {
    let top = PredefinedMenuItem::separator(app)?;
    let middle = PredefinedMenuItem::separator(app)?;
    let bottom = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &items.open,
            &top,
            &items.vpn,
            &middle,
            &items.hide,
            &items.close_to_tray,
            &bottom,
            &items.quit,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(texts.tooltip)
        .on_tray_icon_event(|tray, event| {
            // Left click opens the window; the menu stays on the right button.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    // The bundled .ico (Windows) / PNG (Linux) the app already uses: reusing it
    // means the icon is set once at startup and never replaced (no flicker).
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// Close button → hide, so mihomo and the rooms survive. Without a tray icon
/// the event is left alone and the app exits as before.
fn install_close_handler(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let handle = app.clone();
    window.on_window_event(move |event| {
        let tauri::WindowEvent::CloseRequested { api, .. } = event else {
            return;
        };
        if !hides_on_close_now(&handle) {
            return;
        }
        api.prevent_close();
        hide_main(&handle);
        tracing::debug!("tray: window hidden, the launcher keeps running");
    });
}

fn hides_on_close_now(app: &AppHandle) -> bool {
    match app.try_state::<TrayState>() {
        Some(tray) => hides_on_close(tray.prefs.get(), tray.available),
        None => false,
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().0.as_str() {
        ID_OPEN => show_main(app),
        ID_HIDE => hide_main(app),
        ID_VPN => toggle_vpn(app),
        ID_CLOSE_TO_TRAY => toggle_close_to_tray(app),
        ID_QUIT => quit_app(app),
        _ => {}
    }
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn hide_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

/// Toggles the VPN from the tray: stops it when it runs, otherwise restores the
/// mode that ran last (`LauncherOnly` on a fresh start).
fn toggle_vpn(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let vpn = state.vpn.clone();
    let restore = app
        .try_state::<TrayState>()
        .and_then(|tray| tray.last_mode.lock().ok().map(|mode| *mode))
        .unwrap_or(VpnMode::LauncherOnly);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let running = vpn.state().await.map(|s| s.running).unwrap_or(false);
        let target = if running { VpnMode::Off } else { restore };
        if let Err(e) = vpn.set_mode(target).await {
            tracing::warn!("tray: vpn toggle failed: {e}");
        }
        refresh(&handle).await;
    });
}

fn toggle_close_to_tray(app: &AppHandle) {
    let Some(tray) = app.try_state::<TrayState>() else {
        return;
    };
    let next = !tray.prefs.get().close_to_tray;
    if let Err(e) = tray.prefs.set(TrayPrefs {
        close_to_tray: next,
    }) {
        tracing::warn!("tray: cannot save preferences: {e}");
    }
    let _ = tray.items.close_to_tray.set_checked(next);
}

/// Applies the `close_to_tray` pref coming from `Settings`.
pub(crate) fn set_close_to_tray(app: &AppHandle, enabled: bool) {
    let Some(tray) = app.try_state::<TrayState>() else {
        return;
    };
    if let Err(e) = tray.prefs.set(TrayPrefs {
        close_to_tray: enabled,
    }) {
        tracing::warn!("tray: cannot save preferences: {e}");
    }
    let _ = tray.items.close_to_tray.set_checked(enabled);
}

/// Stops the services and exits, used by the tray menu and `app_quit`.
pub(crate) fn quit_now(app: &AppHandle) {
    quit_app(app);
}

fn quit_app(app: &AppHandle) {
    let target = AppQuit(app.clone());
    tauri::async_runtime::spawn(async move { quit(&target).await });
}

/// Shutdown steps for an explicit quit. Kept behind a trait so the order
/// (services stopped, then the process goes) is testable without Tauri.
#[async_trait::async_trait]
trait QuitTarget: Send + Sync {
    async fn stop_services(&self);
    fn exit_app(&self);
}

async fn quit(target: &dyn QuitTarget) {
    target.stop_services().await;
    target.exit_app();
}

struct AppQuit(AppHandle);

#[async_trait::async_trait]
impl QuitTarget for AppQuit {
    async fn stop_services(&self) {
        let Some(state) = self.0.try_state::<AppState>() else {
            return;
        };
        // mihomo is a sidecar: it must not outlive the launcher.
        if let Some(mihomo) = state.mihomo.as_ref() {
            if tokio::time::timeout(SHUTDOWN_TIMEOUT, mihomo.shutdown())
                .await
                .is_err()
            {
                tracing::warn!("tray: vpn shutdown timed out, killing the sidecar");
            }
            mihomo.kill_now();
        }
        // Rooms send `bye` to peers and voice releases the microphone.
        let _ = tokio::time::timeout(LEAVE_TIMEOUT, state.rooms.leave()).await;
        let _ = tokio::time::timeout(LEAVE_TIMEOUT, state.voice.stop()).await;
    }

    fn exit_app(&self) {
        self.0.exit(0);
    }
}

fn spawn_watcher(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let period = if current_light_mode(&app).await {
            POLL_LIGHT
        } else {
            POLL
        };
        let mut ticker = tokio::time::interval(period);
        loop {
            ticker.tick().await;
            refresh(&app).await;
        }
    });
}

/// Re-reads the VPN state and the language; the menu is native, so it has to be
/// kept in sync from here (there is no "menu about to open" hook).
async fn refresh(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let language = state
        .settings
        .get()
        .await
        .map(|s| s.language)
        .unwrap_or(Language::Ru);
    let vpn = state.vpn.state().await.ok();
    apply(app, language, vpn.as_ref());
}

fn apply(app: &AppHandle, language: Language, vpn: Option<&VpnState>) {
    let Some(tray) = app.try_state::<TrayState>() else {
        return;
    };
    let running = vpn.map(|s| s.running).unwrap_or(false);
    if let Some(mode) = vpn.map(|s| s.mode).filter(|m| *m != VpnMode::Off) {
        if let Ok(mut last) = tray.last_mode.lock() {
            *last = mode;
        }
    }
    let Ok(mut applied) = tray.applied.lock() else {
        return;
    };
    if *applied == Some((language, running)) {
        return;
    }
    *applied = Some((language, running));

    let texts = labels(language);
    let _ = tray.items.open.set_text(texts.open);
    let _ = tray.items.hide.set_text(texts.hide);
    let _ = tray
        .items
        .vpn
        .set_text(if running { texts.vpn_on } else { texts.vpn_off });
    let _ = tray.items.close_to_tray.set_text(texts.close_to_tray);
    let _ = tray.items.quit.set_text(texts.quit);
    if let Some(icon) = app.tray_by_id(TRAY_ID) {
        let tooltip = if running {
            texts.tooltip_vpn
        } else {
            texts.tooltip
        };
        let _ = icon.set_tooltip(Some(tooltip));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_hides_only_with_a_visible_tray() {
        let on = TrayPrefs {
            close_to_tray: true,
        };
        let off = TrayPrefs {
            close_to_tray: false,
        };
        assert!(hides_on_close(on, true));
        // No AppIndicator/StatusNotifier: a hidden window could not be restored.
        assert!(!hides_on_close(on, false));
        assert!(!hides_on_close(off, true));
        assert!(!hides_on_close(off, false));
    }

    #[test]
    fn prefs_default_to_closing_into_the_tray() {
        assert!(TrayPrefs::default().close_to_tray);
    }

    #[test]
    fn prefs_survive_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("mineded-tray-{}", std::process::id()));
        let path = dir.join(FILE);
        let store = TrayStore::load(path.clone());
        assert!(store.get().close_to_tray);
        store
            .set(TrayPrefs {
                close_to_tray: false,
            })
            .unwrap();
        assert!(!TrayStore::load(path).get().close_to_tray);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_or_missing_prefs_fall_back_to_the_default() {
        let dir = std::env::temp_dir().join(format!("mineded-tray-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        assert!(TrayStore::load(path.clone()).get().close_to_tray);
        std::fs::write(&path, "{ not json").unwrap();
        assert!(TrayStore::load(path).get().close_to_tray);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_prefs_file_keeps_defaults() {
        let prefs: TrayPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs, TrayPrefs::default());
    }

    #[test]
    fn labels_follow_the_language() {
        assert_eq!(labels(Language::Ru).open, "Открыть MineDed");
        assert_eq!(labels(Language::En).open, "Open MineDed");
        assert!(labels(Language::Ru).vpn_on.contains("VPN"));
        assert!(labels(Language::En).vpn_off.contains("VPN"));
    }

    /// Records the shutdown steps so the order can be asserted.
    #[derive(Default)]
    struct Recorder {
        steps: Mutex<Vec<&'static str>>,
    }

    #[async_trait::async_trait]
    impl QuitTarget for Recorder {
        async fn stop_services(&self) {
            self.steps.lock().unwrap().push("stop");
        }

        fn exit_app(&self) {
            self.steps.lock().unwrap().push("exit");
        }
    }

    #[test]
    fn quit_stops_services_before_exiting() {
        let recorder = Recorder::default();
        tauri::async_runtime::block_on(quit(&recorder));
        assert_eq!(*recorder.steps.lock().unwrap(), ["stop", "exit"]);
    }
}
