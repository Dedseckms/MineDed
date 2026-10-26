use async_trait::async_trait;
use lc_types::account::{Account, YggdrasilServer};
use lc_types::skin::{Cape, RemoteSkin, Skin, SkinModel};
use lc_types::{AccountId, AppError, AppResult, SkinId};
use url::Url;

use crate::contract::{AccountService, AuthProvider, LaunchAuth, SkinService};

/// Placeholder until accounts implements the real services.
#[derive(Debug, Default)]
pub struct StubAccounts;

#[async_trait]
impl AuthProvider for StubAccounts {
    async fn launch_auth(&self, _account: Option<AccountId>) -> AppResult<LaunchAuth> {
        Err(AppError::not_implemented("AuthProvider::launch_auth"))
    }
}

#[async_trait]
impl AccountService for StubAccounts {
    async fn list(&self) -> AppResult<Vec<Account>> {
        Ok(Vec::new())
    }

    async fn offline_add(&self, _name: &str) -> AppResult<Account> {
        Err(AppError::not_implemented("AccountService::offline_add"))
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

    async fn remove(&self, _id: AccountId) -> AppResult<()> {
        Err(AppError::not_implemented("AccountService::remove"))
    }

    async fn set_default(&self, _id: AccountId) -> AppResult<()> {
        Err(AppError::not_implemented("AccountService::set_default"))
    }
}

#[derive(Debug, Default)]
pub struct StubSkins;

#[async_trait]
impl SkinService for StubSkins {
    async fn library(&self) -> AppResult<Vec<Skin>> {
        Ok(Vec::new())
    }

    async fn add(&self, _png_path: &str, _name: &str, _model: SkinModel) -> AppResult<Skin> {
        Err(AppError::not_implemented("SkinService::add"))
    }

    async fn fetch_remote(&self, _source: &str) -> AppResult<RemoteSkin> {
        Err(AppError::not_implemented("SkinService::fetch_remote"))
    }

    async fn remove(&self, _id: SkinId) -> AppResult<()> {
        Err(AppError::not_implemented("SkinService::remove"))
    }

    async fn apply(&self, _account: AccountId, _skin: SkinId) -> AppResult<()> {
        Err(AppError::not_implemented("SkinService::apply"))
    }

    async fn capes(&self, _account: AccountId) -> AppResult<Vec<Cape>> {
        Ok(Vec::new())
    }

    async fn set_cape(&self, _account: AccountId, _cape_id: Option<String>) -> AppResult<()> {
        Err(AppError::not_implemented("SkinService::set_cape"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Secret;
    use lc_types::ErrorCode;

    #[test]
    fn secret_is_redacted() {
        assert_eq!(format!("{:?}", Secret::new("token")), "Secret(***)");
    }

    #[tokio::test]
    async fn stub_launch_auth_not_implemented() {
        let err = StubAccounts.launch_auth(None).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
    }
}
