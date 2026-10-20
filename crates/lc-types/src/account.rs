use serde::{Deserialize, Serialize};
use specta::Type;

use crate::AccountId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AccountKind {
    // Microsoft accounts are intentionally not supported (owner decision 2026-10-20).
    Offline,
    /// Any authlib-injector compatible server: own skin server, Ely.by...
    Yggdrasil,
}

/// Public account info. Tokens never leave Rust (they live in the keyring).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: AccountId,
    pub kind: AccountKind,
    pub name: String,
    /// Minecraft profile UUID (dashed).
    pub uuid: String,
    /// API root of the Yggdrasil server, for `yggdrasil` accounts.
    pub server: Option<String>,
    pub skin_url: Option<String>,
    pub is_default: bool,
    /// `false` = refresh failed, re-login required.
    pub valid: bool,
}

/// Known Yggdrasil servers offered in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct YggdrasilServer {
    pub name: String,
    pub api_root: String,
    /// Own server of the project (default choice).
    pub official: bool,
}
