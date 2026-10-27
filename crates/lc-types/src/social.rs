//! Global chat, direct messages, friends and moderation (hub `/chat`, plan in
//! `docs/plans/chat.md`). Names are prefixed `Social` so they don't clash with
//! the room chat types in `voice`.

use serde::{Deserialize, Serialize};
use specta::Type;

/// `global` or `dm:<uuidA>:<uuidB>` (sorted undashed profile UUIDs).
pub type SocialChannelId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SocialChannelKind {
    Global,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SocialChannel {
    pub id: SocialChannelId,
    pub kind: SocialChannelKind,
    /// The other participant of a direct chat.
    pub peer: Option<SocialUser>,
    pub last_message: Option<SocialMessage>,
    pub unread: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SocialRole {
    User,
    /// Chat moderation only (assigned by an admin).
    Moderator,
    /// Drasl administrator: everything, including roles and server tools.
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SocialStatus {
    Offline,
    Online,
    InGame,
    InRoom,
}

/// Public view of a player (respects their privacy settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SocialUser {
    /// Undashed profile UUID.
    pub uuid: String,
    pub name: String,
    pub skin_url: Option<String>,
    /// `None` when hidden by the player's privacy settings.
    pub status: Option<SocialStatus>,
    pub role: SocialRole,
    pub is_friend: bool,
    /// The caller may open a direct chat with this player.
    pub can_message: bool,
    pub can_invite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SocialContent {
    Text {
        text: String,
    },
    /// Image or file uploaded to hub (≤ 10 MB).
    Attachment {
        attachment: SocialAttachment,
        caption: Option<String>,
    },
    /// Invitation to a room (`mineded://join/<code>`).
    RoomInvite {
        code: String,
        link: String,
    },
    /// A modpack offered over P2P while the sender is online (direct chats only).
    PackShare {
        name: String,
        mc: String,
        loader: String,
        mods: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SocialAttachment {
    pub id: String,
    pub file_name: String,
    pub mime: String,
    pub size: u64,
    /// Full file / image URL (authorized download through hub).
    pub url: String,
    /// Small preview for images.
    pub thumb_url: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SocialMessage {
    /// Monotonic per channel; also the history cursor.
    pub id: String,
    pub channel: SocialChannelId,
    pub author: SocialUser,
    pub content: SocialContent,
    /// RFC 3339.
    pub created_at: String,
    pub edited_at: Option<String>,
    /// Removed by the author or a moderator (content is emptied).
    pub deleted: bool,
}

/// A page of history, newest last. `before` = pass to load older messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SocialHistory {
    pub messages: Vec<SocialMessage>,
    pub before: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SocialAudience {
    Everyone,
    Friends,
    Nobody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SocialPrivacy {
    pub who_can_message: SocialAudience,
    /// `Nobody` is treated as `Friends` for invites.
    pub who_can_invite: SocialAudience,
    pub who_sees_status: SocialAudience,
    /// `Friends` is treated as `Nobody`.
    pub who_can_add_friend: SocialAudience,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum FriendState {
    Friends,
    /// The caller sent a request.
    Outgoing,
    /// The caller received a request.
    Incoming,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SocialFriend {
    pub user: SocialUser,
    pub state: FriendState,
}

/// Events pushed by hub over the chat WebSocket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SocialEvent {
    Message {
        message: SocialMessage,
    },
    MessageUpdated {
        message: SocialMessage,
    },
    Presence {
        uuid: String,
        status: SocialStatus,
    },
    Friend {
        friend: SocialFriend,
    },
    FriendRemoved {
        uuid: String,
    },
    /// Message rejected by the public chat filter (profanity/insults) or a mute.
    Rejected {
        reason: String,
    },
    /// Admins only: disk/backup state changed (see `AdminServerState`).
    Admin {
        state: AdminServerState,
    },
    Connection {
        connected: bool,
    },
}

// ---------------------------------------------------------------- moderation

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SocialReport {
    pub id: String,
    pub message: SocialMessage,
    pub reporter: SocialUser,
    pub reason: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ModAction {
    /// Seconds; moderators are limited to 86 400.
    Mute,
    /// Admin only; permanent chat ban.
    Ban,
    Unmute,
    Unban,
}

/// Server health shown to admins (disk-full banner, backups, migrations).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AdminServerState {
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    /// New attachments are refused (≥ 95 % full).
    pub attachments_paused: bool,
    /// RFC 3339 of the last successful backup to the home server.
    pub last_backup: Option<String>,
    pub backup_error: Option<String>,
    /// A running storage migration, 0..=1.
    pub migration_progress: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum StorageTarget {
    /// A new, bigger disk attached to the VPS.
    NewDisk,
    /// S3-compatible object storage (credentials configured on the server).
    ObjectStorage,
}
