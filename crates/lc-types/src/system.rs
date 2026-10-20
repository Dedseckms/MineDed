use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    /// `windows`, `linux`.
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_threads: u32,
    pub total_ram_mb: u64,
    pub free_ram_mb: u64,
    pub gpu: Option<String>,
    /// Heuristic: few threads / little RAM / iGPU.
    pub weak_hardware: bool,
    /// Linux only: running under Wayland (no global PTT hotkeys).
    pub wayland: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecommendation {
    pub min_mb: u32,
    pub max_mb: u32,
    /// i18n key explaining the choice.
    pub reason_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: String,
    /// RFC 3339.
    pub date: Option<String>,
}
