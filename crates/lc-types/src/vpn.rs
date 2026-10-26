use serde::{Deserialize, Serialize};
use specta::Type;

use crate::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum VpnMode {
    Off,
    /// mihomo mixed-port; only launcher HTTP goes through it. No admin rights.
    LauncherOnly,
    /// Same mixed-port, plus the game's Java is pointed at it via JVM proxy
    /// properties (`-DsocksProxyHost` etc.). No TUN, no admin rights.
    LauncherAndGame,
    /// mihomo TUN: the launcher, the game and the programs from
    /// `VpnController::apps` go through the VPN, everything else direct.
    /// Needs admin rights (UAC on Windows, CAP_NET_ADMIN on Linux).
    Apps,
}

/// A program routed through the VPN in `VpnMode::Apps`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VpnApp {
    /// Display name ("Telegram").
    pub name: String,
    /// Executable file name matched by mihomo `PROCESS-NAME` ("Telegram.exe").
    pub process: String,
    /// Full path when picked from disk or a running process.
    pub path: Option<String>,
}

/// Personal MineDed VPN access from hub `/me/vpn`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VpnAccess {
    pub quota_bytes: u64,
    pub used_bytes: u64,
    /// RFC 3339 of the next monthly reset.
    pub reset_at: Option<String>,
    /// False when the quota is used up or the key is disabled.
    pub enabled: bool,
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
