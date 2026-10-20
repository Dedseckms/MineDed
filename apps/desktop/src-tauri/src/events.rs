//! CONTRACT (lead): global events. Names are fixed via `event_name`;
//! the frontend uses `events.<camelName>.listen(...)` from `bindings.ts`.

use lc_types::account::Account;
use lc_types::events::{
    InstanceLogPayload, InstanceStatePayload, NetBlockedPayload, VoiceSpeakingPayload,
};
use lc_types::room::{Peer, RoomState};
use lc_types::system::UpdateInfo;
use lc_types::task::TaskSummary;
use lc_types::voice::ChatMessage;
use lc_types::vpn::VpnState;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "tasks://changed")]
pub struct TasksChangedEvent(pub Vec<TaskSummary>);

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "instance://state")]
pub struct InstanceStateEvent(pub InstanceStatePayload);

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "instance://log")]
pub struct InstanceLogEvent(pub InstanceLogPayload);

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "account://changed")]
pub struct AccountsChangedEvent(pub Vec<Account>);

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "vpn://state")]
pub struct VpnStateEvent(pub VpnState);

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "net://blocked")]
pub struct NetBlockedEvent(pub NetBlockedPayload);

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "room://state")]
pub struct RoomStateEvent(pub Option<RoomState>);

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "room://peer")]
pub struct RoomPeerEvent {
    pub peer: Peer,
    pub joined: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "chat://message")]
pub struct ChatMessageEvent(pub ChatMessage);

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "voice://speaking")]
pub struct VoiceSpeakingEvent(pub VoiceSpeakingPayload);

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "update://available")]
pub struct UpdateAvailableEvent(pub UpdateInfo);
