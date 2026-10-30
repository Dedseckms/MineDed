//! Durable chat state. Every mutation runs on the blocking pool under one transaction lock.
use super::{denied, invalid, Chat};
use crate::{auth::Identity, error::ApiError, rfc3339};
use lc_types::social::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) struct Store {
    pub db: Connection,
    pub files: PathBuf,
    dictionary: std::sync::Mutex<Option<Vec<u8>>>,
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(super) fn uuid(id: &str) -> Result<String, ApiError> {
    uuid::Uuid::parse_str(id)
        .ok()
        .filter(|v| !v.is_nil())
        .map(|v| v.simple().to_string())
        .ok_or_else(|| invalid("invalid profile UUID"))
}
fn pair(a: &str, b: &str) -> (String, String) {
    if a < b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}
pub(super) fn privacy_default() -> SocialPrivacy {
    SocialPrivacy {
        who_can_message: SocialAudience::Everyone,
        who_can_invite: SocialAudience::Everyone,
        who_sees_status: SocialAudience::Everyone,
        who_can_add_friend: SocialAudience::Everyone,
    }
}
/// Fixed accent palette (ids only, no colours); the UI mirrors the same list.
pub(crate) const ACCENTS: [&str; 8] = [
    "green", "teal", "sky", "violet", "pink", "red", "amber", "slate",
];
/// Adds a column to an existing table; schema v1 databases are upgraded in place.
fn ensure_column(db: &Connection, table: &str, column: &str, ddl: &str) -> Result<(), ApiError> {
    let present = {
        let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
        let names = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        names.iter().any(|name| name == column)
    };
    if !present {
        db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {ddl}"))?;
    }
    Ok(())
}
fn allowed(a: SocialAudience, friends: bool) -> bool {
    a == SocialAudience::Everyone || (a == SocialAudience::Friends && friends)
}

impl Store {
    pub fn open(root: &Path) -> Result<Self, ApiError> {
        std::fs::create_dir_all(root.join("files"))?;
        let db = Connection::open(root.join("chat.sqlite3"))?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS users(uuid TEXT PRIMARY KEY,name TEXT NOT NULL,skin TEXT,admin INTEGER NOT NULL DEFAULT 0,moderator INTEGER NOT NULL DEFAULT 0,privacy TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS channels(id TEXT PRIMARY KEY,a TEXT,b TEXT);
INSERT OR IGNORE INTO channels(id) VALUES('global');
CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY AUTOINCREMENT,channel TEXT NOT NULL REFERENCES channels(id),author TEXT NOT NULL REFERENCES users(uuid),body BLOB NOT NULL,created TEXT NOT NULL,deleted INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS message_history ON messages(channel,id);
CREATE TABLE IF NOT EXISTS reads(user TEXT,channel TEXT,last INTEGER NOT NULL,PRIMARY KEY(user,channel));
CREATE TABLE IF NOT EXISTS friends(a TEXT,b TEXT,sender TEXT NOT NULL,accepted INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(a,b));
CREATE TABLE IF NOT EXISTS blocks(owner TEXT,peer TEXT,PRIMARY KEY(owner,peer));
CREATE TABLE IF NOT EXISTS moderation(user TEXT PRIMARY KEY,mute INTEGER NOT NULL DEFAULT 0,ban INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS reports(id INTEGER PRIMARY KEY AUTOINCREMENT,message INTEGER NOT NULL REFERENCES messages(id),reporter TEXT NOT NULL,reason TEXT,created TEXT NOT NULL,resolved INTEGER NOT NULL DEFAULT 0,UNIQUE(message,reporter));
CREATE TABLE IF NOT EXISTS attachments(id TEXT PRIMARY KEY,owner TEXT NOT NULL,metadata TEXT NOT NULL,hash TEXT NOT NULL,thumb TEXT);
CREATE TABLE IF NOT EXISTS message_files(message INTEGER NOT NULL REFERENCES messages(id),file TEXT NOT NULL REFERENCES attachments(id),PRIMARY KEY(message,file));
CREATE INDEX IF NOT EXISTS attachment_messages ON message_files(file,message);
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS compression_dictionary(id INTEGER PRIMARY KEY,body BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS guest_tokens(token BLOB PRIMARY KEY,uuid TEXT NOT NULL,created TEXT NOT NULL,expires INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS guest_tokens_user ON guest_tokens(uuid);
CREATE TABLE IF NOT EXISTS audit(id INTEGER PRIMARY KEY AUTOINCREMENT,at TEXT NOT NULL,actor TEXT NOT NULL,action TEXT NOT NULL,target TEXT,note TEXT);
CREATE INDEX IF NOT EXISTS user_name ON users(lower(name));")?;
        // Schema v2: guest accounts, profile fields and the avatar blob.
        ensure_column(&db, "users", "guest", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&db, "users", "display_name", "TEXT")?;
        ensure_column(&db, "users", "bio", "TEXT")?;
        ensure_column(&db, "users", "accent", "TEXT")?;
        ensure_column(&db, "users", "avatar", "TEXT")?;
        let files: Option<String> = db
            .query_row("SELECT value FROM settings WHERE key='files'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let dictionary = db
            .query_row(
                "SELECT body FROM compression_dictionary WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let files = files
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("files"));
        if !files.is_dir() {
            return Err(super::internal());
        }
        Ok(Self {
            db,
            files,
            dictionary: std::sync::Mutex::new(dictionary),
        })
    }
    pub fn identify(
        &self,
        who: &Identity,
        admin: bool,
        skin: Option<&str>,
    ) -> Result<String, ApiError> {
        let id = uuid(&who.user_id)?;
        self.db.execute("INSERT INTO users(uuid,name,admin,skin,privacy) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(uuid) DO UPDATE SET name=excluded.name,admin=excluded.admin,skin=COALESCE(excluded.skin,users.skin)",
            params![id, who.name, admin, skin, serde_json::to_string(&privacy_default())?])?;
        Ok(id)
    }
    pub fn privacy(&self, id: &str) -> Result<SocialPrivacy, ApiError> {
        let raw: String = self
            .db
            .query_row("SELECT privacy FROM users WHERE uuid=?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| ApiError::not_found("player not found"))?;
        Ok(serde_json::from_str(&raw)?)
    }
    pub fn friends_with(&self, a: &str, b: &str) -> Result<bool, ApiError> {
        let (a, b) = pair(a, b);
        Ok(self
            .db
            .query_row(
                "SELECT accepted FROM friends WHERE a=?1 AND b=?2",
                params![a, b],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false))
    }
    pub fn blocked(&self, a: &str, b: &str) -> Result<bool, ApiError> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM blocks WHERE (owner=?1 AND peer=?2) OR (owner=?2 AND peer=?1))", params![a,b], |r| r.get(0))?)
    }
    pub fn role(&self, id: &str) -> Result<SocialRole, ApiError> {
        let (admin, moderator, guest): (bool, bool, bool) = self.db.query_row(
            "SELECT admin,moderator,guest FROM users WHERE uuid=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(if admin {
            SocialRole::Admin
        } else if moderator {
            SocialRole::Moderator
        } else if guest {
            SocialRole::Guest
        } else {
            SocialRole::User
        })
    }
    /// Guests are unverified offline nicks with hub-issued tokens: read-only access.
    pub fn is_guest(&self, id: &str) -> Result<bool, ApiError> {
        Ok(self
            .db
            .query_row("SELECT guest FROM users WHERE uuid=?1", [id], |r| {
                r.get::<_, bool>(0)
            })
            .optional()?
            .unwrap_or(false))
    }
    /// Display label for the admin presence list; `None` when the profile is unknown.
    pub(super) fn label(&self, id: &str) -> Result<Option<(String, bool)>, ApiError> {
        Ok(self
            .db
            .query_row("SELECT name,guest FROM users WHERE uuid=?1", [id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
            })
            .optional()?)
    }
    pub fn user(&self, viewer: &str, id: &str, chat: &Chat) -> Result<SocialUser, ApiError> {
        let (name, skin, guest, display_name, bio, accent): (
            String,
            Option<String>,
            bool,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = self
            .db
            .query_row(
                "SELECT name,skin,guest,display_name,bio,accent FROM users WHERE uuid=?1",
                [id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| ApiError::not_found("player not found"))?;
        let viewer_guest = self.is_guest(viewer)?;
        let friend = self.friends_with(viewer, id)?;
        let blocked = self.blocked(viewer, id)?;
        let p = self.privacy(id)?;
        // Direct chats and friend requests need two full accounts.
        let social = viewer != id && !blocked && !guest && !viewer_guest;
        let status = if viewer == id || (!blocked && allowed(p.who_sees_status, friend)) {
            Some(
                chat.presence
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(id)
                    .map(|v| v.status)
                    .unwrap_or(SocialStatus::Offline),
            )
        } else {
            None
        };
        Ok(SocialUser {
            uuid: id.into(),
            name,
            skin_url: skin,
            status,
            role: self.role(id)?,
            is_friend: friend,
            is_blocked: self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM blocks WHERE owner=?1 AND peer=?2)",
                params![viewer, id],
                |r| r.get(0),
            )?,
            can_message: social && allowed(p.who_can_message, friend),
            can_invite: social && (p.who_can_invite == SocialAudience::Everyone || friend),
            guest,
            display_name,
            bio,
            accent,
        })
    }
    /// Guests may only read the global chat: every writer path goes through here.
    fn forbid_guest(&self, id: &str) -> Result<(), ApiError> {
        if self.is_guest(id)? {
            return Err(super::guest_read_only());
        }
        Ok(())
    }
    /// Applies an own-profile patch. An empty string clears a field.
    pub fn update_profile(
        &self,
        me: &str,
        patch: ProfileUpdate,
        chat: &Chat,
    ) -> Result<SocialUser, ApiError> {
        if let Some(raw) = patch.display_name {
            let value = profile_text(&raw, 24, "display name")?;
            self.db.execute(
                "UPDATE users SET display_name=?1 WHERE uuid=?2",
                params![value, me],
            )?;
        }
        if let Some(raw) = patch.bio {
            let value = profile_text(&raw, 120, "bio")?;
            self.db
                .execute("UPDATE users SET bio=?1 WHERE uuid=?2", params![value, me])?;
        }
        if let Some(raw) = patch.accent {
            let raw = raw.trim();
            let value = if raw.is_empty() {
                None
            } else if ACCENTS.contains(&raw) {
                Some(raw)
            } else {
                return Err(invalid("unknown accent"));
            };
            self.db.execute(
                "UPDATE users SET accent=?1 WHERE uuid=?2",
                params![value, me],
            )?;
        }
        self.user(me, me, chat)
    }
    pub fn member(&self, viewer: &str, channel: &str) -> Result<Option<String>, ApiError> {
        if channel == "global" {
            return Ok(None);
        }
        let parts: Vec<_> = channel.split(':').collect();
        if parts.len() != 3
            || parts[0] != "dm"
            || uuid(parts[1])? != parts[1]
            || uuid(parts[2])? != parts[2]
            || parts[1] >= parts[2]
            || (viewer != parts[1] && viewer != parts[2])
        {
            return Err(denied());
        }
        let exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE id=?1 AND (a=?2 OR b=?2))",
            params![channel, viewer],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(denied());
        }
        Ok(Some(
            if viewer == parts[1] {
                parts[2]
            } else {
                parts[1]
            }
            .into(),
        ))
    }
    pub fn can_send(&self, viewer: &str, channel: &str, invite: bool) -> Result<(), ApiError> {
        let ban: bool = self
            .db
            .query_row("SELECT ban FROM moderation WHERE user=?1", [viewer], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or(false);
        if ban {
            return Err(denied());
        }
        if let Some(peer) = self.member(viewer, channel)? {
            let p = self.privacy(&peer)?;
            let friend = self.friends_with(viewer, &peer)?;
            if self.blocked(viewer, &peer)?
                || !allowed(p.who_can_message, friend)
                || (invite && p.who_can_invite != SocialAudience::Everyone && !friend)
            {
                return Err(denied());
            }
        } else {
            let mute: u64 = self
                .db
                .query_row("SELECT mute FROM moderation WHERE user=?1", [viewer], |r| {
                    r.get(0)
                })
                .optional()?
                .unwrap_or(0);
            if mute > now() {
                return Err(super::rejected("muted"));
            }
        }
        Ok(())
    }
    pub fn message(&self, viewer: &str, id: i64, chat: &Chat) -> Result<SocialMessage, ApiError> {
        let (channel, author, body, created, deleted): (String, String, Vec<u8>, String, bool) =
            self.db
                .query_row(
                    "SELECT channel,author,body,created,deleted FROM messages WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?
                .ok_or_else(|| ApiError::not_found("message not found"))?;
        self.member(viewer, &channel)?;
        // Decoding is bounded even if a database file is corrupted.
        let dictionary = self.dictionary.lock().map_err(|_| super::internal())?;
        let raw =
            zstd::bulk::Decompressor::with_dictionary(dictionary.as_deref().unwrap_or_default())
                .and_then(|mut d| d.decompress(&body, 32 * 1024))
                .map_err(|_| super::internal())?;
        drop(dictionary);
        let content = if deleted {
            SocialContent::Text {
                text: String::new(),
            }
        } else {
            serde_json::from_slice(&raw)?
        };
        Ok(SocialMessage {
            id: id.to_string(),
            channel,
            author: self.user(viewer, &author, chat)?,
            content,
            created_at: created,
            edited_at: None,
            deleted,
        })
    }
    pub fn history(
        &self,
        viewer: &str,
        channel: &str,
        before: Option<i64>,
        limit: usize,
        chat: &Chat,
    ) -> Result<SocialHistory, ApiError> {
        self.member(viewer, channel)?;
        let mut stmt = self.db.prepare(
            "SELECT id FROM messages WHERE channel=?1 AND id<?2 ORDER BY id DESC LIMIT ?3",
        )?;
        let mut ids = stmt
            .query_map(
                params![channel, before.unwrap_or(i64::MAX), limit + 1],
                |r| r.get::<_, i64>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let more = ids.len() > limit;
        ids.truncate(limit);
        ids.reverse();
        let messages = ids
            .iter()
            .map(|id| self.message(viewer, *id, chat))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SocialHistory {
            before: if more {
                ids.first().map(ToString::to_string)
            } else {
                None
            },
            messages,
        })
    }
    pub fn channel(&self, viewer: &str, id: &str, chat: &Chat) -> Result<SocialChannel, ApiError> {
        let peer = self.member(viewer, id)?;
        let unread:u32=self.db.query_row("SELECT count(*) FROM messages WHERE channel=?1 AND author<>?2 AND deleted=0 AND id>COALESCE((SELECT last FROM reads WHERE user=?2 AND channel=?1),0)",params![id,viewer],|r|r.get(0))?;
        let last: Option<i64> =
            self.db
                .query_row("SELECT max(id) FROM messages WHERE channel=?1", [id], |r| {
                    r.get(0)
                })?;
        Ok(SocialChannel {
            id: id.into(),
            kind: if peer.is_some() {
                SocialChannelKind::Direct
            } else {
                SocialChannelKind::Global
            },
            peer: peer.map(|p| self.user(viewer, &p, chat)).transpose()?,
            last_message: last.map(|id| self.message(viewer, id, chat)).transpose()?,
            unread,
        })
    }
    pub fn send(
        &self,
        viewer: &str,
        channel: &str,
        mut content: SocialContent,
        chat: &Chat,
    ) -> Result<SocialMessage, ApiError> {
        // Guests have no identity to moderate on: read-only everywhere, not just global.
        if self.is_guest(viewer)? {
            return Err(super::guest_read_only());
        }
        self.can_send(
            viewer,
            channel,
            matches!(
                content,
                SocialContent::RoomInvite { .. } | SocialContent::PackShare { .. }
            ),
        )?;
        let mut file = None;
        match &mut content {
            SocialContent::Text { text } => {
                validate_text(text, 8000)?;
                if channel == "global" && super::filter::filtered(text) {
                    return Err(super::rejected("filtered"));
                }
            }
            SocialContent::Attachment {
                attachment,
                caption,
            } => {
                if let Some(c) = caption {
                    validate_text(c, 2000)?;
                    if channel == "global" && super::filter::filtered(c) {
                        return Err(super::rejected("filtered"));
                    }
                }
                let raw: Option<String> = self
                    .db
                    .query_row(
                        "SELECT metadata FROM attachments WHERE id=?1 AND owner=?2",
                        params![attachment.id, viewer],
                        |r| r.get(0),
                    )
                    .optional()?;
                *attachment = serde_json::from_str(&raw.ok_or_else(denied)?)?;
                file = Some(attachment.id.clone());
            }
            SocialContent::RoomInvite { code, link } => {
                *code = crate::rooms::normalize_code(code)
                    .ok_or_else(|| invalid("invalid room code"))?;
                *link = format!("https://mineded.dedseckms.ru/join/{code}");
            }
            SocialContent::PackShare {
                name,
                mc,
                loader,
                code,
                ..
            } => {
                if channel == "global" {
                    return Err(invalid("pack sharing requires a direct chat"));
                }
                validate_text(name, 128)?;
                validate_text(mc, 64)?;
                validate_text(loader, 64)?;
                *code = Some(
                    crate::rooms::normalize_code(code.as_deref().unwrap_or_default())
                        .ok_or_else(|| invalid("invalid room code"))?,
                );
            }
        }
        self.train_dictionary()?;
        let dictionary = self.dictionary.lock().map_err(|_| super::internal())?;
        let body =
            zstd::bulk::Compressor::with_dictionary(3, dictionary.as_deref().unwrap_or_default())
                .and_then(|mut c| {
                    c.compress(&serde_json::to_vec(&content).map_err(std::io::Error::other)?)
                })
                .map_err(|_| super::internal())?;
        drop(dictionary);
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO messages(channel,author,body,created) VALUES(?1,?2,?3,?4)",
            params![channel, viewer, body, rfc3339::format(SystemTime::now())],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(file) = file {
            tx.execute(
                "INSERT INTO message_files(message,file) VALUES(?1,?2)",
                params![id, file],
            )?;
        }
        tx.commit()?;
        self.message(viewer, id, chat)
    }
    fn train_dictionary(&self) -> Result<(), ApiError> {
        let mut dictionary = self.dictionary.lock().map_err(|_| super::internal())?;
        if dictionary.is_some() {
            return Ok(());
        }
        let count: i64 = self
            .db
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?;
        if count != 200 {
            return Ok(());
        }
        let mut stmt = self
            .db
            .prepare("SELECT body FROM messages ORDER BY id LIMIT 200")?;
        let bodies = stmt
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let samples = bodies
            .iter()
            .map(|body| zstd::bulk::decompress(body, 32 * 1024))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| super::internal())?;
        // An empty dictionary records a failed training attempt; don't rescan an
        // indefinitely growing history when the first sample set lacks diversity.
        let trained = zstd::dict::from_samples(&samples, 8192).unwrap_or_default();
        self.db.execute(
            "INSERT OR IGNORE INTO compression_dictionary(id,body) VALUES(1,?1)",
            [&trained],
        )?;
        *dictionary = Some(trained);
        Ok(())
    }
    pub fn handle(
        &mut self,
        me: &str,
        method: &str,
        path: &[&str],
        q: &Value,
        body: Value,
        chat: &Chat,
    ) -> Result<Value, ApiError> {
        match (method, path) {
            ("GET", ["me"]) => Ok(serde_json::to_value(self.user(me, me, chat)?)?),
            ("GET", ["channels"]) => {
                let mut stmt=self.db.prepare("SELECT c.id FROM channels c LEFT JOIN messages m ON m.channel=c.id WHERE c.id='global' OR c.a=?1 OR c.b=?1 GROUP BY c.id ORDER BY (c.id='global') DESC,max(m.id) DESC")?;
                let mut ids = stmt
                    .query_map([me], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                // Guests never join direct chats, even if a stale row references them.
                if self.is_guest(me)? {
                    ids.retain(|id| id == "global");
                }
                Ok(serde_json::to_value(
                    ids.iter()
                        .map(|id| self.channel(me, id, chat))
                        .collect::<Result<Vec<_>, _>>()?,
                )?)
            }
            ("GET", ["channels", c, "messages"]) => {
                let before = q["before"]
                    .as_str()
                    .map(|s| s.parse::<i64>().map_err(|_| invalid("invalid cursor")))
                    .transpose()?;
                let limit = q["limit"]
                    .as_str()
                    .map(|s| s.parse::<usize>().map_err(|_| invalid("invalid limit")))
                    .transpose()?
                    .unwrap_or(50);
                if limit == 0 || limit > 100 {
                    return Err(invalid("limit must be 1..100"));
                }
                Ok(serde_json::to_value(
                    self.history(me, c, before, limit, chat)?,
                )?)
            }
            ("POST", ["channels", c, "messages"]) => {
                let message = self.send(
                    me,
                    c,
                    serde_json::from_value(body).map_err(|_| invalid("invalid message"))?,
                    chat,
                )?;
                let _ = chat.events.send(super::Notice::Message(
                    message.id.parse().map_err(|_| super::internal())?,
                    false,
                ));
                Ok(serde_json::to_value(message)?)
            }
            ("POST", ["channels", c, "read"]) => {
                self.member(me, c)?;
                let id = message_id(&body["messageId"])?;
                let exists: bool = self.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND channel=?2)",
                    params![id, c],
                    |r| r.get(0),
                )?;
                if !exists {
                    return Err(invalid("message does not belong to channel"));
                }
                self.db.execute("INSERT INTO reads(user,channel,last) VALUES(?1,?2,?3) ON CONFLICT(user,channel) DO UPDATE SET last=max(last,excluded.last)",params![me,c,id])?;
                Ok(Value::Null)
            }
            ("GET", ["privacy"]) => {
                self.forbid_guest(me)?;
                Ok(serde_json::to_value(self.privacy(me)?)?)
            }
            ("PUT", ["privacy"]) => {
                self.forbid_guest(me)?;
                let mut p: SocialPrivacy =
                    serde_json::from_value(body).map_err(|_| invalid("invalid privacy"))?;
                if p.who_can_invite == SocialAudience::Nobody {
                    p.who_can_invite = SocialAudience::Friends;
                }
                if p.who_can_add_friend == SocialAudience::Friends {
                    p.who_can_add_friend = SocialAudience::Nobody;
                }
                self.db.execute(
                    "UPDATE users SET privacy=?1 WHERE uuid=?2",
                    params![serde_json::to_string(&p)?, me],
                )?;
                Ok(serde_json::to_value(p)?)
            }
            ("GET", ["users"]) => {
                let prefix = q["q"].as_str().unwrap_or("");
                if prefix.chars().count() < 2 || prefix.len() > 64 {
                    return Err(invalid("search must contain 2..64 characters"));
                }
                let mut stmt=self.db.prepare("SELECT uuid FROM users WHERE substr(lower(name),1,length(?1))=lower(?1) ORDER BY name LIMIT 30")?;
                let ids = stmt
                    .query_map([prefix], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(serde_json::to_value(
                    ids.iter()
                        .map(|id| self.user(me, id, chat))
                        .collect::<Result<Vec<_>, _>>()?,
                )?)
            }
            ("GET", ["users", id]) => Ok(serde_json::to_value(self.user(me, &uuid(id)?, chat)?)?),
            ("GET", ["profile", id]) => {
                Ok(serde_json::to_value(self.user(me, &uuid(id)?, chat)?)?)
            }
            ("PATCH", ["profile"]) => {
                self.forbid_guest(me)?;
                let patch: ProfileUpdate =
                    serde_json::from_value(body).map_err(|_| invalid("invalid profile update"))?;
                Ok(serde_json::to_value(self.update_profile(me, patch, chat)?)?)
            }
            ("POST", ["users", id, "direct"]) => {
                self.forbid_guest(me)?;
                let id = uuid(id)?;
                if !self.user(me, &id, chat)?.can_message {
                    return Err(denied());
                }
                let (a, b) = pair(me, &id);
                let channel = format!("dm:{a}:{b}");
                self.db.execute(
                    "INSERT OR IGNORE INTO channels(id,a,b) VALUES(?1,?2,?3)",
                    params![channel, a, b],
                )?;
                Ok(serde_json::to_value(self.channel(me, &channel, chat)?)?)
            }
            ("PUT" | "DELETE", ["users", id, "block"]) => {
                let id = uuid(id)?;
                if id == me {
                    return Err(invalid("cannot block self"));
                }
                if method == "PUT" {
                    self.db.execute(
                        "INSERT OR IGNORE INTO blocks(owner,peer) VALUES(?1,?2)",
                        params![me, id],
                    )?;
                    let (a, b) = pair(me, &id);
                    self.db
                        .execute("DELETE FROM friends WHERE a=?1 AND b=?2", params![a, b])?;
                } else {
                    self.db.execute(
                        "DELETE FROM blocks WHERE owner=?1 AND peer=?2",
                        params![me, id],
                    )?;
                }
                let _ = chat.events.send(super::Notice::Friend(me.into(), id));
                Ok(Value::Null)
            }
            ("GET", ["friends"]) => {
                self.forbid_guest(me)?;
                let mut stmt = self.db.prepare(
                    "SELECT CASE WHEN a=?1 THEN b ELSE a END FROM friends WHERE a=?1 OR b=?1",
                )?;
                let ids = stmt
                    .query_map([me], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(serde_json::to_value(
                    ids.iter()
                        .map(|id| self.friend(me, id, chat))
                        .collect::<Result<Vec<_>, _>>()?,
                )?)
            }
            ("PUT" | "DELETE", ["friends", id]) => {
                self.forbid_guest(me)?;
                let id = uuid(id)?;
                let (a, b) = pair(me, &id);
                if method == "PUT" {
                    if id == me || self.blocked(me, &id)? {
                        return Err(denied());
                    }
                    let existing: Option<String> = self
                        .db
                        .query_row(
                            "SELECT sender FROM friends WHERE a=?1 AND b=?2",
                            params![a, b],
                            |r| r.get(0),
                        )
                        .optional()?;
                    if existing.as_deref() != Some(id.as_str())
                        && self.privacy(&id)?.who_can_add_friend != SocialAudience::Everyone
                    {
                        return Err(denied());
                    }
                    self.db.execute("INSERT INTO friends(a,b,sender) VALUES(?1,?2,?3) ON CONFLICT(a,b) DO UPDATE SET accepted=CASE WHEN sender<>excluded.sender THEN 1 ELSE accepted END",params![a,b,me])?;
                } else {
                    self.db
                        .execute("DELETE FROM friends WHERE a=?1 AND b=?2", params![a, b])?;
                }
                let _ = chat
                    .events
                    .send(super::Notice::Friend(me.into(), id.clone()));
                if method == "PUT" {
                    Ok(serde_json::to_value(self.friend(me, &id, chat)?)?)
                } else {
                    Ok(Value::Null)
                }
            }
            ("DELETE", ["messages", id]) => {
                let id = message_id(&json!(id))?;
                let m = self.message(me, id, chat)?;
                let role = self.role(me)?;
                let moderator = m.channel == "global"
                    && matches!(role, SocialRole::Moderator | SocialRole::Admin);
                if m.author.uuid != me && !moderator {
                    return Err(denied());
                }
                self.db
                    .execute("UPDATE messages SET deleted=1 WHERE id=?1", [id])?;
                if m.author.uuid != me {
                    self.audit(me, "delete", Some(&id.to_string()), Some(&m.channel))?;
                }
                let _ = chat.events.send(super::Notice::Message(id, true));
                Ok(Value::Null)
            }
            ("POST", ["messages", id, "report"]) => {
                self.forbid_guest(me)?;
                let id = message_id(&json!(id))?;
                let m = self.message(me, id, chat)?;
                if m.channel != "global" {
                    return Err(invalid("only public messages can be reported"));
                }
                let reason = body["reason"].as_str();
                if let Some(reason) = reason {
                    validate_text(reason, 1000)?;
                }
                self.db.execute("INSERT OR IGNORE INTO reports(message,reporter,reason,created) VALUES(?1,?2,?3,?4)",params![id,me,reason,rfc3339::format(SystemTime::now())])?;
                Ok(Value::Null)
            }
            (_, ["mod", ..]) => self.moderation(me, method, path, body, chat),
            (_, ["admin", ..]) => {
                if self.role(me)? != SocialRole::Admin {
                    return Err(denied());
                }
                match (method, path) {
                    ("PUT", ["admin", "roles", id]) => {
                        let id = uuid(id)?;
                        let role = body["role"]
                            .as_str()
                            .ok_or_else(|| invalid("role required"))?;
                        if !matches!(role, "user" | "moderator") {
                            return Err(invalid("invalid role"));
                        }
                        self.user(me, &id, chat)?;
                        if self.is_guest(&id)? {
                            return Err(invalid("guest accounts cannot hold roles"));
                        }
                        self.db.execute(
                            "UPDATE users SET moderator=?1 WHERE uuid=?2",
                            params![role == "moderator", id],
                        )?;
                        self.audit(me, "role", Some(&id), Some(role))?;
                        Ok(Value::Null)
                    }
                    ("GET", ["admin", "server"]) => Ok(serde_json::to_value(self.health(chat)?)?),
                    ("GET", ["admin", "presence"]) => Ok(serde_json::to_value(
                        super::presence::admin_presence(chat, self)?,
                    )?),
                    ("GET", ["admin", "audit"]) => Ok(serde_json::to_value(self.audit_page()?)?),
                    _ => Err(ApiError::not_found("no such route")),
                }
            }
            _ => Err(ApiError::not_found("no such route")),
        }
    }
    pub fn friend(&self, me: &str, id: &str, chat: &Chat) -> Result<SocialFriend, ApiError> {
        let (a, b) = pair(me, id);
        let (sender, accepted): (String, bool) = self.db.query_row(
            "SELECT sender,accepted FROM friends WHERE a=?1 AND b=?2",
            params![a, b],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(SocialFriend {
            user: self.user(me, id, chat)?,
            state: if accepted {
                FriendState::Friends
            } else if sender == me {
                FriendState::Outgoing
            } else {
                FriendState::Incoming
            },
        })
    }
    fn moderation(
        &self,
        me: &str,
        method: &str,
        path: &[&str],
        body: Value,
        chat: &Chat,
    ) -> Result<Value, ApiError> {
        let role = self.role(me)?;
        if !matches!(role, SocialRole::Moderator | SocialRole::Admin) {
            return Err(denied());
        }
        match (method, path) {
            ("GET", ["mod", "reports"]) => {
                let mut stmt=self.db.prepare("SELECT id,message,reporter,reason,created FROM reports WHERE resolved=0 ORDER BY id LIMIT 100")?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, String>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                let reports = rows
                    .into_iter()
                    .map(|(id, message, reporter, reason, created_at)| {
                        Ok(SocialReport {
                            id: id.to_string(),
                            message: self.message(me, message, chat)?,
                            reporter: self.user(me, &reporter, chat)?,
                            reason,
                            created_at,
                        })
                    })
                    .collect::<Result<Vec<_>, ApiError>>()?;
                Ok(serde_json::to_value(reports)?)
            }
            ("DELETE", ["mod", "reports", id]) => {
                let id = message_id(&json!(id))?;
                self.db
                    .execute("UPDATE reports SET resolved=1 WHERE id=?1", [id])?;
                Ok(Value::Null)
            }
            ("POST", ["mod", "users", id]) => {
                let id = uuid(id)?;
                let target = self.role(&id)?;
                if target == SocialRole::Admin
                    || (target == SocialRole::Moderator && role != SocialRole::Admin)
                {
                    return Err(denied());
                }
                let action = body["action"].as_str().unwrap_or("");
                let seconds = body["seconds"].as_u64().unwrap_or(0);
                match action {
                    "mute" => {
                        if seconds == 0
                            || seconds > i64::MAX as u64 - now()
                            || (role != SocialRole::Admin && seconds > 86400)
                        {
                            return Err(denied());
                        }
                        self.db.execute("INSERT INTO moderation(user,mute) VALUES(?1,?2) ON CONFLICT(user) DO UPDATE SET mute=excluded.mute",params![id,now()+seconds])?;
                        self.audit(me, action, Some(&id), Some(&seconds.to_string()))?;
                    }
                    "unmute" => {
                        self.db
                            .execute("UPDATE moderation SET mute=0 WHERE user=?1", [&id])?;
                        self.audit(me, action, Some(&id), None)?;
                    }
                    "ban" | "unban" => {
                        if role != SocialRole::Admin {
                            return Err(denied());
                        }
                        self.db.execute("INSERT INTO moderation(user,ban) VALUES(?1,?2) ON CONFLICT(user) DO UPDATE SET ban=excluded.ban",params![id,action=="ban"])?;
                        self.audit(me, action, Some(&id), None)?;
                    }
                    _ => return Err(invalid("invalid moderation action")),
                }
                Ok(Value::Null)
            }
            _ => Err(ApiError::not_found("no such route")),
        }
    }
}
pub(super) fn validate_text(text: &str, max: usize) -> Result<(), ApiError> {
    if text.trim().is_empty()
        || text.chars().count() > max
        || text.len() > max * 4
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(invalid("invalid text"));
    }
    Ok(())
}
/// Trims a profile field and rejects control characters. Empty means "clear".
fn profile_text(raw: &str, max: usize, what: &str) -> Result<Option<String>, ApiError> {
    let text = raw.trim();
    if text.chars().count() > max || text.chars().any(char::is_control) {
        return Err(invalid(&format!("invalid {what}")));
    }
    Ok((!text.is_empty()).then(|| text.to_owned()))
}
fn message_id(v: &Value) -> Result<i64, ApiError> {
    v.as_str()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("invalid message id"))
}
