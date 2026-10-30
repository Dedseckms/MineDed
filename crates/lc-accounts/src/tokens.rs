//! `TokenStore` implementations: OS keyring, encrypted-file fallback, in-memory (tests).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use chacha20poly1305::aead::{Aead, Generate, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use lc_runtime::Paths;
use lc_types::{AppError, AppResult, ErrorCode};

use crate::contract::{Secret, TokenStore};
use crate::fsutil::{read_optional, write_atomic};

/// Keyring service name for every launcher secret.
pub const KEYRING_SERVICE: &str = "ru.dedseckms.mineup";

/// Fallback file names in `Paths::root()`.
pub const FALLBACK_FILE: &str = "tokens.enc";
pub const FALLBACK_KEY_FILE: &str = "tokens.key";

const MAGIC: &[u8; 4] = b"LCT1";
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const PROBE_USER: &str = "__mineup_probe__";

fn internal(msg: &str) -> AppError {
    AppError::new(ErrorCode::Internal, msg)
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

/// In-memory store for tests and ephemeral sessions.
#[derive(Default)]
pub struct MemoryTokenStore {
    map: Mutex<HashMap<String, Secret>>,
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, HashMap<String, Secret>>> {
        self.map
            .lock()
            .map_err(|_| internal("memory token store poisoned"))
    }
}

impl std::fmt::Debug for MemoryTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.map.lock().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("MemoryTokenStore")
            .field("entries", &n)
            .finish()
    }
}

#[async_trait]
impl TokenStore for MemoryTokenStore {
    async fn get(&self, key: &str) -> AppResult<Option<Secret>> {
        Ok(self.lock()?.get(key).cloned())
    }

    async fn set(&self, key: &str, value: &Secret) -> AppResult<()> {
        self.lock()?.insert(key.to_owned(), value.clone());
        Ok(())
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.lock()?.remove(key);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Encrypted file
// ---------------------------------------------------------------------------

/// Fallback store: all secrets in one XChaCha20-Poly1305 encrypted file.
///
/// Layout of `tokens.enc`: `b"LCT1" | nonce (24) | ciphertext(JSON map)`, the magic is
/// authenticated as AAD. The 32-byte key lives in `tokens.key` (owner-only on Unix).
/// This only protects against casual reading of the data dir, not against malware
/// running as the same user.
pub struct EncryptedFileTokenStore {
    file: PathBuf,
    key_file: PathBuf,
    lock: Mutex<()>,
}

impl std::fmt::Debug for EncryptedFileTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedFileTokenStore")
            .field("file", &self.file)
            .finish_non_exhaustive()
    }
}

impl EncryptedFileTokenStore {
    /// Files are `<dir>/tokens.enc` and `<dir>/tokens.key`; nothing is created until `set`.
    pub fn new(dir: &Path) -> Self {
        Self {
            file: dir.join(FALLBACK_FILE),
            key_file: dir.join(FALLBACK_KEY_FILE),
            lock: Mutex::new(()),
        }
    }

    fn load_key(&self, create: bool) -> AppResult<Option<XChaCha20Poly1305>> {
        match read_optional(&self.key_file)? {
            Some(bytes) if bytes.len() == KEY_LEN => {
                let key = Key::try_from(bytes.as_slice())
                    .map_err(|_| internal("token store key file is corrupted"))?;
                Ok(Some(XChaCha20Poly1305::new(&key)))
            }
            Some(_) => Err(internal("token store key file is corrupted")
                .detail("path", self.key_file.display().to_string())),
            None if create => {
                let key = Key::try_generate()
                    .map_err(|_| internal("token store key generation failed"))?;
                write_atomic(&self.key_file, key.as_slice(), true)?;
                Ok(Some(XChaCha20Poly1305::new(&key)))
            }
            None => Ok(None),
        }
    }

