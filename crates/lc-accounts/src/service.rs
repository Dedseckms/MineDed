//! `AccountsService`: account list in `accounts.json` + `AuthProvider` for launches.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use lc_http::{Downloader, Route};
use lc_runtime::{MemorySettingsStore, Paths, SettingsStore};
use lc_types::account::{Account, AccountKind, YggdrasilServer};
use lc_types::{AccountId, AppError, AppResult, ErrorCode};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use url::Url;

use crate::authlib::AuthlibManager;
use crate::contract::{
    AccountService, AuthProvider, AuthlibInjector, LaunchAuth, Secret, TokenStore, UserType,
};
use crate::fsutil::{read_optional, write_atomic};
use crate::offline::{offline_uuid, random_access_token, validate_offline_name};
use crate::yggdrasil::{self as ygg, Session};

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

/// Real `AccountService` + `AuthProvider`: offline and Yggdrasil
/// (authlib-injector) accounts. Yggdrasil tokens live in the `TokenStore`.
pub struct AccountsService {
    file: PathBuf,
    downloader: Arc<dyn Downloader>,
    tokens: Arc<dyn TokenStore>,
    settings: Arc<dyn SettingsStore>,
    authlib: AuthlibManager,
    /// Serializes read-modify-write cycles of `accounts.json`.
    lock: Mutex<()>,
}

/// A Yggdrasil account with fresh (validated or refreshed) tokens.
pub(crate) struct YggTarget {
    pub root: Url,
    pub session: Session,
    pub uuid: String,
    pub name: String,
}

impl std::fmt::Debug for AccountsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountsService")
            .field("file", &self.file)
            .finish_non_exhaustive()
    }
}

impl AccountsService {
    /// authlib-injector is cached in `Paths::cache()`. Without
    /// [`Self::with_settings`] the own server is [`ygg::DEFAULT_API_ROOT`].
    pub fn new(
        paths: &Paths,
        downloader: Arc<dyn Downloader>,
        token_store: Arc<dyn TokenStore>,
    ) -> Self {
        Self {
            file: paths.root().join(ACCOUNTS_FILE),
            downloader,
            tokens: token_store,
            settings: Arc::new(MemorySettingsStore::default()),
            authlib: AuthlibManager::new(paths.cache().join("authlib-injector")),
            lock: Mutex::new(()),
        }
    }

    /// Settings source for `skin_server` (own server API root, read on every call).
    pub fn with_settings(mut self, settings: Arc<dyn SettingsStore>) -> Self {
        self.settings = settings;
        self
    }

    /// Replaces the authlib-injector source (tests, self-hosted mirror).
    pub fn with_authlib(mut self, authlib: AuthlibManager) -> Self {
        self.authlib = authlib;
        self
    }

    pub(crate) fn http(&self) -> reqwest::Client {
        self.downloader.client(Route::Auto)
    }

