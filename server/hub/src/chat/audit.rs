//! Journal of moderation and admin actions shown in the admin panel
//! (`GET /chat/admin/audit`). Owner identity never leaves the server.
use super::store::Store;
use crate::error::ApiError;
use crate::rfc3339;
use lc_types::social::AuditEntry;
use rusqlite::params;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Entries returned by the admin panel; the table is trimmed at four times that.
const AUDIT_PAGE: usize = 200;
const AUDIT_KEEP: usize = AUDIT_PAGE * 4;

/// End of a running mute as RFC 3339; `0` (never muted) and an expired mute are `None`.
fn mute_end(mute: i64, now: u64) -> Option<String> {
    (mute > now as i64).then(|| rfc3339::format(UNIX_EPOCH + Duration::from_secs(mute as u64)))
}

impl Store {
    /// Records an action. `actor` and `target` are profile UUIDs; the actor name
    /// is resolved here so the journal survives a later rename by itself.
    pub(super) fn audit(
        &self,
        actor: &str,
        action: &str,
        target: Option<&str>,
        note: Option<&str>,
    ) -> Result<(), ApiError> {
        let name: String =
            self.db
                .query_row("SELECT name FROM users WHERE uuid=?1", [actor], |r| {
                    r.get(0)
                })?;
        self.db.execute(
            "INSERT INTO audit(at,actor,action,target,note) VALUES(?1,?2,?3,?4,?5)",
            params![
                rfc3339::format(SystemTime::now()),
                name,
                action,
                target,
                note
            ],
        )?;
        self.db.execute(
            "DELETE FROM audit WHERE id <= (SELECT id FROM audit ORDER BY id DESC LIMIT 1 OFFSET ?1)",
            [AUDIT_KEEP as i64],
        )?;
        Ok(())
    }
    /// Last `AUDIT_PAGE` actions, newest first.
    pub(super) fn audit_page(&self) -> Result<Vec<AuditEntry>, ApiError> {
        let mut stmt = self.db.prepare(
            "SELECT id,at,actor,action,target,note FROM audit ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([AUDIT_PAGE as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(id, at, actor, action, target, note)| AuditEntry {
                id: id.to_string(),
                at,
                actor,
                action,
                target,
                note,
            })
            .collect())
    }
}

impl Store {
    /// Records a freshly created Drasl invite for an account request.
    pub fn store_invite(
        &self,
        actor: &str,
        request: &str,
        code: &str,
        url: &str,
    ) -> Result<(), ApiError> {
        self.db.execute(
            "INSERT OR REPLACE INTO invites(request,code,url,created) VALUES(?1,?2,?3,?4)",
            params![
                request,
                code,
                url,
                crate::rfc3339::format(std::time::SystemTime::now())
            ],
        )?;
        self.audit(actor, "invite", Some(request), Some(code))
    }

    /// Guests waiting for an account, newest first. `login` is the Drasl name the
    /// guest asked for when it differs from the chat nickname.
    pub(super) fn account_requests(
        &self,
    ) -> Result<Vec<lc_types::social::AccountRequest>, ApiError> {
        let mut stmt = self.db.prepare(
            "SELECT uuid,name,request,created_at,username_hint FROM users \
             WHERE request_password IS NOT NULL ORDER BY created_at DESC LIMIT 200",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(lc_types::social::AccountRequest {
                id: r.get(0)?,
                name: r.get(1)?,
                note: r.get(2)?,
                created_at: r.get(3)?,
                login: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every account the launcher knows, newest first: joined players, chat
    /// guests and offline nicknames waiting for an account request.
    ///
    /// The response adds `accountCreated`, the requested `username` and the
    /// moderation state (`mutedUntil`, `banned`) to
    /// [`lc_types::social::AdminAccount`]; `mutedUntil` is RFC 3339 and `null`
    /// when no mute is running (expired mutes included).
    pub(super) fn admin_accounts(&self) -> Result<Vec<serde_json::Value>, ApiError> {
        let now = super::store::now();
        let mut stmt = self.db.prepare(
            "SELECT u.uuid,u.name,u.guest,u.display_name,u.created_at,i.code,i.url,u.account_created,u.username_hint,u.request,m.mute,m.ban \
             FROM users u LEFT JOIN invites i ON i.request=u.uuid \
             LEFT JOIN moderation m ON m.user=u.uuid \
             ORDER BY CASE WHEN u.created_at='' THEN 1 ELSE 0 END, u.created_at DESC LIMIT 500",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                lc_types::social::AdminAccount {
                    uuid: r.get(0)?,
                    name: r.get(1)?,
                    guest: r.get::<_, i64>(2)? != 0,
                    display_name: r.get(3)?,
                    created_at: r.get(4)?,
                    invite: r.get::<_, Option<String>>(5)?,
                    invite_url: r.get::<_, Option<String>>(6)?,
                    muted_until: None,
                    banned: r.get::<_, Option<i64>>(11)?.unwrap_or(0) != 0,
                },
                r.get::<_, bool>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<i64>>(10)?.unwrap_or(0),
            ))
        })?;
        let accounts = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(accounts
            .into_iter()
            .map(|(a, created, username, request, mute)| {
                serde_json::json!({"uuid": a.uuid, "name": a.name, "guest": a.guest,
                    "displayName": a.display_name, "createdAt": a.created_at,
                    "invite": a.invite, "inviteUrl": a.invite_url,
                    "accountCreated": created,
                    "username": if request.is_some() { username } else { None },
                    "mutedUntil": mute_end(mute, now), "banned": a.banned})
            })
            .collect())
    }
}
