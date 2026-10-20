//! CONTRACT (lead): `AuthProvider` is consumed by core; services by `src-tauri`.

use std::path::PathBuf;

use async_trait::async_trait;
use lc_types::account::{Account, YggdrasilServer};
use lc_types::skin::{Cape, Skin, SkinModel};
use lc_types::{AccountId, AppResult, SkinId};
use url::Url;

/// String that never shows up in `Debug` output or logs.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// `--userType` game argument. Microsoft accounts are out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserType {
    /// Offline accounts.
    Legacy,
    /// Yggdrasil (authlib-injector) accounts.
    Mojang,
}

/// Everything needed to run `-javaagent:authlib-injector.jar=<api_root>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthlibInjector {
    pub jar: PathBuf,
    pub api_root: Url,
    /// Base64 of the API metadata, passed as `-Dauthlibinjector.yggdrasil.prefetched`.
    pub prefetched: Option<String>,
}

/// Credentials for one launch. Produced right before launch (tokens refreshed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchAuth {
    pub name: String,
    /// Undashed or dashed profile UUID.
    pub uuid: String,
    pub access_token: Secret,
    pub user_type: UserType,
    pub authlib: Option<AuthlibInjector>,
}

/// Consumed by `lc-core` at launch.
#[async_trait]
pub trait AuthProvider: Send + Sync {
    /// `None` = default account. Refreshes tokens when needed.
    async fn launch_auth(&self, account: Option<AccountId>) -> AppResult<LaunchAuth>;
}

#[async_trait]
pub trait AccountService: Send + Sync {
    async fn list(&self) -> AppResult<Vec<Account>>;
    async fn offline_add(&self, name: &str) -> AppResult<Account>;
    async fn yggdrasil_login(
        &self,
        api_root: &Url,
        username: &str,
        password: &str,
    ) -> AppResult<Account>;
    async fn yggdrasil_servers(&self) -> AppResult<Vec<YggdrasilServer>>;
    async fn remove(&self, id: AccountId) -> AppResult<()>;
    async fn set_default(&self, id: AccountId) -> AppResult<()>;
}

/// OS keyring abstraction (Windows Credential Manager / Secret Service), with
/// an encrypted-file fallback. Store refresh tokens only (Windows ~2.5 KB limit).
#[async_trait]
pub trait TokenStore: Send + Sync {
    async fn get(&self, key: &str) -> AppResult<Option<Secret>>;
    async fn set(&self, key: &str, value: &Secret) -> AppResult<()>;
    async fn delete(&self, key: &str) -> AppResult<()>;
}

/// Skin library (local + own skin server) and capes.
#[async_trait]
pub trait SkinService: Send + Sync {
    async fn library(&self) -> AppResult<Vec<Skin>>;
    /// Validates PNG (64x64 / 64x32) and stores it in the library.
    async fn add(&self, png_path: &str, name: &str, model: SkinModel) -> AppResult<Skin>;
    async fn remove(&self, id: SkinId) -> AppResult<()>;
    /// Uploads to the account's server (own Yggdrasil / Mojang) and selects it.
    async fn apply(&self, account: AccountId, skin: SkinId) -> AppResult<()>;
    async fn capes(&self, account: AccountId) -> AppResult<Vec<Cape>>;
    async fn set_cape(&self, account: AccountId, cape_id: Option<String>) -> AppResult<()>;
}
