//! Persistent VPN state under `Paths::mihomo()`. Subscription URLs carry
//! access tokens: they stay on the user's disk and never reach logs.

use std::path::{Path, PathBuf};

use lc_runtime::Paths;
use lc_types::vpn::{VpnApp, VpnMode};
use lc_types::{AppError, AppResult, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::subscription::ProxyMap;

/// `state.json`. Deliberately not `Debug` (URLs hold tokens).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Stored {
    #[serde(default)]
    pub urls: Vec<String>,
    /// Imported subscription file (re-read on refresh when it still exists).
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default = "mode_off")]
    pub mode: VpnMode,
    #[serde(default)]
    pub selected: Option<String>,
    /// RFC 3339 of the last successful refresh/import.
    #[serde(default)]
    pub updated: Option<String>,
    /// Programs routed in `VpnMode::Apps`.
    #[serde(default)]
    pub apps: Vec<VpnApp>,
    /// Mode the user had on before a restart that was not resumed
    /// automatically (`apps` needs a UAC/pkexec prompt).
    #[serde(default)]
    pub resume: Option<VpnMode>,
}

fn mode_off() -> VpnMode {
    VpnMode::Off
}

impl Default for Stored {
    fn default() -> Self {
        Self {
            urls: Vec::new(),
            file: None,
            mode: VpnMode::Off,
            selected: None,
            updated: None,
            apps: Vec::new(),
            resume: None,
        }
    }
}

pub(crate) struct Files {
    pub dir: PathBuf,
}

impl Files {
    pub fn new(paths: &Paths) -> Self {
        Self {
            dir: paths.mihomo(),
        }
    }

    pub fn state(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    pub fn nodes(&self) -> PathBuf {
        self.dir.join("nodes.json")
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join("config.json")
    }

    pub fn pid(&self) -> PathBuf {
        self.dir.join("mihomo.pid")
    }

    /// Created to make the elevated TUN helper stop mihomo and exit.
    pub fn stop_flag(&self) -> PathBuf {
        self.dir.join("tun.stop")
    }

    pub fn log(&self) -> PathBuf {
        self.dir.join("mihomo.log")
    }

    /// Missing or corrupt state → defaults (logged without content).
    pub fn load_state(&self) -> Stored {
        read_json(&self.state()).unwrap_or_default()
    }

    pub fn load_nodes(&self) -> Vec<ProxyMap> {
        read_json::<Vec<Value>>(&self.nodes())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .collect()
    }

    pub fn save_state(&self, s: &Stored) -> AppResult<()> {
        write_private(&self.state(), &to_vec(s)?)
    }

    pub fn save_nodes(&self, nodes: &[ProxyMap]) -> AppResult<()> {
        write_private(&self.nodes(), &to_vec(&nodes)?)
    }
}

fn to_vec<T: Serialize>(v: &T) -> AppResult<Vec<u8>> {
    serde_json::to_vec_pretty(v).map_err(|e| AppError::internal(format!("serialize: {e}")))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(v) => Some(v),
        Err(_) => {
            tracing::warn!(path = %path.display(), "corrupt VPN state file ignored");
            None
        }
    }
}

/// Atomic write (`*.tmp` + rename); owner-only permissions on Unix since the
/// files hold subscription tokens and node credentials.
pub(crate) fn write_private(path: &Path, data: &[u8]) -> AppResult<()> {
    let io = |e: std::io::Error| {
        AppError::new(ErrorCode::Io, e.to_string()).detail("path", path.display().to_string())
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
    }
    std::fs::rename(&tmp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::parse_link;

    #[test]
    fn roundtrip_and_defaults() {
        let dir = std::env::temp_dir().join(format!("lc-vpn-store-{}", std::process::id()));
        let f = Files::new(&Paths::new(&dir));
        assert!(f.load_state() == Stored::default());
        let s = Stored {
            urls: vec!["https://sub.example/a?token=x".into()],
            file: None,
            mode: VpnMode::LauncherAndGame,
            selected: Some("A".into()),
            updated: Some("2026-10-22T00:00:00Z".into()),
            apps: vec![VpnApp {
                name: "Telegram".into(),
                process: "Telegram.exe".into(),
                path: None,
            }],
            resume: Some(VpnMode::Apps),
        };
        f.save_state(&s).unwrap();
        assert!(f.load_state() == s);
        let nodes = vec![parse_link("trojan://pw@a.example:443#A").unwrap()];
        f.save_nodes(&nodes).unwrap();
        assert_eq!(f.load_nodes(), nodes);
        std::fs::write(f.state(), b"{not json").unwrap();
        assert!(f.load_state() == Stored::default());
        // Old files without newer fields still load.
        std::fs::write(f.state(), br#"{"mode":"launcherOnly"}"#).unwrap();
        assert_eq!(f.load_state().mode, VpnMode::LauncherOnly);
        let _ = std::fs::remove_dir_all(dir);
    }
}
