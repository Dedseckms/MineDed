//! `AccountsService`: account list in `accounts.json` + `AuthProvider` for launches.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use lc_http::Downloader;
use lc_runtime::Paths;
use lc_types::account::{Account, AccountKind, YggdrasilServer};
use lc_types::{AccountId, AppError, AppResult, ErrorCode};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use url::Url;

use crate::contract::{AccountService, AuthProvider, LaunchAuth, Secret, TokenStore, UserType};
use crate::fsutil::{read_optional, write_atomic};
use crate::offline::{offline_uuid, random_access_token, validate_offline_name};

/// File with the account list in `Paths::root()`. Contains no secrets.
pub const ACCOUNTS_FILE: &str = "accounts.json";

/// `TokenStore` key under which an account's refresh token is stored.
pub fn token_key(id: AccountId) -> String {
    format!("account:{id}")
}

/// One stored account (public data only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAccount {
    id: AccountId,
    kind: AccountKind,
    name: String,
    uuid: String,
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    skin_url: Option<String>,
    #[serde(default = "yes")]
    valid: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountsFile {
    #[serde(default = "version")]
    version: u32,
    #[serde(default)]
    default: Option<AccountId>,
    #[serde(default)]
    accounts: Vec<StoredAccount>,
}

fn version() -> u32 {
    1
}

impl AccountsFile {
    fn to_dto(&self, a: &StoredAccount) -> Account {
        Account {
            id: a.id,
            kind: a.kind,
            name: a.name.clone(),
            uuid: a.uuid.clone(),
            server: a.server.clone(),
            skin_url: a.skin_url.clone(),
            is_default: self.default == Some(a.id),
            valid: a.valid,
        }
    }

    /// Keeps `default` pointing at an existing account (first one if dangling/unset).
    fn fix_default(&mut self) {
        let ok = self
            .default
            .is_some_and(|d| self.accounts.iter().any(|a| a.id == d));
        if !ok {
            self.default = self.accounts.first().map(|a| a.id);
        }
    }
}

/// Real `AccountService` + `AuthProvider`. Offline accounts are implemented;
/// Yggdrasil is pending (needs the server), skins live in `StubSkins` for now.
pub struct AccountsService {
    file: PathBuf,
    #[allow(dead_code)] // used by the Yggdrasil client (next task)
    downloader: Arc<dyn Downloader>,
    tokens: Arc<dyn TokenStore>,
    /// Serializes read-modify-write cycles of `accounts.json`.
    lock: Mutex<()>,
}

impl std::fmt::Debug for AccountsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountsService")
            .field("file", &self.file)
            .finish_non_exhaustive()
    }
}

impl AccountsService {
    pub fn new(
        paths: &Paths,
        downloader: Arc<dyn Downloader>,
        token_store: Arc<dyn TokenStore>,
    ) -> Self {
        Self {
            file: paths.root().join(ACCOUNTS_FILE),
            downloader,
            tokens: token_store,
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> AppResult<AccountsFile> {
        let Some(bytes) = read_optional(&self.file)? else {
            return Ok(AccountsFile {
                version: version(),
                ..Default::default()
            });
        };
        let mut data: AccountsFile = serde_json::from_slice(&bytes).map_err(|e| {
            AppError::new(ErrorCode::Io, format!("accounts.json is invalid: {e}"))
                .detail("path", self.file.display().to_string())
        })?;
        data.fix_default();
        Ok(data)
    }

    fn save(&self, data: &AccountsFile) -> AppResult<()> {
        let bytes = serde_json::to_vec_pretty(data)
            .map_err(|e| AppError::internal(format!("serialize accounts: {e}")))?;
        write_atomic(&self.file, &bytes, false)
    }

    fn not_found(id: AccountId) -> AppError {
        AppError::new(ErrorCode::NotFound, "account not found").detail("id", id.to_string())
    }
}

#[async_trait]
impl AccountService for AccountsService {
    async fn list(&self) -> AppResult<Vec<Account>> {
        let _g = self.lock.lock().await;
        let data = self.load()?;
        Ok(data.accounts.iter().map(|a| data.to_dto(a)).collect())
    }

    async fn offline_add(&self, name: &str) -> AppResult<Account> {
        let name = validate_offline_name(name)?;
        let _g = self.lock.lock().await;
        let mut data = self.load()?;
        if data
            .accounts
            .iter()
            .any(|a| a.kind == AccountKind::Offline && a.name.eq_ignore_ascii_case(name))
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "offline account with this name exists",
            )
            .detail("name", name));
        }
        let account = StoredAccount {
            id: AccountId::new(),
            kind: AccountKind::Offline,
            name: name.to_owned(),
            uuid: offline_uuid(name).hyphenated().to_string(),
            server: None,
            skin_url: None,
            valid: true,
        };
        data.accounts.push(account.clone());
        data.fix_default();
        self.save(&data)?;
        Ok(data.to_dto(&account))
    }

    async fn yggdrasil_login(
        &self,
        _api_root: &Url,
        _username: &str,
        _password: &str,
    ) -> AppResult<Account> {
        Err(AppError::not_implemented("AccountService::yggdrasil_login"))
    }

    async fn yggdrasil_register(
        &self,
        _api_root: &Url,
        _invite: &str,
        _username: &str,
        _password: &str,
    ) -> AppResult<Account> {
        Err(AppError::not_implemented(
            "AccountService::yggdrasil_register",
        ))
    }

    async fn yggdrasil_servers(&self) -> AppResult<Vec<YggdrasilServer>> {
        Ok(Vec::new())
    }

    async fn remove(&self, id: AccountId) -> AppResult<()> {
        let _g = self.lock.lock().await;
        let mut data = self.load()?;
        let Some(pos) = data.accounts.iter().position(|a| a.id == id) else {
            return Err(Self::not_found(id));
        };
        // Drop the secret first so a failure never leaves an orphaned token behind.
        // Offline accounts have no token, so a broken store must not block their removal.
        if let Err(e) = self.tokens.delete(&token_key(id)).await {
            if data.accounts[pos].kind == AccountKind::Offline {
                tracing::warn!(account = %id, error = %e.message, "token delete failed");
            } else {
                return Err(e);
            }
        }
        data.accounts.remove(pos);
        data.fix_default();
        self.save(&data)
    }

    async fn set_default(&self, id: AccountId) -> AppResult<()> {
        let _g = self.lock.lock().await;
        let mut data = self.load()?;
        if !data.accounts.iter().any(|a| a.id == id) {
            return Err(Self::not_found(id));
        }
        data.default = Some(id);
        self.save(&data)
    }
}

