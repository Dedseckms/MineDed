use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum VoiceMode {
    Off,
    PushToTalk,
    /// Voice activity detection (fallback on Wayland where global keys are unavailable).
    Vad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AudioDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub direction: AudioDirection,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSettings {
    pub mode: VoiceMode,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// Key code in `keyboard-types` naming, e.g. `KeyV`, `CapsLock`.
    pub ptt_key: Option<String>,
    /// 0.0..=2.0
    pub input_gain: f32,
    pub output_volume: f32,
    pub noise_suppression: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub from_peer: String,
    pub from_name: String,
    pub text: String,
    /// RFC 3339.
    pub sent_at: String,
    /// Delivered via hub WebSocket instead of P2P.
    pub via_relay: bool,
}
