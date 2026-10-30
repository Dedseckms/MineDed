//! Adding local files (drag-and-drop / file picker) to an instance.

use std::path::{Path, PathBuf};

use lc_types::content::{ContentKind, ContentSourceKind, InstalledContent};
use lc_types::{AppError, AppResult, ErrorCode, InstanceId};

use crate::index::{key, ContentIndex, IndexEntry};
use crate::service::Inner;
use crate::util::{
    check_file_name, ensure_no_links, file_hashes_blocking, io_err, is_link, join, kind_dir,
};

/// A validated source: path, bare name, folder or file.
#[derive(Debug)]
struct Item {
    src: PathBuf,
    name: String,
    is_dir: bool,
}

fn kind_str(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Mod => "mod",
        ContentKind::ResourcePack => "resourcePack",
        ContentKind::Shader => "shader",
        ContentKind::DataPack => "dataPack",
        ContentKind::Modpack => "modpack",
    }
}

fn wrong_type(path: &Path, kind: ContentKind, expected: &str) -> AppError {
    AppError::new(
        ErrorCode::InvalidInput,
        "file type does not match content kind",
    )
    .detail("reason", "wrongFileType")
    .detail("path", path.to_string_lossy())
    .detail("kind", kind_str(kind))
    .detail("expected", expected)
}

/// Mods: `.jar` files; resource packs, shaders and data packs: `.zip` files or folders.
fn validate(path: &str, kind: ContentKind) -> AppResult<Item> {
    let src = PathBuf::from(path);
    let meta = std::fs::metadata(&src).map_err(|e| io_err(&src, e))?;
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput, "path has no file name").detail("path", path)
        })?;
    check_file_name(&name)?;
    if name.starts_with('.') || name.to_ascii_lowercase().ends_with(".disabled") {
        return Err(AppError::new(ErrorCode::InvalidInput, "bad file name").detail("file", name));
    }
    let lower = name.to_ascii_lowercase();
    let (ok, expected) = match kind {
        ContentKind::Mod => (meta.is_file() && lower.ends_with(".jar"), ".jar"),
        _ => (
            meta.is_dir() || (meta.is_file() && lower.ends_with(".zip")),
            ".zip|folder",
        ),
    };
    if !ok {
        return Err(wrong_type(&src, kind, expected));
    }
    Ok(Item {
        src,
        name,
        is_dir: meta.is_dir(),
    })
}

fn remove_any(p: &Path) -> AppResult<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if is_link(&m) && m.is_dir() => std::fs::remove_dir(p).map_err(|e| io_err(p, e)),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p).map_err(|e| io_err(p, e)),
        Ok(_) => std::fs::remove_file(p).map_err(|e| io_err(p, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err(p, e)),
    }
}

/// Recursive copy of regular files and folders; symlinks are skipped.
fn copy_tree(src: &Path, dst: &Path) -> AppResult<()> {
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((from_dir, to_dir)) = stack.pop() {
        std::fs::create_dir_all(&to_dir).map_err(|e| io_err(&to_dir, e))?;
        let rd = std::fs::read_dir(&from_dir).map_err(|e| io_err(&from_dir, e))?;
        for e in rd {
            let e = e.map_err(|e| io_err(&from_dir, e))?;
            let from = e.path();
            let to = to_dir.join(e.file_name());
            let meta = std::fs::symlink_metadata(&from).map_err(|err| io_err(&from, err))?;
            if is_link(&meta) {
                continue;
            }
            let ft = meta.file_type();
            if ft.is_dir() {
                stack.push((from, to));
            } else if ft.is_file() {
                std::fs::copy(&from, &to).map_err(|e| io_err(&to, e))?;
            }
        }
    }
    Ok(())
}

/// Copies `item` into `dest_dir` through a hidden temp name, then swaps it in
/// (replacing an enabled or disabled copy with the same name).
fn install_item(item: &Item, dest_dir: &Path) -> AppResult<()> {
    ensure_no_links(dest_dir)?;
    std::fs::create_dir_all(dest_dir).map_err(|e| io_err(dest_dir, e))?;
    let dest = dest_dir.join(&item.name);
    let disabled = dest_dir.join(format!("{}.disabled", item.name));
    if item.is_dir {
        // Refuse copying a folder into itself (e.g. picking `resourcepacks/`).
        let src = std::fs::canonicalize(&item.src).map_err(|e| io_err(&item.src, e))?;
        let dst = std::fs::canonicalize(dest_dir).map_err(|e| io_err(dest_dir, e))?;
        if dst.starts_with(&src) {
            return Err(
                AppError::new(ErrorCode::InvalidInput, "folder contains the destination")
                    .detail("path", item.src.to_string_lossy()),
            );
        }
    }
    let tmp = dest_dir.join(format!(".{}.part", item.name));
    remove_any(&tmp)?;
    let copied = if item.is_dir {
        copy_tree(&item.src, &tmp)
    } else {
        std::fs::copy(&item.src, &tmp)
            .map(|_| ())
            .map_err(|e| io_err(&tmp, e))
    };
    if let Err(e) = copied {
        let _ = remove_any(&tmp);
        return Err(e);
    }
    let swapped = remove_any(&dest)
        .and_then(|_| remove_any(&disabled))
        .and_then(|_| std::fs::rename(&tmp, &dest).map_err(|e| io_err(&dest, e)));
    if swapped.is_err() {
        let _ = remove_any(&tmp);
    }
    swapped
}

