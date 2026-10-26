//! Programs routed in `VpnMode::Apps`: validation of the persisted list and
//! the running-process picker.

use std::path::{Path, PathBuf};

use lc_types::vpn::VpnApp;
use lc_types::{AppError, AppResult, ErrorCode};

use crate::config::rule_safe;

/// Upper bound for the persisted list (rules stay small).
const MAX_APPS: usize = 200;

/// Infrastructure processes never offered in the picker (lowercase).
const HIDDEN_NAMES: &[&str] = &[
    // Windows (outside %SystemRoot%)
    "msedgewebview2.exe",
    "crashpad_handler.exe",
    "mihomo.exe",
    // Linux
    "mihomo",
    "systemd",
    "dbus-daemon",
    "dbus-broker",
    "dbus-broker-launch",
    "pipewire",
    "pipewire-pulse",
    "wireplumber",
    "pulseaudio",
    "xorg",
    "xwayland",
    "gnome-shell",
    "plasmashell",
    "kwin_wayland",
    "kwin_x11",
    "ksmserver",
    "kded5",
    "kded6",
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ssh-agent",
    "gpg-agent",
    "sshd",
    "bwrap",
    "flatpak-portal",
];

/// Linux helper families (prefixes, lowercase).
const HIDDEN_PREFIXES: &[&str] = &[
    "gvfs",
    "at-spi",
    "xdg-",
    "gsd-",
    "evolution-",
    "ibus",
    "tracker-",
    "goa-",
];

/// Linux system executable directories.
const SYSTEM_DIRS: &[&str] = &[
    "/usr/lib/systemd/",
    "/lib/systemd/",
    "/usr/libexec/",
    "/usr/sbin/",
    "/sbin/",
];

fn key(process: &str) -> String {
    if cfg!(windows) {
        process.to_lowercase()
    } else {
        process.to_owned()
    }
}

fn file_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
}

fn display_name(process: &str) -> String {
    let stem = if process.to_ascii_lowercase().ends_with(".exe") {
        &process[..process.len() - 4]
    } else {
        process
    };
    stem.to_owned()
}

/// Trims, fills `process` from `path` and `name` from `process`, rejects
/// values that could break a mihomo rule, dedups by process name.
pub(crate) fn normalize(apps: Vec<VpnApp>) -> AppResult<Vec<VpnApp>> {
    if apps.len() > MAX_APPS {
        return Err(AppError::new(ErrorCode::InvalidInput, "too many VPN apps")
            .detail("max", MAX_APPS.to_string()));
    }
    let mut out: Vec<VpnApp> = Vec::with_capacity(apps.len());
    for (i, app) in apps.into_iter().enumerate() {
        let path = app
            .path
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty());
        let mut process = app.process.trim().to_owned();
        if process.is_empty() {
            process = path.as_deref().and_then(file_name).unwrap_or_default();
        }
        if !rule_safe(&process) {
            return Err(
                AppError::new(ErrorCode::InvalidInput, "invalid program file name")
                    .detail("index", i.to_string()),
            );
        }
        let mut name = app.name.trim().to_owned();
        if name.is_empty() {
            name = display_name(&process);
        }
        if out.iter().any(|a| key(&a.process) == key(&process)) {
            continue;
        }
        out.push(VpnApp {
            name,
            process,
            path,
        });
    }
    Ok(out)
}

/// Process names that are always routed in `VpnMode::Apps`: the launcher
/// itself and the game's Java.
pub(crate) fn always_routed() -> Vec<String> {
    let mut out: Vec<String> = if cfg!(windows) {
        vec!["java.exe".into(), "javaw.exe".into()]
    } else {
        vec!["java".into()]
    };
    if let Some(me) = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned))
    {
        out.insert(0, me);
    }
    out
}

/// Whether a running executable belongs in the picker.
fn visible(exe: &Path, own_exe: Option<&Path>, system_root: Option<&Path>) -> bool {
    let Some(name) = exe.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lname = name.to_lowercase();
    if HIDDEN_NAMES.contains(&lname.as_str())
        || HIDDEN_PREFIXES.iter().any(|p| lname.starts_with(p))
    {
        return false;
    }
    let s = exe.to_string_lossy();
    let ls = s.to_lowercase();
    if let Some(own) = own_exe {
        if own.to_string_lossy().to_lowercase() == ls {
            return false;
        }
    }
    if let Some(root) = system_root {
        let mut r = root.to_string_lossy().to_lowercase();
        if !r.ends_with(['\\', '/']) {
            r.push('\\');
        }
        if ls.starts_with(&r) {
            return false;
        }
    }
    !SYSTEM_DIRS.iter().any(|d| s.starts_with(d))
}

