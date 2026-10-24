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

/// Parses a settings file written by any older version: top-level fields it
/// lacks take their `Settings::default()` values instead of failing.
fn parse_with_defaults(bytes: &[u8]) -> serde_json::Result<Settings> {
    let mut merged = serde_json::to_value(Settings::default())?;
    let file: serde_json::Value = serde_json::from_slice(bytes)?;
    match (merged.as_object_mut(), file) {
        (Some(base), serde_json::Value::Object(fields)) => base.extend(fields),
        (_, other) => return serde_json::from_value(other),
    }
    serde_json::from_value(merged)
}

/// Settings persisted as JSON. Missing file = defaults; a corrupt file is
/// moved aside (`settings.json.corrupt`) and defaults are used.
#[derive(Debug)]
pub struct JsonSettingsStore {
    path: std::path::PathBuf,
    cache: RwLock<Option<Settings>>,
}

impl JsonSettingsStore {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            cache: RwLock::new(None),
        }
    }

    async fn load(&self) -> AppResult<Settings> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => match parse_with_defaults(&bytes) {
                Ok(s) => Ok(s),
                Err(_) => {
                    let mut aside = self.path.clone().into_os_string();
                    aside.push(".corrupt");
                    let _ = tokio::fs::rename(&self.path, aside).await;
                    Ok(Settings::default())
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl SettingsStore for JsonSettingsStore {
    async fn get(&self) -> AppResult<Settings> {
        if let Some(s) = self.cache.read().await.as_ref() {
            return Ok(s.clone());
        }
        let mut cache = self.cache.write().await;
        if cache.is_none() {
            *cache = Some(self.load().await?);
        }
        Ok(cache.clone().unwrap_or_default())
    }

    async fn set(&self, settings: Settings) -> AppResult<()> {
        let mut cache = self.cache.write().await;
        if let Some(dir) = self.path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let json = serde_json::to_vec_pretty(&settings)
            .map_err(|e| lc_types::AppError::internal(format!("settings: {e}")))?;
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        tokio::fs::write(&tmp, json).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        *cache = Some(settings);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_file_gets_new_fields_from_defaults() {
        let s = parse_with_defaults(br#"{"language":"en","lightMode":true}"#).unwrap();
        assert!(s.light_mode);
        assert_eq!(s.translucent, Settings::default().translucent);
        assert_eq!(s.hub_url, Settings::default().hub_url);
    }

    #[tokio::test]
    async fn json_store_roundtrip_and_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("mineup-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let store = JsonSettingsStore::new(&path);
        assert_eq!(store.get().await.unwrap(), Settings::default());
        let s = Settings {
            light_mode: true,
            ..Settings::default()
        };
        store.set(s.clone()).await.unwrap();
        assert_eq!(JsonSettingsStore::new(&path).get().await.unwrap(), s);

        tokio::fs::write(&path, b"{not json").await.unwrap();
        assert_eq!(
            JsonSettingsStore::new(&path).get().await.unwrap(),
            Settings::default()
        );
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn roundtrip() {
        let store = MemorySettingsStore::default();
        let mut s = store.get().await.unwrap();
        s.light_mode = true;
        store.set(s.clone()).await.unwrap();
        assert_eq!(store.get().await.unwrap(), s);
    }
}