    fn load_map(
        &self,
        create_key: bool,
    ) -> AppResult<(XChaCha20Poly1305Opt, BTreeMap<String, String>)> {
        let data = read_optional(&self.file)?;
        let cipher = self.load_key(create_key || data.is_none())?;
        let Some(data) = data else {
            return Ok((cipher, BTreeMap::new()));
        };
        let corrupted = || {
            internal("token store file is corrupted")
                .detail("path", self.file.display().to_string())
        };
        let Some(cipher) = cipher else {
            return Err(internal("token store key file is missing")
                .detail("path", self.key_file.display().to_string()));
        };
        if data.len() < MAGIC.len() + NONCE_LEN || &data[..MAGIC.len()] != MAGIC {
            return Err(corrupted());
        }
        let (nonce, ct) = data[MAGIC.len()..].split_at(NONCE_LEN);
        let nonce = XNonce::try_from(nonce).map_err(|_| corrupted())?;
        let plain = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ct,
                    aad: MAGIC,
                },
            )
            .map_err(|_| corrupted())?;
        let map = serde_json::from_slice(&plain).map_err(|_| corrupted())?;
        Ok((Some(cipher), map))
    }

    fn save_map(
        &self,
        cipher: &XChaCha20Poly1305,
        map: &BTreeMap<String, String>,
    ) -> AppResult<()> {
        let plain =
            serde_json::to_vec(map).map_err(|_| internal("token store serialization failed"))?;
        let nonce =
            XNonce::try_generate().map_err(|_| internal("token store nonce generation failed"))?;
        let ct = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: &plain,
                    aad: MAGIC,
                },
            )
            .map_err(|_| internal("token store encryption failed"))?;
        let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ct.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        write_atomic(&self.file, &out, true)
    }

    fn guard(&self) -> AppResult<std::sync::MutexGuard<'_, ()>> {
        self.lock
            .lock()
            .map_err(|_| internal("file token store poisoned"))
    }

    fn get_sync(&self, key: &str) -> AppResult<Option<Secret>> {
        let _g = self.guard()?;
        let (_, map) = self.load_map(false)?;
        Ok(map.get(key).map(|v| Secret::new(v.clone())))
    }

    fn set_sync(&self, key: &str, value: &Secret) -> AppResult<()> {
        let _g = self.guard()?;
        let (cipher, mut map) = self.load_map(true)?;
        let cipher = cipher.ok_or_else(|| internal("token store key unavailable"))?;
        map.insert(key.to_owned(), value.expose().to_owned());
        self.save_map(&cipher, &map)
    }

    fn delete_sync(&self, key: &str) -> AppResult<()> {
        let _g = self.guard()?;
        let (cipher, mut map) = self.load_map(false)?;
        if map.remove(key).is_some() {
            let cipher = cipher.ok_or_else(|| internal("token store key unavailable"))?;
            self.save_map(&cipher, &map)?;
        }
        Ok(())
    }
}

type XChaCha20Poly1305Opt = Option<XChaCha20Poly1305>;

#[async_trait]
impl TokenStore for EncryptedFileTokenStore {
    async fn get(&self, key: &str) -> AppResult<Option<Secret>> {
        self.get_sync(key)
    }

    async fn set(&self, key: &str, value: &Secret) -> AppResult<()> {
        self.set_sync(key, value)
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.delete_sync(key)
    }
}

// ---------------------------------------------------------------------------
// Keyring
// ---------------------------------------------------------------------------

/// Describes a keyring error without echoing any payload (e.g. `BadEncoding` holds secret bytes).
fn keyring_err(op: &str, e: &keyring::Error) -> AppError {
    let what = match e {
        keyring::Error::PlatformFailure(_) => "platform failure",
        keyring::Error::NoStorageAccess(_) => "storage not accessible",
        keyring::Error::NoEntry => "no entry",
        keyring::Error::BadEncoding(_) => "stored value is not valid UTF-8",
        keyring::Error::TooLong(_, _) => "value or attribute too long",
        keyring::Error::Invalid(_, _) => "invalid attribute",
        keyring::Error::Ambiguous(_) => "ambiguous entry",
        _ => "unknown error",
    };
    internal(&format!("keyring {op}: {what}"))
}

fn entry(key: &str) -> AppResult<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, key).map_err(|e| keyring_err("open", &e))
}

fn keyring_get(key: &str) -> AppResult<Option<Secret>> {
    match entry(key)?.get_password() {
        Ok(v) => Ok(Some(Secret::new(v))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(keyring_err("get", &e)),
    }
}

fn keyring_set(key: &str, value: &Secret) -> AppResult<()> {
    entry(key)?
        .set_password(value.expose())
        .map_err(|e| keyring_err("set", &e))
}

fn keyring_delete(key: &str) -> AppResult<()> {
    match entry(key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(keyring_err("delete", &e)),
    }
}

/// Returns `Ok` when the OS keyring answers (an absent probe entry counts as success).
fn probe_keyring() -> Result<(), &'static str> {
    let entry =
        keyring::Entry::new(KEYRING_SERVICE, PROBE_USER).map_err(|_| "cannot open entry")?;
    match entry.get_password() {
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(keyring::Error::NoStorageAccess(_)) => Err("storage not accessible"),
        Err(keyring::Error::PlatformFailure(_)) => Err("platform failure"),
        Err(_) => Err("unexpected error"),
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| internal("keyring task failed"))?
}

enum Backend {
    Keyring,
    File(EncryptedFileTokenStore),
}

/// Production `TokenStore`: OS keyring (service `ru.dedseckms.mineup`, user = key),
/// or the encrypted file in `Paths::root()` when the keyring is unavailable
/// (e.g. Linux without Secret Service). The choice is made once in [`Self::new`].
pub struct KeyringTokenStore {
    backend: Backend,
}