#[async_trait]
impl AuthProvider for AccountsService {
    async fn launch_auth(&self, account: Option<AccountId>) -> AppResult<LaunchAuth> {
        let data = {
            let _g = self.lock.lock().await;
            self.load()?
        };
        let id = match account.or(data.default) {
            Some(id) => id,
            None => return Err(AppError::new(ErrorCode::AuthRequired, "no account")),
        };
        let acc = data
            .accounts
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| Self::not_found(id))?;
        match acc.kind {
            AccountKind::Offline => Ok(LaunchAuth {
                name: acc.name.clone(),
                uuid: acc.uuid.clone(),
                access_token: Secret::new(random_access_token()),
                user_type: UserType::Legacy,
                authlib: None,
            }),
            AccountKind::Yggdrasil => Err(AppError::not_implemented(
                "AuthProvider::launch_auth for Yggdrasil accounts",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil::testutil::TempDir;
    use crate::tokens::MemoryTokenStore;
    use lc_http::StubDownloader;

    fn service(dir: &TempDir, tokens: Arc<dyn TokenStore>) -> AccountsService {
        AccountsService::new(&Paths::new(dir.path()), Arc::new(StubDownloader), tokens)
    }

    fn mem() -> Arc<MemoryTokenStore> {
        Arc::new(MemoryTokenStore::new())
    }

    #[tokio::test]
    async fn crud_and_default_logic() {
        let dir = TempDir::new();
        let svc = service(&dir, mem());
        assert!(svc.list().await.unwrap().is_empty());

        let a = svc.offline_add("Steve").await.unwrap();
        assert!(a.is_default, "first account becomes default");
        assert_eq!(a.kind, AccountKind::Offline);
        assert_eq!(a.uuid, offline_uuid("Steve").hyphenated().to_string());
        assert!(a.valid && a.server.is_none());

        let b = svc.offline_add("Alex").await.unwrap();
        assert!(!b.is_default);
        let c = svc.offline_add("Herobrine").await.unwrap();

        svc.set_default(c.id).await.unwrap();
        let list = svc.list().await.unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(
            list.iter()
                .filter(|x| x.is_default)
                .map(|x| x.id)
                .collect::<Vec<_>>(),
            vec![c.id]
        );

        // Removing a non-default keeps the default.
        svc.remove(b.id).await.unwrap();
        let list = svc.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|x| x.id == c.id && x.is_default));

        // Removing the default promotes another one.
        svc.remove(c.id).await.unwrap();
        let list = svc.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].is_default && list[0].id == a.id);

        svc.remove(a.id).await.unwrap();
        assert!(svc.list().await.unwrap().is_empty());

