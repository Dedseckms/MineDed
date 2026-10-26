//! Persistent VPN state under `Paths::mihomo()`. Subscription URLs carry
//! access tokens: they stay on the user's disk and never reach logs.

use std::path::{Path, PathBuf};

use lc_runtime::Paths;
use lc_types::vpn::{VpnApp, VpnMode, VpnSource};
use lc_types::{AppError, AppResult, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::subscription::ProxyMap;

/// One subscription (MineDed or the user's own). Not `Debug` (URLs hold tokens).
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceStore {
    #[serde(default)]
    pub urls: Vec<String>,
    /// Imported subscription file (re-read on refresh when it still exists).
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub selected: Option<String>,
    /// RFC 3339 of the last successful refresh/import.
    #[serde(default)]
    pub updated: Option<String>,
}

impl SourceStore {
    pub fn configured(&self) -> bool {
        !self.urls.is_empty() || self.file.is_some()
    }
}

/// `state.json`. Deliberately not `Debug` (URLs hold tokens).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Stored {
    /// Active subscription.
    #[serde(default)]
    pub source: Option<VpnSource>,
    #[serde(default)]
    pub own: SourceStore,
    #[serde(default)]
    pub mineded: SourceStore,
    /// Pre-`VpnSource` single subscription; read once and moved to `own`.
    #[serde(default, skip_serializing, rename = "urls")]
    legacy_urls: Vec<String>,
    #[serde(default, skip_serializing, rename = "file")]
    legacy_file: Option<String>,
    #[serde(default, skip_serializing, rename = "selected")]
    legacy_selected: Option<String>,
    #[serde(default, skip_serializing, rename = "updated")]
    legacy_updated: Option<String>,
    #[serde(default = "mode_off")]
    pub mode: VpnMode,
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
            source: None,
            own: SourceStore::default(),
            mineded: SourceStore::default(),
            legacy_urls: Vec::new(),
            legacy_file: None,
            legacy_selected: None,
            legacy_updated: None,
            mode: VpnMode::Off,
            apps: Vec::new(),
            resume: None,
        }
    }
}

impl Stored {
    pub fn get(&self, source: VpnSource) -> &SourceStore {
        match source {
            VpnSource::Mineded => &self.mineded,
            VpnSource::Own => &self.own,
        }
    }

    pub fn get_mut(&mut self, source: VpnSource) -> &mut SourceStore {
        match source {
            VpnSource::Mineded => &mut self.mineded,
            VpnSource::Own => &mut self.own,
        }
    }

    /// The active source's subscription (empty when none is active).
    pub fn active(&self) -> Option<&SourceStore> {
        self.source.map(|s| self.get(s))
    }

    /// Moves a pre-`VpnSource` subscription into `own` (MineDed access used
    /// to overwrite it; `MihomoVpn::mineded_access` moves it back to
    /// `mineded` when its URL matches) and repairs a dangling `source`.
    fn migrate(&mut self) {
        let legacy = SourceStore {
            urls: std::mem::take(&mut self.legacy_urls),
            file: self.legacy_file.take(),
            selected: self.legacy_selected.take(),
            updated: self.legacy_updated.take(),
        };
        if legacy.configured() && !self.own.configured() && !self.mineded.configured() {
            self.own = legacy;
        }
        self.source = match self.source {
            Some(s) if self.get(s).configured() => Some(s),
            _ if self.own.configured() => Some(VpnSource::Own),
            _ if self.mineded.configured() => Some(VpnSource::Mineded),
            _ => None,
        };
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

    /// Cached nodes of `source` (`nodes.json` keeps the pre-`VpnSource` name
    /// for the user's own subscription).
    pub fn nodes(&self, source: VpnSource) -> PathBuf {
        self.dir.join(match source {
            VpnSource::Own => "nodes.json",
            VpnSource::Mineded => "nodes-mineded.json",
        })
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join("config.json")
    }

    pub fn pid(&self) -> PathBuf {
        self.dir.join("mihomo.pid")
    }

    /// Created to make the elevated TUN helper stop mihomo and exit.
    /// The helper exists only on Windows; Linux uses setcap instead.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn stop_flag(&self) -> PathBuf {
        self.dir.join("tun.stop")
    }

    pub fn log(&self) -> PathBuf {
        self.dir.join("mihomo.log")
    }

    /// Missing or corrupt state → defaults (logged without content).
    pub fn load_state(&self) -> Stored {
        let mut s: Stored = read_json(&self.state()).unwrap_or_default();
        s.migrate();
        s
    }

    /// Nodes of `source`; `None` → empty.
    pub fn load_nodes(&self, source: Option<VpnSource>) -> Vec<ProxyMap> {
        let Some(source) = source else {
            return Vec::new();
        };
        read_json::<Vec<Value>>(&self.nodes(source))
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

    pub fn save_nodes(&self, source: VpnSource, nodes: &[ProxyMap]) -> AppResult<()> {
        write_private(&self.nodes(source), &to_vec(&nodes)?)
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
            source: Some(VpnSource::Mineded),
            own: SourceStore {
                urls: vec!["https://sub.example/a?token=x".into()],
                selected: Some("A".into()),
                updated: Some("2026-10-22T00:00:00Z".into()),
                ..SourceStore::default()
            },
            mineded: SourceStore {
                urls: vec!["https://m.example/s/t".into()],
                ..SourceStore::default()
            },
            mode: VpnMode::LauncherAndGame,
            apps: vec![VpnApp {
                name: "Telegram".into(),
                process: "Telegram.exe".into(),
                path: None,
            }],
            resume: Some(VpnMode::Apps),
            ..Stored::default()
        };
        f.save_state(&s).unwrap();
        assert!(f.load_state() == s);
        let nodes = vec![parse_link("trojan://pw@a.example:443#A").unwrap()];
        f.save_nodes(VpnSource::Own, &nodes).unwrap();
        assert_eq!(f.load_nodes(Some(VpnSource::Own)), nodes);
        assert!(f.load_nodes(Some(VpnSource::Mineded)).is_empty());
        assert!(f.load_nodes(None).is_empty());
        std::fs::write(f.state(), b"{not json").unwrap();
        assert!(f.load_state() == Stored::default());
        // Old files without newer fields still load.
        std::fs::write(f.state(), br#"{"mode":"launcherOnly"}"#).unwrap();
        assert_eq!(f.load_state().mode, VpnMode::LauncherOnly);
        // The pre-VpnSource single subscription becomes the own one.
        std::fs::write(
            f.state(),
            br#"{"urls":["https://x.example/s"],"selected":"A","mode":"off"}"#,
        )
        .unwrap();
        let m = f.load_state();
        assert_eq!(m.source, Some(VpnSource::Own));
        assert_eq!(m.own.urls, ["https://x.example/s"]);
        assert_eq!(m.own.selected.as_deref(), Some("A"));
        assert!(!m.mineded.configured());
        f.save_state(&m).unwrap();
        assert!(f.load_state() == m);
        // A dangling source falls back to a configured one.
        std::fs::write(
            f.state(),
            br#"{"source":"own","mineded":{"urls":["https://m.example/s"]}}"#,
        )
        .unwrap();
        assert_eq!(f.load_state().source, Some(VpnSource::Mineded));
        let _ = std::fs::remove_dir_all(dir);
    }
}