/// Sorted, deduplicated (by executable path) user-visible programs.
fn collect(
    exes: impl IntoIterator<Item = PathBuf>,
    own: Option<&Path>,
    root: Option<&Path>,
) -> Vec<VpnApp> {
    let mut out: Vec<VpnApp> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for exe in exes {
        if !visible(&exe, own, root) {
            continue;
        }
        let path = exe.to_string_lossy().into_owned();
        let k = key(&path);
        if seen.contains(&k) {
            continue;
        }
        seen.push(k);
        let Some(process) = file_name(&path) else {
            continue;
        };
        out.push(VpnApp {
            name: display_name(&process),
            process,
            path: Some(path),
        });
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

#[cfg(target_os = "linux")]
fn owned_by_me(pid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let me = std::fs::metadata("/proc/self").map(|m| m.uid());
    let it = std::fs::metadata(format!("/proc/{pid}")).map(|m| m.uid());
    matches!((me, it), (Ok(a), Ok(b)) if a == b)
}

#[cfg(not(target_os = "linux"))]
fn owned_by_me(_pid: u32) -> bool {
    // Windows: processes of other users / elevated ones have no readable
    // executable path for us and are skipped anyway.
    true
}

/// Running user programs. Blocking (process table scan).
pub(crate) fn running() -> Vec<VpnApp> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let me = std::process::id();
    let exes: Vec<PathBuf> = sys
        .processes()
        .iter()
        .filter(|(pid, p)| {
            pid.as_u32() != me && p.thread_kind().is_none() && owned_by_me(pid.as_u32())
        })
        .filter_map(|(_, p)| p.exe().map(Path::to_path_buf))
        .collect();
    let own = std::env::current_exe().ok();
    let root = if cfg!(windows) {
        Some(PathBuf::from(
            std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
        ))
    } else {
        None
    };
    collect(exes, own.as_deref(), root.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, process: &str, path: Option<&str>) -> VpnApp {
        VpnApp {
            name: name.into(),
            process: process.into(),
            path: path.map(Into::into),
        }
    }

    #[test]
    fn normalize_fills_validates_and_dedups() {
        let discord = if cfg!(windows) {
            "C:\\Apps\\Discord.exe"
        } else {
            "/opt/Discord.exe"
        };
        let out = normalize(vec![
            app(" Telegram ", " Telegram.exe ", None),
            app("", "", Some(discord)),
            app("", "firefox", Some(" ")),
            app("Dup", "Telegram.exe", None),
        ])
        .unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], app("Telegram", "Telegram.exe", None));
        assert_eq!(out[1], app("Discord", "Discord.exe", Some(discord)));
        assert_eq!(out[2], app("firefox", "firefox", None));
        for bad in ["a,DIRECT", "x\ny", "", "dir/x", "dir\\x"] {
            let err = normalize(vec![app("n", bad, None)]).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad:?}");
        }
        let many = vec![app("a", "a.exe", None); MAX_APPS + 1];
        assert!(normalize(many).is_err());
    }

    #[test]
    fn always_routes_java_and_launcher() {
        let r = always_routed();
        assert!(r.iter().any(|p| p.starts_with("java")));
        assert!(r.len() >= 2);
    }

    #[cfg(windows)]
    #[test]
    fn picker_skips_system_and_dedups() {
        let root = Path::new("C:\\Windows");
        let own = Path::new("C:\\Program Files\\Launcher\\launcher.exe");
        let exes = vec![
            PathBuf::from("C:\\Windows\\explorer.exe"),
            PathBuf::from("C:\\WINDOWS\\System32\\svchost.exe"),
            PathBuf::from("C:\\Users\\u\\AppData\\Roaming\\Telegram Desktop\\Telegram.exe"),
            PathBuf::from("C:\\Users\\u\\AppData\\Roaming\\Telegram Desktop\\Telegram.exe"),
            PathBuf::from("C:\\Program Files\\Launcher\\launcher.exe"),
            PathBuf::from("C:\\Program Files\\Mozilla Firefox\\firefox.exe"),
            PathBuf::from("C:\\x\\mihomo.exe"),
            PathBuf::from("/usr/lib/systemd/systemd-logind"),
            PathBuf::from("/usr/libexec/gsd-power"),
            PathBuf::from("/usr/bin/bash"),
            PathBuf::from("/opt/discord/Discord"),
        ];
        let out = collect(exes, Some(own), Some(root));
        let names: Vec<&str> = out.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Discord", "firefox", "Telegram"]);
        assert_eq!(out[2].process, "Telegram.exe");
        assert!(out[2].path.as_deref().unwrap().ends_with("Telegram.exe"));
    }

    #[cfg(unix)]
    #[test]
    fn picker_skips_system_and_dedups() {
        let own = Path::new("/opt/launcher/launcher");
        let exes = vec![
            PathBuf::from("/usr/lib/systemd/systemd-logind"),
            PathBuf::from("/usr/libexec/gsd-power"),
            PathBuf::from("/usr/bin/bash"),
            PathBuf::from("/usr/bin/gvfsd-fuse"),
            PathBuf::from("/opt/launcher/launcher"),
            PathBuf::from("/opt/discord/Discord"),
            PathBuf::from("/opt/discord/Discord"),
            PathBuf::from("/usr/lib/firefox/firefox"),
        ];
        let out = collect(exes, Some(own), None);
        let names: Vec<&str> = out.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Discord", "firefox"]);
        assert_eq!(out[0].path.as_deref(), Some("/opt/discord/Discord"));
    }

    #[test]
    fn running_lists_without_panicking() {
        let me = std::env::current_exe().unwrap();
        let list = running();
        let own = me.to_string_lossy().to_lowercase();
        assert!(list
            .iter()
            .all(|a| a.path.as_deref().map(str::to_lowercase) != Some(own.clone())));
    }
}
