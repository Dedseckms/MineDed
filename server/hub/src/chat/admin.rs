//! Storage health and copy/verify/switch migration. Source objects are never deleted.
use super::{denied, internal, invalid, store::Store, Chat, Notice};
use crate::error::ApiError;
use lc_types::social::{AdminServerState, SocialRole};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;

fn conflict(message: &str) -> ApiError {
    ApiError::new(axum::http::StatusCode::CONFLICT, "invalidInput", message)
}
fn checksum(path: PathBuf, expected: &str) -> Result<(), ApiError> {
    if std::fs::metadata(&path)?.len() > super::files::MAX_UPLOAD as u64 {
        return Err(internal());
    }
    if format!("{:x}", Sha256::digest(std::fs::read(path)?)) != expected {
        return Err(conflict("attachment verification failed"));
    }
    Ok(())
}
pub(super) struct Plan {
    source: PathBuf,
    dest: PathBuf,
    hashes: Vec<String>,
}
impl Plan {
    fn copy_verify(&self, chat: &Chat) -> Result<(), ApiError> {
        let probe =
            super::files::persist(&self.dest, b"MineDed chat storage write verification v1")?;
        checksum(self.dest.join(&probe), &probe)?;
        let total = self.hashes.len().max(1);
        for (index, hash) in self.hashes.iter().enumerate() {
            checksum(self.source.join(hash), hash)?;
            let bytes = std::fs::read(self.source.join(hash))?;
            if super::files::persist(&self.dest, &bytes)? != *hash {
                return Err(conflict("destination verification failed"));
            }
            checksum(self.dest.join(hash), hash)?;
            *chat.migration.lock().map_err(|_| internal())? =
                Some((index + 1) as f32 / total as f32);
            if index % ((total / 20).max(1)) == 0 {
                let _ = chat.events.send(Notice::Admin);
            }
        }
        // A complete second pass precedes the persistent pointer switch.
        for hash in &self.hashes {
            checksum(self.dest.join(hash), hash)?;
        }
        Ok(())
    }
}
impl Chat {
    pub(super) async fn start_migration(
        self: &Arc<Self>,
        me: String,
        body: Value,
    ) -> Result<(), ApiError> {
        let plan = self
            .run(move |db, chat| {
                if db.role(&me)? != SocialRole::Admin {
                    return Err(denied());
                }
                db.prepare_migration(body, chat)
            })
            .await?;
        let chat = self.clone();
        let _ = chat.events.send(Notice::Admin);
        tokio::spawn(async move {
            let copy_chat = chat.clone();
            let copied = tokio::task::spawn_blocking(move || {
                plan.copy_verify(&copy_chat)?;
                Ok::<_, ApiError>(plan)
            })
            .await;
            let result=match copied{
                Ok(Ok(plan))=>chat.run(move|db,_|{
                    if std::fs::canonicalize(&db.files)?!=plan.source{return Err(conflict("storage root changed during migration"));}
                    let tx=db.db.transaction()?;
                    tx.execute("INSERT INTO settings(key,value) VALUES('files',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[plan.dest.to_string_lossy().as_ref()])?;tx.commit()?;
                    db.files=plan.dest;Ok(())
                }).await,
                Ok(Err(e))=>Err(e),Err(_)=>Err(internal()),
            };
            if result.is_err() {
                eprintln!("chat storage migration failed; active source retained");
            }
            if let Ok(mut progress) = chat.migration.lock() {
                *progress = None;
            }
            let _ = chat.events.send(Notice::Admin);
        });
        Ok(())
    }
}
impl Store {
    pub(super) fn health(&self, chat: &Chat) -> Result<AdminServerState, ApiError> {
        let total = fs2::total_space(&self.files)?;
        let free = fs2::free_space(&self.files)?;
        let used = total.saturating_sub(free);
        let (full, warning) = thresholds(used, total);
        if warning
            && chat.disk_warning.swap(
                super::store::now() / 3600,
                std::sync::atomic::Ordering::Relaxed,
            ) != super::store::now() / 3600
        {
            eprintln!(
                "chat disk warning: used={}%, attachments_paused={full}",
                u128::from(used) * 100 / u128::from(total)
            );
        }
        let backup_path = chat.root.join("backup-status.json");
        let backup = std::fs::metadata(&backup_path)
            .ok()
            .filter(|m| m.len() <= 4096)
            .and_then(|_| std::fs::read(&backup_path).ok())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        Ok(AdminServerState {
            disk_used_bytes: used,
            disk_total_bytes: total,
            attachments_paused: full,
            last_backup: backup
                .as_ref()
                .and_then(|v| v["lastBackup"].as_str().map(str::to_owned)),
            backup_error: backup
                .as_ref()
                .and_then(|v| v["backupError"].as_str().map(str::to_owned)),
            migration_progress: *chat.migration.lock().map_err(|_| internal())?,
        })
    }
    pub(super) fn prepare_migration(&self, body: Value, chat: &Chat) -> Result<Plan, ApiError> {
        let key = match body["target"].as_str() {
            Some("newDisk") => "HUB_CHAT_NEW_DISK_DIR",
            Some("objectStorage") => "HUB_CHAT_OBJECT_STORAGE_DIR",
            _ => return Err(invalid("invalid storage target")),
        };
        let dest = std::env::var_os(key)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| conflict("storage destination is not configured"))?;
        self.plan_destination(dest, chat)
    }
    fn plan_destination(&self, dest: PathBuf, chat: &Chat) -> Result<Plan, ApiError> {
        if !dest.is_absolute() || !dest.is_dir() {
            return Err(conflict(
                "destination must be an existing absolute directory",
            ));
        }
        let dest = std::fs::canonicalize(dest)?;
        let source = std::fs::canonicalize(&self.files)?;
        if dest.starts_with(&source) || source.starts_with(&dest) {
            return Err(conflict("destination must be separate from source"));
        }
        let mut stmt=self.db.prepare("SELECT hash FROM attachments UNION SELECT thumb FROM attachments WHERE thumb IS NOT NULL UNION SELECT avatar FROM users WHERE avatar IS NOT NULL")?;
        let hashes = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if hashes
            .iter()
            .any(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(internal());
        }
        let mut progress = chat.migration.lock().map_err(|_| internal())?;
        if progress.is_some() {
            return Err(conflict("migration already running"));
        }
        *progress = Some(0.0);
        Ok(Plan {
            source,
            dest,
            hashes,
        })
    }
}
fn thresholds(used: u64, total: u64) -> (bool, bool) {
    (
        total > 0 && u128::from(used) * 100 >= u128::from(total) * 95,
        total > 0 && u128::from(used) * 100 >= u128::from(total) * 80,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disk_thresholds_are_exact_and_overflow_safe() {
        assert_eq!(thresholds(79, 100), (false, false));
        assert_eq!(thresholds(80, 100), (false, true));
        assert_eq!(thresholds(94, 100), (false, true));
        assert_eq!(thresholds(95, 100), (true, true));
        assert_eq!(thresholds(u64::MAX, u64::MAX), (true, true));
    }
    #[test]
    #[allow(clippy::unwrap_used)]
    fn migration_copy_verifies_and_preserves_source() {
        let source = std::env::temp_dir().join(format!("chat-migrate-{}", uuid::Uuid::new_v4()));
        let dest = std::env::temp_dir().join(format!("chat-migrate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dest).unwrap();
        let chat = Chat::open(&source).unwrap();
        let db = chat.store.lock().unwrap();
        let hash = super::super::files::persist(&db.files, b"original").unwrap();
        db.db
            .execute(
                "INSERT INTO attachments(id,owner,metadata,hash) VALUES('a','owner','{}',?1)",
                [&hash],
            )
            .unwrap();
        let plan = db.plan_destination(dest.clone(), &chat).unwrap();
        plan.copy_verify(&chat).unwrap();
        assert_eq!(std::fs::read(dest.join(&hash)).unwrap(), b"original");
        assert!(plan.source.join(&hash).exists());
        std::fs::write(dest.join(&hash), b"corrupted").unwrap();
        assert!(plan.copy_verify(&chat).is_err());
        assert_eq!(std::fs::canonicalize(&db.files).unwrap(), plan.source);
        drop(db);
        drop(chat);
        let _ = std::fs::remove_dir_all(source);
        let _ = std::fs::remove_dir_all(dest);
    }
}