impl std::fmt::Debug for KeyringTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyringTokenStore")
            .field("fallback", &self.is_fallback())
            .finish()
    }
}

impl KeyringTokenStore {
    /// Probes the OS keyring (read-only, blocking call) and falls back to the
    /// encrypted file with a warning in the log if it does not respond.
    pub fn new(paths: &Paths) -> Self {
        match probe_keyring() {
            Ok(()) => Self {
                backend: Backend::Keyring,
            },
            Err(reason) => {
                tracing::warn!(
                    reason,
                    file = %paths.root().join(FALLBACK_FILE).display(),
                    "OS keyring unavailable, storing tokens in an encrypted file"
                );
                Self::file_only(paths)
            }
        }
    }

    /// Always uses the encrypted file (no keyring probe).
    pub fn file_only(paths: &Paths) -> Self {
        Self {
            backend: Backend::File(EncryptedFileTokenStore::new(paths.root())),
        }
    }

    /// `true` when the encrypted-file fallback is in use.
    pub fn is_fallback(&self) -> bool {
        matches!(self.backend, Backend::File(_))
    }
}

#[async_trait]
impl TokenStore for KeyringTokenStore {
    async fn get(&self, key: &str) -> AppResult<Option<Secret>> {
        match &self.backend {
            Backend::Keyring => {
                let key = key.to_owned();
                blocking(move || keyring_get(&key)).await
            }
            Backend::File(f) => f.get(key).await,
        }
    }

