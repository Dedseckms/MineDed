//! Payloads of global events (`app.emit`). The event wrappers with names live
//! in `apps/desktop/src-tauri/src/events.rs`.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::instance::RunState;
use crate::InstanceId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstanceStatePayload {
    pub id: InstanceId,
    pub state: RunState,
    pub exit_code: Option<i32>,
    /// Path to the crash report, if one was produced.
    pub crash_report: Option<String>,
}

/// Game log lines, batched (~100 ms) to keep the UI responsive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstanceLogPayload {
    pub id: InstanceId,
    pub lines: Vec<String>,
}

/// A host looks blocked; UI may offer to enable VPN.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NetBlockedPayload {
    pub host: String,
    /// i18n key of the suggested action.
    pub suggestion_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSpeakingPayload {
    /// Room peer id, or `"self"` for the local microphone (also during mic test).
    pub peer_id: String,
    pub speaking: bool,
    /// 0.0..=1.0
    pub level: f32,
}
