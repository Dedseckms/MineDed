//! Per-instance record of where content files came from
//! (`instances/<id>/content-index.json`, outside the game dir).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lc_types::content::{ContentKind, ContentSourceKind};
use lc_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};

use crate::util::{ensure_no_links, io_err};

pub(crate) const INDEX_FILE: &str = "content-index.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexEntry {
    pub kind: ContentKind,
    pub source: ContentSourceKind,
    pub project_id: String,
    pub version_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub version_number: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub sha512: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// Download URL (needed for `.mrpack` export).
    #[serde(default)]
    pub url: Option<String>,
}

/// Keyed by `<dir>/<file name without .disabled>`, e.g. `mods/sodium.jar`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ContentIndex {
    #[serde(default)]
    pub files: BTreeMap<String, IndexEntry>,
}

pub(crate) fn key(dir: &str, file_name: &str) -> String {
    format!("{dir}/{}", base_name(file_name))
}

pub(crate) fn base_name(file_name: &str) -> &str {
    file_name.strip_suffix(".disabled").unwrap_or(file_name)
}

impl ContentIndex {
    pub fn path(instance_dir: &Path) -> PathBuf {
        instance_dir.join(INDEX_FILE)
    }

    pub async fn load(instance_dir: &Path) -> AppResult<Self> {
        let p = Self::path(instance_dir);
        ensure_no_links(&p)?;
        match tokio::fs::read(&p).await {
            Ok(b) => Ok(serde_json::from_slice(&b).unwrap_or_default()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(io_err(&p, e)),
        }
    }

    pub async fn save(&self, instance_dir: &Path) -> AppResult<()> {
        let p = Self::path(instance_dir);
        ensure_no_links(&p)?;
        tokio::fs::create_dir_all(instance_dir)
            .await
            .map_err(|e| io_err(instance_dir, e))?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| AppError::internal(format!("json encode: {e}")))?;
        let tmp = p.with_extension("json.tmp");
        ensure_no_links(&tmp)?;
        tokio::fs::write(&tmp, bytes)
            .await
            .map_err(|e| io_err(&tmp, e))?;
        tokio::fs::rename(&tmp, &p).await.map_err(|e| io_err(&p, e))
    }

    /// Key of the file currently holding `project_id`, if any.
    pub fn find_project(&self, source: ContentSourceKind, project_id: &str) -> Option<String> {
        self.files
            .iter()
            .find(|(_, e)| e.source == source && e.project_id == project_id)
            .map(|(k, _)| k.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut idx = ContentIndex::default();
        idx.files.insert(
            key("mods", "a.jar.disabled"),
            IndexEntry {
                kind: ContentKind::Mod,
                source: ContentSourceKind::Modrinth,
                project_id: "p".into(),
                version_id: "v".into(),
                title: None,
                version_number: None,
                sha1: None,
                sha512: None,
                size: None,
                url: None,
            },
        );
        idx.save(dir.path()).await.unwrap();
        let back = ContentIndex::load(dir.path()).await.unwrap();
        assert_eq!(back, idx);
        assert_eq!(
            back.find_project(ContentSourceKind::Modrinth, "p")
                .as_deref(),
            Some("mods/a.jar")
        );
    }
}
