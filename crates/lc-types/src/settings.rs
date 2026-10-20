use serde::{Deserialize, Serialize};
use specta::Type;

use crate::instance::MemoryCfg;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    Ru,
    En,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    System,
    Dark,
    Light,
    /// Named custom theme bundled with the app.
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ProxyKind {
    Http,
    Socks5,
}

/// Manual proxy for launcher HTTP traffic (independent of the built-in VPN).
/// The password crosses IPC (settings form) but never appears in `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProxyCfg {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl std::fmt::Debug for ProxyCfg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyCfg")
            .field("kind", &self.kind)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_debug_hides_password() {
        let p = ProxyCfg {
            kind: ProxyKind::Socks5,
            host: "h".into(),
            port: 1080,
            username: Some("u".into()),
            password: Some("hunter2".into()),
        };
        assert!(!format!("{p:?}").contains("hunter2"));
    }
}

/// Global launcher settings (`settings_get` / `settings_set`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub language: Language,
    pub theme: Theme,
    /// No blur, reduced motion, fewer previews. Auto-enabled on weak hardware.
    pub light_mode: bool,
    pub reduce_motion: bool,
    /// Root for instances, libraries, assets. `None` = OS default data dir.
    pub game_dir: Option<String>,
    pub default_memory: MemoryCfg,
    /// Global Java override; per-instance config wins.
    pub java_path: Option<String>,
    pub default_jvm_args: Vec<String>,
    pub max_parallel_downloads: u32,
    /// Prefer mirrors (BMCLAPI, own mirror) over official hosts.
    pub prefer_mirrors: bool,
    /// Manual proxy; `None` = direct (or built-in VPN when enabled).
    pub proxy: Option<ProxyCfg>,
    pub close_on_launch: bool,
    pub auto_update: bool,
    /// Hub API root, e.g. `https://mc.example.ru/api/v1`.
    pub hub_url: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::Ru,
            theme: Theme::System,
            light_mode: false,
            reduce_motion: false,
            game_dir: None,
            default_memory: MemoryCfg {
                min_mb: 512,
                max_mb: 4096,
                auto: true,
            },
            java_path: None,
            default_jvm_args: Vec::new(),
            max_parallel_downloads: 16,
            prefer_mirrors: false,
            proxy: None,
            close_on_launch: false,
            auto_update: true,
            hub_url: String::new(),
        }
    }
}