impl Inner {
    pub async fn add_local(
        &self,
        id: InstanceId,
        kind: ContentKind,
        paths: Vec<String>,
    ) -> AppResult<Vec<InstalledContent>> {
        self.instances.get(id).await?;
        let dir = kind_dir(kind)?;
        if paths.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidInput, "no files given"));
        }
        let dest_dir = self.instances.game_dir(id).join(dir);
        let inst_dir = self.instance_dir(id);

        // Validate everything before copying anything; later duplicates win.
        let entries = join(
            tokio::task::spawn_blocking(move || -> AppResult<Vec<(String, IndexEntry)>> {
                let mut items: Vec<Item> = Vec::new();
                for p in &paths {
                    let it = validate(p, kind)?;
                    items.retain(|o| !o.name.eq_ignore_ascii_case(&it.name));
                    items.push(it);
                }
                let mut out = Vec::new();
                for it in &items {
                    install_item(it, &dest_dir)?;
                    let (sha1, sha512, size) = if it.is_dir {
                        (None, None, None)
                    } else {
                        let dest = dest_dir.join(&it.name);
                        let (a, b) = file_hashes_blocking(&dest)?;
                        let size = std::fs::metadata(&dest).ok().map(|m| m.len());
                        (Some(a), Some(b), size)
                    };
                    let k = key(dir, &it.name);
                    out.push((
                        it.name.clone(),
                        IndexEntry {
                            kind,
                            source: ContentSourceKind::Local,
                            project_id: k,
                            version_id: sha1.clone().unwrap_or_default(),
                            title: None,
                            version_number: None,
                            sha1,
                            sha512,
                            size,
                            url: None,
                        },
                    ));
                }
                Ok(out)
            })
            .await,
        )?;

        let mut idx = ContentIndex::load(&inst_dir).await?;
        let mut added = Vec::with_capacity(entries.len());
        for (name, entry) in entries {
            idx.files.insert(key(dir, &name), entry);
            added.push(InstalledContent {
                file_name: name,
                kind,
                enabled: true,
                source: ContentSourceKind::Local,
                project_id: None,
                version_id: None,
                title: None,
                version_number: None,
                update_available: false,
            });
        }
        idx.save(&inst_dir).await?;
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_types() {
        let d = tempfile::tempdir().unwrap();
        let jar = d.path().join("a.jar");
        let zip = d.path().join("p.zip");
        let folder = d.path().join("Pack");
        std::fs::write(&jar, "j").unwrap();
        std::fs::write(&zip, "z").unwrap();
        std::fs::create_dir(&folder).unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();

        assert!(validate(&s(&jar), ContentKind::Mod).is_ok());
        let e = validate(&s(&zip), ContentKind::Mod).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
        assert!(validate(&s(&folder), ContentKind::Mod).is_err());
        assert!(validate(&s(&zip), ContentKind::ResourcePack).is_ok());
        assert!(validate(&s(&folder), ContentKind::Shader).unwrap().is_dir);
        assert!(validate(&s(&jar), ContentKind::DataPack).is_err());
        let missing = d.path().join("none.jar");
        assert_eq!(
            validate(&s(&missing), ContentKind::Mod).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn refuses_folder_into_itself() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("resourcepacks");
        std::fs::create_dir(&root).unwrap();
        let it = Item {
            src: root.clone(),
            name: "resourcepacks".into(),
            is_dir: true,
        };
        assert!(install_item(&it, &root).is_err());
        assert!(!root.join("resourcepacks").exists());
    }

    #[test]
    fn local_pack_copy_skips_redirected_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "private").unwrap();
        let src = dir.path().join("source");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("pack.mcmeta"), "{}").unwrap();
        crate::util::link_dir(outside.path(), &src.join("assets"));
        let item = Item {
            src,
            name: "pack".into(),
            is_dir: true,
        };
        let dest = dir.path().join("resourcepacks");
        install_item(&item, &dest).unwrap();
        assert!(dest.join("pack/pack.mcmeta").exists());
        assert!(!dest.join("pack/assets").exists());

        crate::util::link_dir(outside.path(), &dir.path().join("redirected"));
        assert!(install_item(&item, &dir.path().join("redirected")).is_err());
        assert!(!outside.path().join("pack").exists());
    }
}
