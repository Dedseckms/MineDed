//! Scrubs AppImage runtime environment from child processes.
//!
//! The AppImage `AppRun` (linuxdeploy + gtk plugin) exports variables that
//! point into the mounted AppDir. Java children inherit them, so LWJGL, GLFW
//! and OpenAL end up loading the launcher's bundled libraries instead of the
//! system ones. Every Java command is passed through [`scrub`] on Linux.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::BTreeMap;

/// Variables removed outright (or restored from a saved original).
const DROP: &[&str] = &[
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "GDK_PIXBUF_MODULE_FILE",
    "GDK_PIXBUF_MODULEDIR",
    "GDK_BACKEND",
    "GIO_MODULE_DIR",
    "GIO_EXTRA_MODULES",
    "GTK_PATH",
    "GTK_EXE_PREFIX",
    "GTK_DATA_PREFIX",
    "GTK_IM_MODULE_FILE",
    "GSETTINGS_SCHEMA_DIR",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
    "GST_PLUGIN_SCANNER",
    "QT_PLUGIN_PATH",
    "PYTHONHOME",
    "PYTHONPATH",
    "PERLLIB",
    "APPDIR",
    "APPIMAGE",
    "ARGV0",
    "OWD",
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "WEBKIT_DISABLE_COMPOSITING_MODE",
];

/// Path lists from which only entries under `$APPDIR` are removed.
const FILTER: &[&str] = &["PATH", "XDG_DATA_DIRS"];

/// One change to apply to a child's environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnvOp {
    Set(String, String),
    Remove(String),
}

/// Original value saved by the AppImage runtime, if any
/// (`APPIMAGE_ORIGINAL_<VAR>` or `<VAR>_ORIG`).
fn original<'a>(env: &'a BTreeMap<String, String>, var: &str) -> Option<&'a str> {
    env.get(&format!("APPIMAGE_ORIGINAL_{var}"))
        .or_else(|| env.get(&format!("{var}_ORIG")))
        .map(String::as_str)
}

fn set_or_remove(var: &str, value: &str) -> EnvOp {
    if value.is_empty() {
        EnvOp::Remove(var.to_owned())
    } else {
        EnvOp::Set(var.to_owned(), value.to_owned())
    }
}

