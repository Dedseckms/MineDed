//! `SkinsService`: local skin library (`<root>/skins/*.png` + `skins.json`)
//! and upload to the account's Yggdrasil server (authlib-injector texture API).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use lc_runtime::Paths;
use lc_types::skin::{Cape, RemoteSkin, Skin, SkinModel};
use lc_types::{AccountId, AppError, AppResult, ErrorCode, SkinId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::contract::SkinService;
use crate::fsutil::{io_err, read_optional, write_atomic};
use crate::remote::{self, RemoteEndpoints};
use crate::service::AccountsService;
use crate::yggdrasil::{self as ygg, TextureKind};

/// Library index inside `<root>/skins`.
pub const SKINS_INDEX: &str = "skins.json";
/// Upper bound for a skin file; real skins are a few KB.
const MAX_PNG: u64 = 1024 * 1024;
const MAX_NAME: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSkin {
    id: SkinId,
    name: String,
    model: SkinModel,
    /// SHA-256 hex of the PNG; the file is `<hash>.png`.
    hash: String,
    /// Texture URL on the server after a successful `apply`.
    #[serde(default)]
    remote_url: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SkinsFile {
    #[serde(default)]
    skins: Vec<StoredSkin>,
}

/// Checks the PNG signature and IHDR size: 64x64 or legacy 64x32.
pub fn validate_skin_png(data: &[u8]) -> AppResult<(u32, u32)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    let bad = |m: &str| AppError::new(ErrorCode::InvalidInput, m.to_owned()).detail("field", "png");
    if data.len() < 33 || &data[..8] != SIG || &data[12..16] != b"IHDR" {
        return Err(bad("not a PNG file"));
    }
    let be = |o: usize| u32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let (w, h) = (be(16), be(20));
    if (w, h) != (64, 64) && (w, h) != (64, 32) {
        return Err(bad("skin must be 64x64 or 64x32")
            .detail("width", w.to_string())
            .detail("height", h.to_string()));
    }
    Ok((w, h))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn hex_sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Real `SkinService`. Uploads go to the account's own Yggdrasil server.
pub struct SkinsService {
    dir: PathBuf,
    /// Downloaded previews of `fetch_remote` (`<cache>/skins-remote`).
    remote_cache: PathBuf,
    pub(crate) remote: RemoteEndpoints,
    accounts: Arc<AccountsService>,
    lock: Mutex<()>,
}

impl std::fmt::Debug for SkinsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkinsService")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl SkinsService {
    pub fn new(paths: &Paths, accounts: Arc<AccountsService>) -> Self {
        Self {
            dir: paths.root().join("skins"),
            remote_cache: paths.cache().join("skins-remote"),
            remote: RemoteEndpoints::default(),
            accounts,
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> AppResult<SkinsFile> {
        let path = self.dir.join(SKINS_INDEX);
        match read_optional(&path)? {
            None => Ok(SkinsFile::default()),
            Some(b) => {
                let mut data: SkinsFile = serde_json::from_slice(&b).map_err(|e| {
                    AppError::new(ErrorCode::Io, format!("skins.json is invalid: {e}"))
                        .detail("path", path.display().to_string())
                })?;
                // The hash becomes a file name: drop tampered entries (path traversal).
                data.skins.retain(|s| {
                    let ok = is_sha256_hex(&s.hash);
                    if !ok {
                        tracing::warn!(skin = %s.id, "skins.json entry with invalid hash ignored");
                    }
                    ok
                });
                Ok(data)
            }
        }
    }

    fn save(&self, data: &SkinsFile) -> AppResult<()> {
        let bytes = serde_json::to_vec_pretty(data)
            .map_err(|e| AppError::internal(format!("serialize skins: {e}")))?;
        write_atomic(&self.dir.join(SKINS_INDEX), &bytes, false)
    }

    fn png_path(&self, hash: &str) -> PathBuf {
        self.dir.join(format!("{hash}.png"))
    }

    fn to_dto(&self, s: &StoredSkin) -> Skin {
        Skin {
            id: s.id,
            name: s.name.clone(),
            model: s.model,
            hash: s.hash.clone(),
            url: s.remote_url.clone().unwrap_or_default(),
            local_path: Some(self.png_path(&s.hash).display().to_string()),
            local_only: s.remote_url.is_none(),
        }
    }

    fn not_found(id: SkinId) -> AppError {
        AppError::new(ErrorCode::NotFound, "skin not found").detail("id", id.to_string())
    }
}

#[async_trait]
impl SkinService for SkinsService {
    async fn library(&self) -> AppResult<Vec<Skin>> {
        let _g = self.lock.lock().await;
        Ok(self.load()?.skins.iter().map(|s| self.to_dto(s)).collect())
    }

    /// Identical PNG + model already in the library в†’ the existing entry.
    async fn add(&self, png_path: &str, name: &str, model: SkinModel) -> AppResult<Skin> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "skin name must be 1-64 characters",
            )
            .detail("field", "name"));
        }
        let src = PathBuf::from(png_path);
        let meta = std::fs::metadata(&src).map_err(|e| io_err("stat skin", &src, e))?;
        if meta.len() > MAX_PNG {
            return Err(
                AppError::new(ErrorCode::InvalidInput, "skin file is too large")
                    .detail("field", "png"),
            );
        }
        let data = std::fs::read(&src).map_err(|e| io_err("read skin", &src, e))?;
        validate_skin_png(&data)?;
        let hash = hex_sha256(&data);

        let _g = self.lock.lock().await;
        let mut index = self.load()?;
        if let Some(s) = index
            .skins
            .iter()
            .find(|s| s.hash == hash && s.model == model)
        {
            return Ok(self.to_dto(s));
        }
        let dest = self.png_path(&hash);
        if !dest.exists() {
            write_atomic(&dest, &data, false)?;
        }
        let entry = StoredSkin {
            id: SkinId::new(),
            name: name.to_owned(),
            model,
            hash,
            remote_url: None,
        };
        index.skins.push(entry.clone());
        self.save(&index)?;
        Ok(self.to_dto(&entry))
    }

    /// Network goes through the downloader's `Route::Auto` client; bodies are
    /// capped at 1 MB. The model comes from profile metadata when the source is
    /// a nickname, otherwise from the PNG's arm pixels.
    async fn fetch_remote(&self, source: &str) -> AppResult<RemoteSkin> {
        remote::fetch_remote(
            &self.accounts.http(),
            &self.remote,
            &self.remote_cache,
            source,
        )
        .await
    }

    async fn remove(&self, id: SkinId) -> AppResult<()> {
        let _g = self.lock.lock().await;
        let mut index = self.load()?;
        let Some(pos) = index.skins.iter().position(|s| s.id == id) else {
            return Err(Self::not_found(id));
        };
        let removed = index.skins.remove(pos);
        self.save(&index)?;
        if !index.skins.iter().any(|s| s.hash == removed.hash) {
            let _ = std::fs::remove_file(self.png_path(&removed.hash));
        }
        Ok(())
    }

    /// `PUT <api root>/api/user/profile/<uuid>/skin` with the account's access
    /// token (Drasl supports the authlib-injector texture upload API).
    async fn apply(&self, account: AccountId, skin: SkinId) -> AppResult<()> {
        let entry = {
            let _g = self.lock.lock().await;
            self.load()?
                .skins
                .into_iter()
                .find(|s| s.id == skin)
                .ok_or_else(|| Self::not_found(skin))?
        };
        let path = self.png_path(&entry.hash);
        let png = std::fs::read(&path).map_err(|e| io_err("read skin", &path, e))?;
        let t = self.accounts.ygg_target(Some(account)).await?;
        let client = self.accounts.http();
        ygg::upload_texture(
            &client,
            &t.root,
            &t.session,
            &t.uuid,
            TextureKind::Skin,
            png,
            entry.model,
        )
        .await?;
        // The texture URL is informational: a failed lookup does not fail apply.
        let url = match ygg::fetch_textures(&client, &t.root, &t.uuid).await {
            Ok(tex) => tex.skin_url,
            Err(e) => {
                tracing::warn!(error = %e.message, "cannot read profile textures after upload");
                None
            }
        };
        self.accounts.set_skin_url(account, url.clone()).await?;
        if let Some(url) = url {
            let _g = self.lock.lock().await;
            let mut index = self.load()?;
            if let Some(s) = index.skins.iter_mut().find(|s| s.id == skin) {
                s.remote_url = Some(url);
                self.save(&index)?;
            }
        }
        Ok(())
    }

    /// The Yggdrasil API has no cape catalog: only the active cape (if any)
    /// from the signed profile textures is listed.
    async fn capes(&self, account: AccountId) -> AppResult<Vec<Cape>> {
        let t = self.accounts.ygg_target(Some(account)).await?;
        let tex = ygg::fetch_textures(&self.accounts.http(), &t.root, &t.uuid).await?;
        Ok(tex
            .cape_url
            .map(|url| {
                let id = url.rsplit('/').next().unwrap_or_default().to_owned();
                vec![Cape {
                    id,
                    name: "Cape".into(),
                    url,
                    active: true,
                }]
            })
            .unwrap_or_default())
    }

    /// `None` removes the cape (`DELETE .../cape`); `Some(id)` must be the
    /// currently active cape (no catalog to choose from).
    async fn set_cape(&self, account: AccountId, cape_id: Option<String>) -> AppResult<()> {
        match cape_id {
            None => {
                let t = self.accounts.ygg_target(Some(account)).await?;
                ygg::delete_texture(
                    &self.accounts.http(),
                    &t.root,
                    &t.session,
                    &t.uuid,
                    TextureKind::Cape,
                )
                .await
            }
            Some(id) => {
                if self.capes(account).await?.iter().any(|c| c.id == id) {
                    Ok(())
                } else {
                    Err(AppError::new(ErrorCode::NotFound, "cape not available").detail("id", id))
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Minimal PNG header with the given IHDR size (enough for validation).
    pub(crate) fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        v
    }

    #[test]
    fn tampered_hash_entries_are_ignored() {
        let dir = crate::fsutil::testutil::TempDir::new();
        let paths = Paths::new(dir.path());
        let accounts = Arc::new(AccountsService::new(
            &paths,
            Arc::new(lc_http::StubDownloader),
            Arc::new(crate::tokens::MemoryTokenStore::new()),
        ));
        let svc = SkinsService::new(&paths, accounts);
        let good = "a".repeat(64);
        let index = serde_json::json!({"skins": [
            {"id": SkinId::new(), "name": "ok", "model": "classic", "hash": good},
            {"id": SkinId::new(), "name": "evil", "model": "classic", "hash": "../../x"},
            {"id": SkinId::new(), "name": "upper", "model": "classic", "hash": "A".repeat(64)},
        ]});
        std::fs::create_dir_all(dir.path().join("skins")).unwrap();
        std::fs::write(
            dir.path().join("skins").join(SKINS_INDEX),
            index.to_string(),
        )
        .unwrap();
        let loaded = svc.load().unwrap();
        assert_eq!(loaded.skins.len(), 1);
        assert_eq!(loaded.skins[0].name, "ok");
    }

    #[test]
    fn png_validation() {
        assert_eq!(validate_skin_png(&png(64, 64)).unwrap(), (64, 64));
        assert_eq!(validate_skin_png(&png(64, 32)).unwrap(), (64, 32));
        for (w, h) in [(32, 32), (128, 128), (64, 16)] {
            assert_eq!(
                validate_skin_png(&png(w, h)).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
        assert!(validate_skin_png(b"GIF89a....................................").is_err());
        assert!(validate_skin_png(&png(64, 64)[..20]).is_err());
    }
}
