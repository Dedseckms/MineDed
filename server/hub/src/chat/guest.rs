//! Guest accounts of offline nicknames: hub-issued tokens, read-only global chat.
//! Only a SHA-256 of the token is stored; the plaintext is returned once.
use super::store::{now, privacy_default, Store};
use super::{invalid, Chat};
use crate::auth::Identity;
use crate::error::ApiError;
use crate::rfc3339;
use lc_types::social::SocialUser;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Guest tokens live for a month; the launcher keeps them in the OS keyring.
const TOKEN_TTL_SECS: u64 = 30 * 24 * 3600;
/// Cap on guest accounts, so an anonymous endpoint cannot fill the disk.
const MAX_GUESTS: i64 = 10_000;
/// Older tokens of the same nickname are dropped beyond this many devices.
const MAX_TOKENS_PER_GUEST: i64 = 5;

/// Response of `POST /chat/guest` (hub.yaml): the token is shown exactly once.
pub(super) struct Issued {
    pub token: String,
    pub user: SocialUser,
    pub expires_at: String,
}
impl Chat {
    /// Signs an offline nickname in, reusing its profile on later logins.
    pub(super) async fn guest_login(self: &Arc<Self>, name: &str) -> Result<Issued, ApiError> {
        let name = validate_name(name)?;
        self.run(move |db, chat| db.guest_login(&name, chat)).await
    }
    /// Resolves a guest bearer; `Ok(None)` = unknown or expired token.
    pub(super) async fn guest_identity(
        self: &Arc<Self>,
        token: &str,
    ) -> Result<Option<Identity>, ApiError> {
        if token.is_empty() || token.len() > 128 {
            return Ok(None);
        }
        let token = token.to_owned();
        self.run(move |db, _| db.guest_identity(&token)).await
    }
}
impl Store {
    pub(super) fn guest_login(&self, name: &str, chat: &Chat) -> Result<Issued, ApiError> {
        // A registered server account keeps its name: no guest may borrow it.
        let taken: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE guest=0 AND lower(name)=lower(?1))",
            [name],
            |r| r.get(0),
        )?;
        if taken {
            return Err(ApiError::new(
                axum::http::StatusCode::CONFLICT,
                "conflict",
                "this name belongs to a registered account",
            ));
        }
        let existing: Option<String> = self
            .db
            .query_row(
                "SELECT uuid FROM users WHERE guest=1 AND lower(name)=lower(?1)",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        let uuid = match existing {
            Some(uuid) => uuid,
            None => {
                let guests: i64 =
                    self.db
                        .query_row("SELECT count(*) FROM users WHERE guest=1", [], |r| r.get(0))?;
                if guests >= MAX_GUESTS {
                    return Err(ApiError::rate_limited(3600));
                }
                let uuid = uuid::Uuid::new_v4().simple().to_string();
                self.db.execute(
                    "INSERT INTO users(uuid,name,privacy,guest) VALUES(?1,?2,?3,1)",
                    params![uuid, name, serde_json::to_string(&privacy_default())?],
                )?;
                uuid
            }
        };
        self.db
            .execute("DELETE FROM guest_tokens WHERE expires<=?1", [now()])?;
        // 256 random bits from the OS: two v4 UUIDs joined.
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        let expires = now().saturating_add(TOKEN_TTL_SECS);
        self.db.execute(
            "INSERT INTO guest_tokens(token,uuid,created,expires) VALUES(?1,?2,?3,?4)",
            params![
                hash,
                uuid,
                rfc3339::format(SystemTime::now()),
                expires as i64
            ],
        )?;
        self.db.execute(
            "DELETE FROM guest_tokens WHERE uuid=?1 AND token NOT IN (SELECT token FROM guest_tokens WHERE uuid=?1 ORDER BY expires DESC LIMIT ?2)",
            params![uuid, MAX_TOKENS_PER_GUEST],
        )?;
        Ok(Issued {
            token,
            user: self.user(&uuid, &uuid, chat)?,
            expires_at: rfc3339::format(UNIX_EPOCH + Duration::from_secs(expires)),
        })
    }
    pub(super) fn guest_identity(&self, token: &str) -> Result<Option<Identity>, ApiError> {
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        let row: Option<(String, String, bool)> = self
            .db
            .query_row(
                "SELECT u.uuid,u.name,COALESCE(m.ban,0) FROM guest_tokens t \
                 JOIN users u ON u.uuid=t.uuid \
                 LEFT JOIN moderation m ON m.user=u.uuid \
                 WHERE t.token=?1 AND t.expires>?2",
                params![hash, now()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        match row {
            // A chat ban revokes guest access together with its tokens.
            Some((_, _, true)) => Err(super::denied()),
            Some((user_id, name, false)) => Ok(Some(Identity { user_id, name })),
            None => Ok(None),
        }
    }
}

/// Guest nicknames follow the Minecraft rule: 3–16 letters, digits or `_`.
fn validate_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if !(3..=16).contains(&name.chars().count())
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(invalid(
            "name must be 3..=16 characters: letters, digits or underscore",
        ));
    }
    Ok(name.to_owned())
}
