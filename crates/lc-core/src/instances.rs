//! On-disk instance store: `instances/<id>/instance.json` + `minecraft/` game dir.

use std::path::{Path, PathBuf};

use lc_runtime::{Paths, ProgressSink};
use lc_types::instance::{Instance, InstancePatch};
use lc_types::{AppError, AppResult, ErrorCode, InstanceId};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::util::{io_err, join, read_json, write_json};

pub(crate) const INSTANCE_FILE: &str = "instance.json";
pub(crate) const GAME_DIR: &str = "minecraft";
pub(crate) const NATIVES_DIR: &str = "natives";

/// `instance.json`: the public DTO plus core-private fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstanceFile {
    #[serde(flatten)]
    pub instance: Instance,
    /// Version profile to launch (vanilla id or loader profile id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_profile: Option<String>,
}

impl InstanceFile {
    pub fn profile_id(&self) -> &str {
        self.version_profile.as_deref().unwrap_or(&self.instance.mc)
    }
}

pub(crate) struct InstanceStore {
    paths: Paths,
    write: Mutex<()>,
}

pub(crate) fn not_found(id: InstanceId) -> AppError {
    AppError::new(ErrorCode::NotFound, "instance not found").detail("id", id.to_string())
}

pub(crate) fn apply_patch(inst: &mut Instance, patch: InstancePatch) -> AppResult<()> {
    if let Some(name) = patch.name {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "empty instance name",
            ));
        }
        inst.name = name.to_owned();
    }
    if let Some(java) = patch.java {
        inst.java = java;
    }
    if let Some(mem) = patch.memory {
        if mem.max_mb < 256 || mem.min_mb > mem.max_mb {
            return Err(
                AppError::new(ErrorCode::InvalidInput, "invalid memory range")
                    .detail("minMb", mem.min_mb.to_string())
                    .detail("maxMb", mem.max_mb.to_string()),
            );
        }
        inst.memory = mem;
    }
    if let Some(args) = patch.jvm_args {
        inst.jvm_args = args;
    }
    if let Some(icon) = patch.icon {
        inst.icon = (!icon.is_empty()).then_some(icon);
    }
    if let Some(preset) = patch.preset {
        inst.preset = Some(preset);
    }
    Ok(())
}

impl InstanceStore {
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            write: Mutex::new(()),
        }
    }

    pub fn dir(&self, id: InstanceId) -> PathBuf {
        self.paths.instance(id)
    }

    pub fn game_dir(&self, id: InstanceId) -> PathBuf {
        self.dir(id).join(GAME_DIR)
    }

    pub fn natives_dir(&self, id: InstanceId) -> PathBuf {
        self.dir(id).join(NATIVES_DIR)
    }

    fn file(&self, id: InstanceId) -> PathBuf {
        self.dir(id).join(INSTANCE_FILE)
    }

    pub async fn load(&self, id: InstanceId) -> AppResult<InstanceFile> {
        match read_json::<InstanceFile>(&self.file(id)).await {
            Ok(f) => Ok(f),
            Err(e) if e.code == ErrorCode::NotFound => Err(not_found(id)),
            Err(e) => Err(e),
        }
    }

    pub async fn save(&self, file: &InstanceFile) -> AppResult<()> {
        let _g = self.write.lock().await;
        tokio::fs::create_dir_all(self.game_dir(file.instance.id))
            .await
            .map_err(|e| io_err(&self.game_dir(file.instance.id), e))?;
        write_json(&self.file(file.instance.id), file).await
    }

    /// Read-modify-write under the store lock.
    pub async fn modify(
        &self,
        id: InstanceId,
        f: impl FnOnce(&mut InstanceFile) -> AppResult<()>,
    ) -> AppResult<InstanceFile> {
        let _g = self.write.lock().await;
        let mut file = self.load(id).await?;
        f(&mut file)?;
        file.instance.id = id;
        write_json(&self.file(id), &file).await?;
        Ok(file)
    }

    /// All readable instances, most recently played first.
    pub async fn list(&self) -> AppResult<Vec<Instance>> {
        let mut out = Vec::new();
        let root = self.paths.instances();
        let mut rd = match tokio::fs::read_dir(&root).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(io_err(&root, e)),
        };
        while let Some(e) = rd.next_entry().await.map_err(|e| io_err(&root, e))? {
            let p = e.path().join(INSTANCE_FILE);
            if let Ok(f) = read_json::<InstanceFile>(&p).await {
                out.push(f.instance);
            }
        }
        out.sort_by(|a, b| {
            b.last_played
                .cmp(&a.last_played)
                .then_with(|| b.created_at.cmp(&a.created_at))
        });
        Ok(out)
    }

    pub async fn delete(&self, id: InstanceId) -> AppResult<()> {
        let _g = self.write.lock().await;
        let dir = self.dir(id);
        if !tokio::fs::try_exists(dir.join(INSTANCE_FILE))
            .await
            .unwrap_or(false)
        {
            return Err(not_found(id));
        }
        tokio::fs::remove_dir_all(&dir)
            .await
            .map_err(|e| io_err(&dir, e))
    }

    /// Removes a half-created instance directory (best effort).
    pub async fn discard(&self, id: InstanceId) {
        let _ = tokio::fs::remove_dir_all(self.dir(id)).await;
    }

    /// Copies `from`'s directory into `to` (except natives and instance.json).
    pub async fn copy_dir(
        &self,
        from: InstanceId,
        to: InstanceId,
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        let src = self.dir(from);
        let dst = self.dir(to);
        let files = join(
            tokio::task::spawn_blocking({
                let src = src.clone();
                move || list_files(&src)
            })
            .await,
        )?;
        let total = files.len() as u64;
        for (i, rel) in files.into_iter().enumerate() {
            sink.check_cancelled()?;
            let first = rel.components().next().map(|c| c.as_os_str().to_owned());
            if first.as_deref() == Some(std::ffi::OsStr::new(NATIVES_DIR))
                || rel == Path::new(INSTANCE_FILE)
            {
                continue;
            }
            let s = src.join(&rel);
            let d = dst.join(&rel);
            if let Some(parent) = d.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| io_err(parent, e))?;
            }
            tokio::fs::copy(&s, &d).await.map_err(|e| io_err(&s, e))?;
            if i % 32 == 0 {
                sink.progress(i as u64, Some(total), "stages.copy");
            }
        }
        sink.progress(total, Some(total), "stages.copy");
        Ok(())
    }
}

/// Relative paths of all regular files under `root` (symlinks are skipped).
fn list_files(root: &Path) -> AppResult<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = std::fs::read_dir(&dir).map_err(|e| io_err(&dir, e))?;
        for e in rd {
            let e = e.map_err(|e| io_err(&dir, e))?;
            let ft = e.file_type().map_err(|err| io_err(&e.path(), err))?;
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                if let Ok(rel) = e.path().strip_prefix(root) {
                    out.push(rel.to_path_buf());
                }
            }
        }
    }
    Ok(out)
}