fn under(entry: &str, appdir: &str) -> bool {
    entry == appdir
        || entry
            .strip_prefix(appdir)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Computes the environment fixups for a child process. Pure: `env` is a
/// snapshot of the parent environment. Returns nothing outside an AppImage
/// (neither `APPIMAGE` nor `APPDIR` set).
pub(crate) fn scrub(env: &BTreeMap<String, String>) -> Vec<EnvOp> {
    if !env.contains_key("APPIMAGE") && !env.contains_key("APPDIR") {
        return Vec::new();
    }
    let mut ops = Vec::new();
    for var in DROP {
        if let Some(orig) = original(env, var) {
            ops.push(set_or_remove(var, orig));
        } else if env.contains_key(*var) {
            ops.push(EnvOp::Remove((*var).to_owned()));
        }
    }
    let appdir = env
        .get("APPDIR")
        .map(|d| d.trim_end_matches('/'))
        .filter(|d| !d.is_empty());
    for var in FILTER {
        if let Some(orig) = original(env, var) {
            ops.push(set_or_remove(var, orig));
            continue;
        }
        let (Some(value), Some(appdir)) = (env.get(*var), appdir) else {
            continue;
        };
        let kept: Vec<&str> = value
            .split(':')
            .filter(|e| !e.is_empty() && !under(e, appdir))
            .collect();
        let joined = kept.join(":");
        if joined != *value {
            ops.push(set_or_remove(var, &joined));
        }
    }
    ops
}

/// Applies [`scrub`] of the current process environment to `cmd`.
/// Must run before any explicit `.envs(..)` so plan-provided values win.
#[cfg(target_os = "linux")]
pub(crate) fn apply(cmd: &mut tokio::process::Command) {
    let env: BTreeMap<String, String> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    for op in scrub(&env) {
        match op {
            EnvOp::Set(k, v) => {
                cmd.env(k, v);
            }
            EnvOp::Remove(k) => {
                cmd.env_remove(k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn rm(k: &str) -> EnvOp {
        EnvOp::Remove(k.to_owned())
    }

    fn set(k: &str, v: &str) -> EnvOp {
        EnvOp::Set(k.to_owned(), v.to_owned())
    }

    #[test]
    fn noop_outside_appimage() {
        let e = env(&[
            ("LD_LIBRARY_PATH", "/opt/x"),
            ("PATH", "/usr/bin"),
            ("GTK_PATH", "/g"),
        ]);
        assert!(scrub(&e).is_empty());
    }

    #[test]
    fn drops_appimage_vars() {
        let e = env(&[
            ("APPIMAGE", "/home/u/L.AppImage"),
            ("APPDIR", "/tmp/.mount_L"),
            ("LD_LIBRARY_PATH", "/tmp/.mount_L/usr/lib"),
            ("GDK_PIXBUF_MODULE_FILE", "/tmp/.mount_L/x"),
            ("HOME", "/home/u"),
        ]);
        let ops = scrub(&e);
        assert!(ops.contains(&rm("LD_LIBRARY_PATH")));
        assert!(ops.contains(&rm("GDK_PIXBUF_MODULE_FILE")));
        assert!(ops.contains(&rm("APPDIR")));
        assert!(ops.contains(&rm("APPIMAGE")));
        assert!(!ops
            .iter()
            .any(|o| matches!(o, EnvOp::Remove(k) if k == "HOME")));
        assert!(!ops.contains(&rm("GTK_PATH")), "absent vars untouched");
    }

    #[test]
    fn appdir_alone_triggers() {
        let e = env(&[("APPDIR", "/tmp/.mount_L"), ("LD_PRELOAD", "x.so")]);
        let ops = scrub(&e);
        assert!(ops.contains(&rm("LD_PRELOAD")));
    }

    #[test]
    fn restores_originals() {
        let e = env(&[
            ("APPIMAGE", "/a.AppImage"),
            ("LD_LIBRARY_PATH", "/tmp/.mount_L/usr/lib"),
            ("APPIMAGE_ORIGINAL_LD_LIBRARY_PATH", "/opt/cuda/lib"),
            ("GTK_PATH", "/tmp/.mount_L/gtk"),
            ("GTK_PATH_ORIG", "/usr/lib/gtk"),
            ("QT_PLUGIN_PATH", "/tmp/.mount_L/qt"),
            ("QT_PLUGIN_PATH_ORIG", ""),
        ]);
        let ops = scrub(&e);
        assert!(ops.contains(&set("LD_LIBRARY_PATH", "/opt/cuda/lib")));
        assert!(ops.contains(&set("GTK_PATH", "/usr/lib/gtk")));
        assert!(ops.contains(&rm("QT_PLUGIN_PATH")));
    }

    #[test]
    fn filters_path_lists() {
        let e = env(&[
            ("APPIMAGE", "/a.AppImage"),
            ("APPDIR", "/tmp/.mount_L/"),
            (
                "PATH",
                "/tmp/.mount_L/usr/bin:/usr/bin:/tmp/.mount_Lother/bin:/bin",
            ),
            ("XDG_DATA_DIRS", "/tmp/.mount_L/usr/share:/tmp/.mount_L"),
        ]);
        let ops = scrub(&e);
        assert!(ops.contains(&set("PATH", "/usr/bin:/tmp/.mount_Lother/bin:/bin")));
        assert!(ops.contains(&rm("XDG_DATA_DIRS")));
    }

    #[test]
    fn path_untouched_when_clean_or_no_appdir() {
        let clean = env(&[("APPDIR", "/tmp/.mount_L"), ("PATH", "/usr/bin:/bin")]);
        assert!(scrub(&clean).iter().all(|o| !matches!(o,
            EnvOp::Set(k, _) | EnvOp::Remove(k) if k == "PATH")));
        let no_dir = env(&[("APPIMAGE", "/a"), ("PATH", "/tmp/.mount_L/bin:/usr/bin")]);
        assert!(scrub(&no_dir).iter().all(|o| !matches!(o,
            EnvOp::Set(k, _) | EnvOp::Remove(k) if k == "PATH")));
    }

    #[test]
    fn path_original_wins() {
        let e = env(&[
            ("APPDIR", "/tmp/.mount_L"),
            ("PATH", "/tmp/.mount_L/bin:/usr/bin"),
            ("APPIMAGE_ORIGINAL_PATH", "/usr/local/bin:/usr/bin"),
        ]);
        assert!(scrub(&e).contains(&set("PATH", "/usr/local/bin:/usr/bin")));
    }
}