    async fn set(&self, key: &str, value: &Secret) -> AppResult<()> {
        match &self.backend {
            Backend::Keyring => {
                let key = key.to_owned();
                let value = value.clone();
                blocking(move || keyring_set(&key, &value)).await
            }
            Backend::File(f) => f.set(key, value).await,
        }
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        match &self.backend {
            Backend::Keyring => {
                let key = key.to_owned();
                blocking(move || keyring_delete(&key)).await
            }
            Backend::File(f) => f.delete(key).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil::testutil::TempDir;

    async fn roundtrip(store: &dyn TokenStore) {
        assert_eq!(store.get("a").await.unwrap(), None);
        store.set("a", &Secret::new("one")).await.unwrap();
        store.set("b", &Secret::new("two")).await.unwrap();
        assert_eq!(store.get("a").await.unwrap(), Some(Secret::new("one")));
        store.set("a", &Secret::new("uno")).await.unwrap();
        assert_eq!(store.get("a").await.unwrap(), Some(Secret::new("uno")));
        store.delete("a").await.unwrap();
        store.delete("a").await.unwrap();
        store.delete("missing").await.unwrap();
        assert_eq!(store.get("a").await.unwrap(), None);
        assert_eq!(store.get("b").await.unwrap(), Some(Secret::new("two")));
    }

    #[tokio::test]
    async fn memory_store_roundtrip() {
        roundtrip(&MemoryTokenStore::new()).await;
    }

    #[tokio::test]
    async fn file_store_roundtrip_and_persistence() {
        let dir = TempDir::new();
        roundtrip(&EncryptedFileTokenStore::new(dir.path())).await;
        // A new instance reads the same data.
        let again = EncryptedFileTokenStore::new(dir.path());
        assert_eq!(again.get("b").await.unwrap(), Some(Secret::new("two")));
        // Also through the facade.
        let paths = Paths::new(dir.path());
        let facade = KeyringTokenStore::file_only(&paths);
        assert!(facade.is_fallback());
        assert_eq!(facade.get("b").await.unwrap(), Some(Secret::new("two")));
    }

    #[tokio::test]
    async fn file_store_reads_legacy_0_10_ciphertext_and_preserves_it_on_update() {
        let dir = TempDir::new();
        // Generated with chacha20poly1305 0.10.1, key [0x42; 32], nonce [0x24; 24],
        // AAD b"LCT1", plaintext {"legacy-account":"dummy-refresh-token"}.
        let ciphertext = [
            222, 124, 187, 0, 9, 69, 47, 223, 143, 78, 92, 89, 97, 253, 99, 190, 16, 203, 172, 222,
            9, 114, 25, 228, 12, 19, 131, 43, 25, 117, 168, 165, 57, 229, 231, 179, 81, 245, 121,
            225, 52, 51, 176, 242, 18, 227, 19, 14, 45, 90, 42, 243, 234, 92, 213, 44,
        ];
        let mut legacy_file = b"LCT1".to_vec();
        legacy_file.extend_from_slice(&[0x24; 24]);
        legacy_file.extend_from_slice(&ciphertext);
        std::fs::write(dir.path().join(FALLBACK_KEY_FILE), [0x42; 32]).unwrap();
        std::fs::write(dir.path().join(FALLBACK_FILE), &legacy_file).unwrap();

        let store = EncryptedFileTokenStore::new(dir.path());
        assert_eq!(
            store.get("legacy-account").await.unwrap(),
            Some(Secret::new("dummy-refresh-token"))
        );
        store
            .set("new-account", &Secret::new("dummy-new-token"))
            .await
            .unwrap();
        let reopened = EncryptedFileTokenStore::new(dir.path());
        assert_eq!(
            reopened.get("legacy-account").await.unwrap(),
            Some(Secret::new("dummy-refresh-token"))
        );
        assert_eq!(
            reopened.get("new-account").await.unwrap(),
            Some(Secret::new("dummy-new-token"))
        );
        assert_eq!(
            std::fs::read(dir.path().join(FALLBACK_KEY_FILE)).unwrap(),
            [0x42; 32]
        );
        assert!(std::fs::read(dir.path().join(FALLBACK_FILE))
            .unwrap()
            .starts_with(b"LCT1"));
    }

    #[tokio::test]
    async fn file_store_does_not_contain_plaintext() {
        let dir = TempDir::new();
        let store = EncryptedFileTokenStore::new(dir.path());
        store
            .set("account:x", &Secret::new("super-secret-token"))
            .await
            .unwrap();
        let raw = std::fs::read(dir.path().join(FALLBACK_FILE)).unwrap();
        let hay = String::from_utf8_lossy(&raw);
        assert!(!hay.contains("super-secret-token"));
        assert!(!hay.contains("account:x"));
        assert_eq!(
            std::fs::read(dir.path().join(FALLBACK_KEY_FILE))
                .unwrap()
                .len(),
            KEY_LEN
        );
    }

    #[tokio::test]
    async fn file_store_corruption_is_error_not_panic() {
        let dir = TempDir::new();
        let store = EncryptedFileTokenStore::new(dir.path());
        store.set("k", &Secret::new("v")).await.unwrap();
        let file = dir.path().join(FALLBACK_FILE);

        // Flip one ciphertext byte.
        let mut raw = std::fs::read(&file).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        std::fs::write(&file, &raw).unwrap();
        assert_eq!(store.get("k").await.unwrap_err().code, ErrorCode::Internal);
        assert!(store.set("k", &Secret::new("v2")).await.is_err());

        // Truncated / garbage file.
        std::fs::write(&file, b"LC").unwrap();
        assert_eq!(store.get("k").await.unwrap_err().code, ErrorCode::Internal);
        std::fs::write(&file, b"not encrypted at all, just junk bytes here....").unwrap();
        assert_eq!(store.get("k").await.unwrap_err().code, ErrorCode::Internal);

        // Broken key file.
        store_with_fresh_file(&dir).await;
        std::fs::write(dir.path().join(FALLBACK_KEY_FILE), b"short").unwrap();
        assert_eq!(store.get("k").await.unwrap_err().code, ErrorCode::Internal);

        // Missing key file with existing data.
        std::fs::remove_file(dir.path().join(FALLBACK_KEY_FILE)).unwrap();
        assert_eq!(store.get("k").await.unwrap_err().code, ErrorCode::Internal);
    }

    async fn store_with_fresh_file(dir: &TempDir) {
        let _ = std::fs::remove_file(dir.path().join(FALLBACK_FILE));
        let _ = std::fs::remove_file(dir.path().join(FALLBACK_KEY_FILE));
        EncryptedFileTokenStore::new(dir.path())
            .set("k", &Secret::new("v"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn errors_and_debug_do_not_leak_secrets() {
        let mem = MemoryTokenStore::new();
        mem.set("k", &Secret::new("leaky-token")).await.unwrap();
        assert!(!format!("{mem:?}").contains("leaky-token"));

        let dir = TempDir::new();
        let file = EncryptedFileTokenStore::new(dir.path());
        file.set("k", &Secret::new("leaky-token")).await.unwrap();
        assert!(!format!("{file:?}").contains("leaky-token"));
        let facade = KeyringTokenStore::file_only(&Paths::new(dir.path()));
        assert!(!format!("{facade:?}").contains("leaky-token"));

        let e = keyring_err("get", &keyring::Error::BadEncoding(b"leaky-token".to_vec()));
        assert!(!format!("{e:?} {e}").contains("leaky-token"));
    }

    /// Touches the real OS keyring: run manually with `cargo test -p lc-accounts -- --ignored`.
    #[tokio::test]
    #[ignore = "uses the real OS keyring"]
    async fn real_keyring_roundtrip() {
        let dir = TempDir::new();
        let store = KeyringTokenStore::new(&Paths::new(dir.path()));
        let key = format!("lc-accounts-test-{}", uuid::Uuid::new_v4());
        store.set(&key, &Secret::new("value")).await.unwrap();
        assert_eq!(store.get(&key).await.unwrap(), Some(Secret::new("value")));
        store.delete(&key).await.unwrap();
        assert_eq!(store.get(&key).await.unwrap(), None);
    }
}
