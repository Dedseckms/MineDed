use serde::{Deserialize, Serialize};
use specta::Type;

use crate::version::LoaderKind;
use crate::RoomId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RoomRole {
    Host,
    Guest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PathKind {
    /// Hole punching succeeded.
    Direct,
    /// Traffic goes through our iroh-relay.
    Relay,
    Connecting,
}

/// How the host's integrated server checks players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RoomAuthMode {
    /// Our Yggdrasil with Mojang fallback (MS + own-server accounts, skins work).
    Yggdrasil,
    /// No checks; offline accounts allowed, no skins for them.
    Open,
}

/// What guests must match to join (same game, loader and mod set).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSig {
    pub mc: String,
    pub loader: LoaderKind,
    pub loader_version: Option<String>,
    /// SHA-256 over sorted mod file hashes.
    pub mods_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    /// iroh endpoint id.
    pub peer_id: String,
    pub name: String,
    pub uuid: Option<String>,
    pub path: PathKind,
    pub rtt_ms: Option<u32>,
    pub speaking: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoomState {
    pub room_id: RoomId,
    /// Human code, e.g. `ABCD-1234`.
    pub code: String,
    pub role: RoomRole,
    pub auth_mode: RoomAuthMode,
    pub sig: InstanceSig,
    pub peers: Vec<Peer>,
    /// RFC 3339.
    pub expires_at: String,
}
