use serde::{Deserialize, Serialize};
use specta::Type;

use crate::SkinId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SkinModel {
    /// Steve, 4px arms.
    Classic,
    /// Alex, 3px arms.
    Slim,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Skin {
    pub id: SkinId,
    pub name: String,
    pub model: SkinModel,
    /// SHA-256 of the PNG, as served by the texture server.
    pub hash: String,
    pub url: String,
    /// Locally cached PNG path (for the 3D preview without network).
    pub local_path: Option<String>,
    /// Stored only in the launcher, not uploaded.
    pub local_only: bool,
}

/// A skin downloaded for preview by `SkinService::fetch_remote`, not yet in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSkin {
    /// Cached PNG; pass it to `SkinService::add`.
    pub local_path: String,
    /// Guessed from the source (profile metadata) or the PNG's arm pixels.
    pub model: SkinModel,
    /// Nickname or file name, for the library entry.
    pub suggested_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Cape {
    pub id: String,
    pub name: String,
    pub url: String,
    pub active: bool,
}
