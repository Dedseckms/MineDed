//! Bridges from service observer traits to Tauri events. Handed to real
//! services when they replace stubs in `state.rs`.

use lc_core::GameObserver;
use lc_p2p::RoomObserver;
use lc_types::events::{InstanceLogPayload, InstanceStatePayload, VoiceSpeakingPayload};
use lc_types::room::{Peer, RoomState};
use lc_types::voice::ChatMessage;
use lc_voice::VoiceObserver;
use tauri::AppHandle;
use tauri_specta::Event;

use crate::events::{
    ChatMessageEvent, InstanceLogEvent, InstanceStateEvent, RoomPeerEvent, RoomStateEvent,
    VoiceSpeakingEvent,
};

pub struct EventObserver {
    app: AppHandle,
}

impl EventObserver {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

// Emission failures mean no window is listening; nothing to recover.
impl GameObserver for EventObserver {
    fn on_state(&self, payload: InstanceStatePayload) {
        let _ = InstanceStateEvent(payload).emit(&self.app);
    }

    fn on_log(&self, payload: InstanceLogPayload) {
        let _ = InstanceLogEvent(payload).emit(&self.app);
    }
}

impl RoomObserver for EventObserver {
    fn on_state(&self, state: Option<RoomState>) {
        let _ = RoomStateEvent(state).emit(&self.app);
    }

    fn on_peer(&self, peer: Peer, joined: bool) {
        let _ = RoomPeerEvent { peer, joined }.emit(&self.app);
    }

    fn on_chat(&self, message: ChatMessage) {
        let _ = ChatMessageEvent(message).emit(&self.app);
    }
}

impl VoiceObserver for EventObserver {
    fn on_speaking(&self, payload: VoiceSpeakingPayload) {
        let _ = VoiceSpeakingEvent(payload).emit(&self.app);
    }
}
