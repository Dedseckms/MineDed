use serde::{Deserialize, Serialize};
use specta::Type;

use crate::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum VpnMode {
    Off,
    /// mihomo mixed-port; only launcher HTTP goes through it. No admin rights.
    LauncherOnly,
    /// mihomo TUN with per-process rules for the launcher and its Java. Needs elevation.
    LauncherAndGame,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VpnState {
    pub mode: VpnMode,
    pub running: bool,
    pub node: Option<String>,
    pub latency_ms: Option<u32>,
    /// RFC 3339 of the last successful subscription refresh.
    pub subscription_updated: Option<String>,
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VpnNode {
    pub name: String,
    /// `vless`, `hysteria2`, `trojan`...
    pub protocol: String,
    pub latency_ms: Option<u32>,
    pub selected: bool,
}
