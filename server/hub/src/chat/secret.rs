//! Sealing of the password a guest asked for (`POST /chat/admin/create-account`).
//!
//! The hub, not the admin, creates the Drasl account, so the password has to stay
//! recoverable until the decision. It is stored as XChaCha20-Poly1305
//! `nonce || ciphertext` with the request UUID as associated data. The 32-byte key
//! lives in the `settings` table (`request-key`, base64) and is generated on the
//! first account request.
use super::internal;
use super::store::Store;
use crate::error::ApiError;
use base64::Engine;
use chacha20poly1305::aead::{Aead, Generate, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rusqlite::{params, OptionalExtension};

/// `settings` row that keeps the sealing key.
const KEY_SETTING: &str = "request-key";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

impl Store {
    /// Encrypts a request password for storage in `users.request_secret`.
    pub(super) fn seal_secret(&self, request: &str, password: &str) -> Result<Vec<u8>, ApiError> {
        let cipher = self.request_cipher()?;
        let nonce = XNonce::try_generate().map_err(|_| internal())?;
        let mut sealed = nonce.to_vec();
        sealed.extend(
            cipher
                .encrypt(
                    &nonce,
                    Payload {
                        msg: password.as_bytes(),
                        aad: request.as_bytes(),
                    },
                )
                .map_err(|_| internal())?,
        );
        Ok(sealed)
    }
    /// Inverse of [`Store::seal_secret`]; a row sealed under another key or bound
    /// to another request id is refused instead of yielding garbage.
    pub(super) fn open_secret(&self, request: &str, sealed: &[u8]) -> Result<String, ApiError> {
        if sealed.len() <= NONCE_LEN {
            return Err(internal());
        }
        let cipher = self.request_cipher()?;
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        let nonce = XNonce::try_from(nonce).map_err(|_| internal())?;
        let plain = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: request.as_bytes(),
                },
            )
            .map_err(|_| internal())?;
        String::from_utf8(plain).map_err(|_| internal())
    }
    /// Loads the sealing key, creating it on first use. A malformed key is an
    /// error rather than a silent replacement: a new key would orphan every
    /// open request.
    fn request_cipher(&self) -> Result<XChaCha20Poly1305, ApiError> {
        let mut cached = self.request_key.lock().map_err(|_| internal())?;
        if let Some(key) = cached.as_ref() {
            return Ok(key.clone());
        }
        let engine = base64::engine::general_purpose::STANDARD;
        let read = || -> Result<Option<String>, ApiError> {
            Ok(self
                .db
                .query_row(
                    "SELECT value FROM settings WHERE key=?1",
                    [KEY_SETTING],
                    |r| r.get(0),
                )
                .optional()?)
        };
        let stored = match read()? {
            Some(text) => text,
            None => {
                let key = Key::try_generate().map_err(|_| internal())?;
                // `INSERT OR IGNORE` for the unlikely race of two hubs or two
                // requests generating a key at once; the first row wins.
                self.db.execute(
                    "INSERT OR IGNORE INTO settings(key,value) VALUES(?1,?2)",
                    params![KEY_SETTING, engine.encode(key.as_slice())],
                )?;
                read()?.ok_or_else(internal)?
            }
        };
        // Decodes without padding quirks: an undecodable row must not be ignored.
        let bytes = engine
            .decode(stored.trim())
            .ok()
            .filter(|bytes| bytes.len() == KEY_LEN)
            .ok_or_else(|| {
                eprintln!("hub: account request key in settings is corrupted");
                internal()
            })?;
        let key = Key::try_from(bytes.as_slice()).map_err(|_| internal())?;
        let cipher = XChaCha20Poly1305::new(&key);
        *cached = Some(cipher.clone());
        Ok(cipher)
    }
}
