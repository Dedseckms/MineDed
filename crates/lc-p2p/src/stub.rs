use async_trait::async_trait;
use lc_types::room::{RoomAuthMode, RoomState};
use lc_types::voice::ChatMessage;
use lc_types::{AppError, AppResult, InstanceId};

use crate::contract::{DatagramTransport, RoomService};

/// Placeholder: no room, nothing to send.
#[derive(Debug, Default)]
pub struct StubRooms;

#[async_trait]
impl RoomService for StubRooms {
    async fn create(
        &self,
        _instance: InstanceId,
        _auth_mode: RoomAuthMode,
        _password: Option<String>,
    ) -> AppResult<RoomState> {
        Err(AppError::not_implemented("RoomService::create"))
    }

    async fn join(
        &self,
        _code: &str,
        _instance: InstanceId,
        _password: Option<String>,
    ) -> AppResult<RoomState> {
        Err(AppError::not_implemented("RoomService::join"))
    }

    async fn leave(&self) -> AppResult<()> {
        Ok(())
    }

    async fn state(&self) -> AppResult<Option<RoomState>> {
        Ok(None)
    }

    async fn kick(&self, _peer_id: &str) -> AppResult<()> {
        Err(AppError::not_implemented("RoomService::kick"))
    }

    async fn send_chat(&self, _text: &str) -> AppResult<ChatMessage> {
        Err(AppError::not_implemented("RoomService::send_chat"))
    }
}

#[async_trait]
impl DatagramTransport for StubRooms {
    async fn send(&self, _to: Option<&str>, _payload: &[u8]) -> AppResult<()> {
        Err(AppError::not_implemented("DatagramTransport::send"))
    }

    async fn recv(&self) -> Option<(String, Vec<u8>)> {
        None
    }

    fn max_datagram_size(&self) -> usize {
        1200
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_has_no_room() {
        assert!(StubRooms.state().await.unwrap().is_none());
        assert!(StubRooms.recv().await.is_none());
    }
}