    /// Own server API root: `Settings.skin_server` or [`ygg::DEFAULT_API_ROOT`].
    pub async fn own_api_root(&self) -> AppResult<Url> {
        let configured = self.settings.get().await?.skin_server;
        match configured.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => ygg::parse_server_input(s),
            _ => ygg::parse_server_input(ygg::DEFAULT_API_ROOT),
        }
    }

    async fn load_session(&self, id: AccountId) -> AppResult<Option<Session>> {
        let Some(secret) = self.tokens.get(&token_key(id)).await? else {
            return Ok(None);
        };
        Ok(serde_json::from_str(secret.expose()).ok())
    }

    async fn store_session(&self, id: AccountId, s: &Session) -> AppResult<()> {
        let json = serde_json::to_string(s)
            .map_err(|e| AppError::internal(format!("serialize session: {e}")))?;
        self.tokens.set(&token_key(id), &Secret::new(json)).await
    }

    /// Applies `f` to a stored account and saves (no-op if it vanished).
    async fn update_account(
        &self,
        id: AccountId,
        f: impl FnOnce(&mut StoredAccount),
    ) -> AppResult<()> {
        let _g = self.lock.lock().await;
        let mut data = self.load()?;
        if let Some(a) = data.accounts.iter_mut().find(|a| a.id == id) {
            f(a);
            self.save(&data)?;
        }
        Ok(())
    }

    pub(crate) async fn set_skin_url(&self, id: AccountId, url: Option<String>) -> AppResult<()> {
        self.update_account(id, move |a| a.skin_url = url).await
    }

    async fn find(&self, id: Option<AccountId>) -> AppResult<StoredAccount> {
        let data = {
            let _g = self.lock.lock().await;
            self.load()?
        };
        let id = match id.or(data.default) {
            Some(id) => id,
            None => return Err(AppError::new(ErrorCode::AuthRequired, "no account")),
        };
        data.accounts
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| Self::not_found(id))
    }

    async fn expired(&self, id: AccountId) -> AppError {
        if let Err(e) = self.update_account(id, |a| a.valid = false).await {
            tracing::warn!(account = %id, error = %e.message, "cannot mark account invalid");
        }
        AppError::new(ErrorCode::AuthExpired, "session expired, log in again")
            .detail("account", id.to_string())
    }

    /// Yggdrasil account with working tokens: `validate`, else `refresh`,
    /// else `AuthExpired` (and the account is marked invalid).
    pub(crate) async fn ygg_target(&self, id: Option<AccountId>) -> AppResult<YggTarget> {
        let acc = self.find(id).await?;
        if acc.kind != AccountKind::Yggdrasil {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "account has no skin server",
            ));
        }
        let root = acc
            .server
            .as_deref()
            .ok_or_else(|| AppError::internal("yggdrasil account without server"))
            .and_then(ygg::parse_server_input)?;
        let Some(mut session) = self.load_session(acc.id).await? else {
            return Err(self.expired(acc.id).await);
        };
        let client = self.http();
        let mut name = acc.name.clone();
        if !ygg::validate(&client, &root, &session).await? {
            match ygg::refresh(&client, &root, &session, None).await? {
                Some((s, profile)) => {
                    session = s;
                    self.store_session(acc.id, &session).await?;
                    if let Some(p) = profile {
                        name = p.name;
                    }
                }
                None => return Err(self.expired(acc.id).await),
            }
        }
        if !acc.valid || name != acc.name {
            let n = name.clone();
            self.update_account(acc.id, move |a| {
                a.valid = true;
                a.name = n;
            })
            .await?;
        }
        Ok(YggTarget {
            root,
            session,
            uuid: acc.uuid,
            name,
        })
    }

    /// Adds or updates (same server + profile) a Yggdrasil account.
    async fn upsert_yggdrasil(
        &self,
        root: &Url,
        session: &Session,
        profile: &ygg::GameProfile,
    ) -> AppResult<Account> {
        let uuid = profile.dashed_uuid()?;
        let server = root.as_str().to_owned();
        let _g = self.lock.lock().await;
        let mut data = self.load()?;
        let existing = data.accounts.iter().position(|a| {
            a.kind == AccountKind::Yggdrasil
                && a.server.as_deref() == Some(server.as_str())
                && a.uuid == uuid
        });
        let id = existing.map_or_else(AccountId::new, |i| data.accounts[i].id);
        // Token first: an account without a token would be unusable.
        self.store_session(id, session).await?;
        let account = StoredAccount {
            id,
            kind: AccountKind::Yggdrasil,
            name: profile.name.clone(),
            uuid,
            server: Some(server),
            skin_url: existing.and_then(|i| data.accounts[i].skin_url.clone()),
            valid: true,
        };
        match existing {
            Some(i) => data.accounts[i] = account.clone(),
            None => data.accounts.push(account.clone()),
        }
        data.fix_default();
        self.save(&data)?;
        Ok(data.to_dto(&account))
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

    /// `api_root` may be just the domain: the real root is found via ALI.
    async fn yggdrasil_login(
        &self,
        api_root: &Url,
        username: &str,
        password: &str,
    ) -> AppResult<Account> {
        let username = check_credentials(username, password)?;
        let client = self.http();
        let (root, _meta) = ygg::discover(&client, api_root).await?;
        let (session, profile) =
            ygg::authenticate(&client, &root, username, password, &random_access_token()).await?;
        self.upsert_yggdrasil(&root, &session, &profile).await
    }

    /// Drasl `POST /drasl/api/v3/users` with the invite, then a normal login.
    async fn yggdrasil_register(
        &self,
        api_root: &Url,
        invite: &str,
        username: &str,
        password: &str,
    ) -> AppResult<Account> {
        let username = check_credentials(username, password)?;
        let client = self.http();
        let (root, _meta) = ygg::discover(&client, api_root).await?;
        ygg::drasl_register(&client, &root, invite, username, password).await?;
        self.yggdrasil_login(&root, username, password).await
    }

    async fn yggdrasil_servers(&self) -> AppResult<Vec<YggdrasilServer>> {
        Ok(vec![
            YggdrasilServer {
                name: "MineDed".into(),
                api_root: self.own_api_root().await?.to_string(),
                official: true,
            },
            YggdrasilServer {
                name: "Ely.by".into(),
                api_root: ygg::ELY_BY_API_ROOT.into(),
                official: false,
            },
        ])
    }

    async fn remove(&self, id: AccountId) -> AppResult<()> {
        let logout = {
            let _g = self.lock.lock().await;
            let mut data = self.load()?;
            let Some(pos) = data.accounts.iter().position(|a| a.id == id) else {
                return Err(Self::not_found(id));
            };
            // Captured before the token is deleted; used after the lock is released.
            let mut logout = None;
            if data.accounts[pos].kind == AccountKind::Yggdrasil {
                let root = data.accounts[pos]
                    .server
                    .as_deref()
                    .and_then(|s| ygg::parse_server_input(s).ok());
                if let (Some(root), Ok(Some(s))) = (root, self.load_session(id).await) {
                    logout = Some((root, s));
                }
            }
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
            self.save(&data)?;
            logout
        };
        // Best-effort server logout outside the lock; never fails removal.
        if let Some((root, s)) = logout {
            ygg::invalidate(&self.http(), &root, &s).await;
        }
        Ok(())
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
    /// Yggdrasil: validate → refresh → `AuthExpired`; then authlib-injector
    /// jar + prefetched metadata (`-javaagent` is added by core).
    async fn launch_auth(&self, account: Option<AccountId>) -> AppResult<LaunchAuth> {
        let acc = self.find(account).await?;
        match acc.kind {
            AccountKind::Offline => Ok(LaunchAuth {
                name: acc.name.clone(),
                uuid: acc.uuid.clone(),
                access_token: Secret::new(random_access_token()),
                user_type: UserType::Legacy,
                authlib: None,
            }),
            AccountKind::Yggdrasil => {
                let t = self.ygg_target(Some(acc.id)).await?;
                let client = self.http();
                let jar = self.authlib.ensure(&client).await?;
                let meta = ygg::fetch_metadata(&client, &t.root).await?;
                Ok(LaunchAuth {
                    name: t.name,
                    uuid: t.uuid,
                    access_token: Secret::new(t.session.access_token),
                    user_type: UserType::Mojang,
                    authlib: Some(AuthlibInjector {
                        jar,
                        api_root: t.root,
                        prefetched: Some(base64::engine::general_purpose::STANDARD.encode(meta)),
                    }),
                })
            }
        }
    }
}

/// Trimmed non-empty username + non-empty password.
fn check_credentials<'a>(username: &'a str, password: &str) -> AppResult<&'a str> {
    let username = username.trim();
    if username.is_empty() || username.len() > 256 {
        return Err(
            AppError::new(ErrorCode::InvalidInput, "username is required")
                .detail("field", "username"),
        );
    }
    if password.is_empty() {
        return Err(
            AppError::new(ErrorCode::InvalidInput, "password is required")
                .detail("field", "password"),
        );
    }
    Ok(username)
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
    async fn yggdrasil_servers_use_settings() {
        let dir = TempDir::new();
        let settings = Arc::new(lc_runtime::MemorySettingsStore::default());
        let svc = AccountsService::new(&Paths::new(dir.path()), Arc::new(StubDownloader), mem())
            .with_settings(settings.clone());
        let list = svc.yggdrasil_servers().await.unwrap();
        assert_eq!(list[0].api_root, ygg::DEFAULT_API_ROOT);
        assert!(list[0].official && !list[1].official);

        let mut s = settings.get().await.unwrap();
        s.skin_server = Some("my.host".into());
        settings.set(s).await.unwrap();
        let list = svc.yggdrasil_servers().await.unwrap();
        assert_eq!(list[0].api_root, "https://my.host/");
    }

    #[tokio::test]
    async fn credentials_are_validated_before_network() {
        let dir = TempDir::new();
        let svc = service(&dir, mem());
        let url = Url::parse("https://example.invalid/").unwrap();
        for (u, p) in [("", "p"), ("  ", "p"), ("u", "")] {
            assert_eq!(
                svc.yggdrasil_login(&url, u, p).await.unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
    }
}
