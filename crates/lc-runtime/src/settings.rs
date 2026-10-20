use async_trait::async_trait;
use lc_types::settings::Settings;
use lc_types::AppResult;
use tokio::sync::RwLock;

/// Persistent global settings. The real implementation (JSON file in the
/// config dir) is wired in `src-tauri`.
#[async_trait]
pub trait SettingsStore: Send + Sync {
    async fn get(&self) -> AppResult<Settings>;
    async fn set(&self, settings: Settings) -> AppResult<()>;
}

/// In-memory store for tests and stubs.
#[derive(Debug, Default)]
pub struct MemorySettingsStore {
    inner: RwLock<Settings>,
}

#[async_trait]
impl SettingsStore for MemorySettingsStore {
    async fn get(&self) -> AppResult<Settings> {
        Ok(self.inner.read().await.clone())
    }

    async fn set(&self, settings: Settings) -> AppResult<()> {
        *self.inner.write().await = settings;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip() {
        let store = MemorySettingsStore::default();
        let mut s = store.get().await.unwrap();
        s.light_mode = true;
        store.set(s.clone()).await.unwrap();
        assert_eq!(store.get().await.unwrap(), s);
    }
}
