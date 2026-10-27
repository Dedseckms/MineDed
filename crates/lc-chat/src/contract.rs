//! CONTRACT (lead). Client of hub `/api/v1/chat` (REST + WebSocket), used by
//! `src-tauri`. Auth: Yggdrasil bearer of the own-server account (never logged).
//! The hub enforces roles and privacy; the client only mirrors them in the UI.

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::social::{
    AdminServerState, ModAction, SocialAudience, SocialChannel, SocialChannelId, SocialContent,
    SocialEvent, SocialFriend, SocialHistory, SocialMessage, SocialPrivacy, SocialReport,
    SocialRole, SocialUser, StorageTarget,
};
use lc_types::{AccountId, AppResult};

/// Receives pushed events (new messages, presence, friends, admin state).
pub trait SocialObserver: Send + Sync {
    fn on_event(&self, event: SocialEvent);
}

#[async_trait]
pub trait ChatService: Send + Sync {
    /// Connects the WebSocket for `account` (own-server account; others →
    /// `InvalidInput` reason `needsOwnServerAccount`) and keeps it alive with
    /// reconnects until `disconnect`. Returns the caller's profile (with role).
    async fn connect(&self, account: AccountId) -> AppResult<SocialUser>;
    async fn disconnect(&self) -> AppResult<()>;

    /// Global chat first, then direct chats by last activity.
    async fn channels(&self) -> AppResult<Vec<SocialChannel>>;
    /// History page, newest last; `before` from a previous page. Cached on disk.
    async fn history(
        &self,
        channel: &SocialChannelId,
        before: Option<String>,
    ) -> AppResult<SocialHistory>;
    async fn mark_read(&self, channel: &SocialChannelId, message_id: &str) -> AppResult<()>;
    /// Text, room invite or pack share. Rejected public messages come back as
    /// `InvalidInput` reason `filtered` / `muted`.
    async fn send(
        &self,
        channel: &SocialChannelId,
        content: SocialContent,
    ) -> AppResult<SocialMessage>;
    /// Uploads a local file (≤ 10 MB) and posts it with an optional caption.
    async fn send_file(
        &self,
        channel: &SocialChannelId,
        path: &str,
        caption: Option<String>,
        sink: &dyn ProgressSink,
    ) -> AppResult<SocialMessage>;
    /// Own messages only (moderators: any public message via `moderate_delete`).
    async fn delete_message(&self, message_id: &str) -> AppResult<()>;
    /// Downloads an attachment to `dest` (a path picked by the user).
    async fn save_attachment(
        &self,
        attachment_id: &str,
        dest: &str,
        sink: &dyn ProgressSink,
    ) -> AppResult<()>;

    /// Opens (or returns) the direct chat with `uuid`; `PermissionDenied` when
    /// their privacy settings or a block forbid it.
    async fn open_direct(&self, uuid: &str) -> AppResult<SocialChannel>;
    async fn search_users(&self, query: &str) -> AppResult<Vec<SocialUser>>;
    async fn profile(&self, uuid: &str) -> AppResult<SocialUser>;

    async fn friends(&self) -> AppResult<Vec<SocialFriend>>;
    /// Sends a request, or accepts an incoming one.
    async fn add_friend(&self, uuid: &str) -> AppResult<SocialFriend>;
    /// Removes a friend or declines/cancels a request.
    async fn remove_friend(&self, uuid: &str) -> AppResult<()>;
    async fn set_blocked(&self, uuid: &str, blocked: bool) -> AppResult<()>;

    async fn privacy(&self) -> AppResult<SocialPrivacy>;
    async fn set_privacy(&self, privacy: SocialPrivacy) -> AppResult<SocialPrivacy>;

    async fn report(&self, message_id: &str, reason: Option<String>) -> AppResult<()>;

    // ------------------------------------------------ moderators and admins
    // The hub refuses these with `PermissionDenied` for plain users.

    async fn reports(&self) -> AppResult<Vec<SocialReport>>;
    async fn resolve_report(&self, report_id: &str) -> AppResult<()>;
    async fn moderate_delete(&self, message_id: &str) -> AppResult<()>;
    /// `seconds` for `Mute` (moderators ≤ 86 400); ignored otherwise.
    async fn moderate_user(
        &self,
        uuid: &str,
        action: ModAction,
        seconds: Option<u64>,
    ) -> AppResult<()>;
    /// Admin only: `Moderator` or `User`.
    async fn set_role(&self, uuid: &str, role: SocialRole) -> AppResult<()>;
    /// Admin only.
    async fn server_state(&self) -> AppResult<AdminServerState>;
    /// Admin only: moves attachments to a new disk or object storage; progress
    /// via `SocialEvent::Admin`. Data is deleted from the old place only after
    /// the copy is verified.
    async fn migrate_storage(&self, target: StorageTarget) -> AppResult<()>;
}

/// Default privacy for new users: everyone may message and invite, everyone
/// sees the status, everyone may send friend requests.
pub fn default_privacy() -> SocialPrivacy {
    SocialPrivacy {
        who_can_message: SocialAudience::Everyone,
        who_can_invite: SocialAudience::Everyone,
        who_sees_status: SocialAudience::Everyone,
        who_can_add_friend: SocialAudience::Everyone,
    }
}
