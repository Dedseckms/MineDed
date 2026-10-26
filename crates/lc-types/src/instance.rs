use serde::{Deserialize, Serialize};
use specta::Type;

use crate::version::LoaderKind;
use crate::InstanceId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LoaderRef {
    pub kind: LoaderKind,
    /// `None` for vanilla.
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCfg {
    pub min_mb: u32,
    pub max_mb: u32,
    /// Recalculate `maxMb` from system RAM and mod count on every launch.
    pub auto: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JavaCfg {
    /// Explicit executable; `None` = managed runtime picked by Java major.
    pub path: Option<String>,
    pub major: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PerfPreset {
    Potato,
    Balanced,
    Quality,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    pub id: InstanceId,
    pub name: String,
    pub mc: String,
    pub loader: LoaderRef,
    pub java: JavaCfg,
    pub memory: MemoryCfg,
    pub jvm_args: Vec<String>,
    pub preset: Option<PerfPreset>,
    /// Icon file name inside the instance dir or a built-in icon key.
    pub icon: Option<String>,
    /// RFC 3339.
    pub created_at: String,
    pub last_played: Option<String>,
    pub playtime_sec: u64,
}

/// Input of `instance_create`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSpec {
    pub name: String,
    pub mc: String,
    pub loader: LoaderRef,
    pub preset: Option<PerfPreset>,
    pub icon: Option<String>,
}

/// Partial update for `instance_update_cfg`; `None` fields are left as is.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstancePatch {
    pub name: Option<String>,
    pub java: Option<JavaCfg>,
    pub memory: Option<MemoryCfg>,
    pub jvm_args: Option<Vec<String>>,
    pub icon: Option<String>,
    /// Set by `PresetService::apply` once the preset is fully applied.
    #[serde(default)]
    #[specta(optional)]
    pub preset: Option<PerfPreset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PackFormat {
    /// Modrinth `.mrpack`.
    Mrpack,
    /// CurseForge modpack zip (`manifest.json`).
    CurseForgeZip,
    /// Launcher's own zip of the whole instance dir.
    LauncherZip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RunState {
    Stopped,
    Preparing,
    Running,
    Crashed,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOpts {
    /// Account to launch with; `None` = default account.
    pub account: Option<crate::AccountId>,
    /// Join a server directly (`host:port`) via quick play.
    pub join_server: Option<String>,
    /// Launch as part of a P2P room (host or guest).
    pub room: Option<crate::RoomId>,
    /// Skip file verification (faster relaunch).
    pub offline_fast: bool,
}