        // Persisted across instances.
        let d = svc.offline_add("Persist").await.unwrap();
        let again = service(&dir, mem());
        assert_eq!(again.list().await.unwrap(), vec![d]);
    }

    #[tokio::test]
    async fn unknown_ids_are_not_found() {
        let dir = TempDir::new();
        let svc = service(&dir, mem());
        let id = AccountId::new();
        assert_eq!(svc.remove(id).await.unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(
            svc.set_default(id).await.unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            svc.launch_auth(Some(id)).await.unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[tokio::test]
    async fn offline_names_validated_and_unique() {
        let dir = TempDir::new();
        let svc = service(&dir, mem());
        svc.offline_add("Steve").await.unwrap();
        for dup in ["Steve", "steve", " STEVE "] {
            assert_eq!(
                svc.offline_add(dup).await.unwrap_err().code,
                ErrorCode::Conflict
            );
        }
        for bad in ["ab", "bad name", "x".repeat(17).as_str()] {
            assert_eq!(
                svc.offline_add(bad).await.unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
        assert_eq!(svc.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn remove_deletes_token() {
        let dir = TempDir::new();
        let tokens = mem();
        let svc = service(&dir, tokens.clone());
        let a = svc.offline_add("Steve").await.unwrap();
        tokens
            .set(&token_key(a.id), &Secret::new("refresh"))
            .await
            .unwrap();
        tokens.set("other", &Secret::new("keep")).await.unwrap();
        svc.remove(a.id).await.unwrap();
        assert_eq!(tokens.get(&token_key(a.id)).await.unwrap(), None);
        assert!(tokens.get("other").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn launch_auth_offline() {
        let dir = TempDir::new();
        let svc = service(&dir, mem());
        assert_eq!(
            svc.launch_auth(None).await.unwrap_err().code,
            ErrorCode::AuthRequired
        );

        let a = svc.offline_add("Steve").await.unwrap();
        let b = svc.offline_add("Alex").await.unwrap();

        let auth = svc.launch_auth(None).await.unwrap();
        assert_eq!(auth.name, "Steve");
        assert_eq!(auth.uuid, a.uuid);
        assert_eq!(auth.user_type, UserType::Legacy);
        assert!(auth.authlib.is_none());
        assert_eq!(auth.access_token.expose().len(), 32);
        assert!(auth
            .access_token
            .expose()
            .bytes()
            .all(|c| c.is_ascii_hexdigit()));
        assert!(!format!("{auth:?}").contains(auth.access_token.expose()));

        let auth = svc.launch_auth(Some(b.id)).await.unwrap();
        assert_eq!(auth.name, "Alex");
    }

    #[tokio::test]
    async fn accounts_file_has_no_secrets_and_is_atomic() {
        let dir = TempDir::new();
        let tokens = mem();
        let svc = service(&dir, tokens.clone());
        let a = svc.offline_add("Steve").await.unwrap();
        tokens
            .set(&token_key(a.id), &Secret::new("refresh-secret"))
            .await
            .unwrap();
        let _ = svc.launch_auth(None).await.unwrap();
        svc.offline_add("Alex").await.unwrap();

        let path = dir.path().join(ACCOUNTS_FILE);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("refresh-secret"));
        assert!(!raw.to_lowercase().contains("token"));
        assert!(!dir.path().join("accounts.json.tmp").exists());

        // A failed write leaves the previous file intact.
        std::fs::create_dir_all(dir.path().join("accounts.json.tmp")).unwrap();
        assert!(svc.offline_add("Third").await.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        assert_eq!(svc.list().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn corrupt_accounts_file_is_error() {
        let dir = TempDir::new();
        std::fs::write(dir.path().join(ACCOUNTS_FILE), b"{not json").unwrap();
        let svc = service(&dir, mem());
        assert_eq!(svc.list().await.unwrap_err().code, ErrorCode::Io);
    }

    #[tokio::test]
    async fn dangling_default_is_repaired_on_load() {
        let dir = TempDir::new();
        let svc = service(&dir, mem());
        svc.offline_add("Steve").await.unwrap();
        let path = dir.path().join(ACCOUNTS_FILE);
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["default"] = serde_json::Value::String(AccountId::new().to_string());
        std::fs::write(&path, v.to_string()).unwrap();
        let list = svc.list().await.unwrap();
        assert!(list[0].is_default);
    }

    #[tokio::test]
    async fn yggdrasil_is_stubbed() {
        let dir = TempDir::new();
        let svc = service(&dir, mem());
        let url = Url::parse("https://example.invalid/api/yggdrasil").unwrap();
        assert_eq!(
            svc.yggdrasil_login(&url, "u", "p").await.unwrap_err().code,
            ErrorCode::NotImplemented
        );
        assert!(svc.yggdrasil_servers().await.unwrap().is_empty());
    }
}
