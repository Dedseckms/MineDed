use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::social::{
    AdminServerState, ModAction, SocialChannel, SocialChannelId, SocialContent, SocialFriend,
    SocialHistory, SocialMessage, SocialPrivacy, SocialReport, SocialRole, SocialUser,
    StorageTarget,
};
use lc_types::{AccountId, AppError, AppResult};

use crate::contract::ChatService;

/// Placeholder until net implements the hub client: chat is unavailable.
#[derive(Debug, Default)]
pub struct StubChat;

fn ni<T>(what: &str) -> AppResult<T> {
    Err(AppError::not_implemented(what))
}

#[async_trait]
impl ChatService for StubChat {
    async fn connect(&self, _account: AccountId) -> AppResult<SocialUser> {
        ni("ChatService::connect")
    }
    async fn disconnect(&self) -> AppResult<()> {
        Ok(())
    }
    async fn channels(&self) -> AppResult<Vec<SocialChannel>> {
        Ok(Vec::new())
    }
    async fn history(
        &self,
        _channel: &SocialChannelId,
        _before: Option<String>,
    ) -> AppResult<SocialHistory> {
        ni("ChatService::history")
    }
    async fn mark_read(&self, _channel: &SocialChannelId, _message_id: &str) -> AppResult<()> {
        ni("ChatService::mark_read")
    }
    async fn send(
        &self,
        _channel: &SocialChannelId,
        _content: SocialContent,
    ) -> AppResult<SocialMessage> {
        ni("ChatService::send")
    }
    async fn send_file(
        &self,
        _channel: &SocialChannelId,
        _path: &str,
        _caption: Option<String>,
        _sink: &dyn ProgressSink,
    ) -> AppResult<SocialMessage> {
        ni("ChatService::send_file")
    }
    async fn delete_message(&self, _message_id: &str) -> AppResult<()> {
        ni("ChatService::delete_message")
    }
    async fn save_attachment(
        &self,
        _attachment_id: &str,
        _dest: &str,
        _sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        ni("ChatService::save_attachment")
    }
    async fn open_direct(&self, _uuid: &str) -> AppResult<SocialChannel> {
        ni("ChatService::open_direct")
    }
    async fn search_users(&self, _query: &str) -> AppResult<Vec<SocialUser>> {
        Ok(Vec::new())
    }
    async fn profile(&self, _uuid: &str) -> AppResult<SocialUser> {
        ni("ChatService::profile")
    }
    async fn friends(&self) -> AppResult<Vec<SocialFriend>> {
        Ok(Vec::new())
    }
    async fn add_friend(&self, _uuid: &str) -> AppResult<SocialFriend> {
        ni("ChatService::add_friend")
    }
    async fn remove_friend(&self, _uuid: &str) -> AppResult<()> {
        ni("ChatService::remove_friend")
    }
    async fn set_blocked(&self, _uuid: &str, _blocked: bool) -> AppResult<()> {
        ni("ChatService::set_blocked")
    }
    async fn privacy(&self) -> AppResult<SocialPrivacy> {
        Ok(crate::contract::default_privacy())
    }
    async fn set_privacy(&self, _privacy: SocialPrivacy) -> AppResult<SocialPrivacy> {
        ni("ChatService::set_privacy")
    }
    async fn report(&self, _message_id: &str, _reason: Option<String>) -> AppResult<()> {
        ni("ChatService::report")
    }
    async fn reports(&self) -> AppResult<Vec<SocialReport>> {
        ni("ChatService::reports")
    }
    async fn resolve_report(&self, _report_id: &str) -> AppResult<()> {
        ni("ChatService::resolve_report")
    }
    async fn moderate_delete(&self, _message_id: &str) -> AppResult<()> {
        ni("ChatService::moderate_delete")
    }
    async fn moderate_user(
        &self,
        _uuid: &str,
        _action: ModAction,
        _seconds: Option<u64>,
    ) -> AppResult<()> {
        ni("ChatService::moderate_user")
    }
    async fn set_role(&self, _uuid: &str, _role: SocialRole) -> AppResult<()> {
        ni("ChatService::set_role")
    }
    async fn server_state(&self) -> AppResult<AdminServerState> {
        ni("ChatService::server_state")
    }
    async fn migrate_storage(&self, _target: StorageTarget) -> AppResult<()> {
        ni("ChatService::migrate_storage")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_is_empty() {
        assert!(StubChat.channels().await.unwrap().is_empty());
        assert!(StubChat.connect(AccountId::new()).await.is_err());
    }
}
