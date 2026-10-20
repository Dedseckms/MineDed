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
            close_on_launch: false,
            auto_update: true,
            hub_url: String::new(),
        }
    }
}
