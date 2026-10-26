//! Guest convenience: the tunnel's local address as a server entry at the
//! top of the instance's `servers.dat` (multiplayer screen), removed on leave
//! and on the next start if the launcher died in a room.
//!
//! Our entries are recognised by name prefix + loopback address; everything
//! else in the file is kept byte for byte. The game rereads the file whenever
//! the multiplayer screen opens, so editing it while the game runs is fine.

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::nbt::{self, Tag};

/// Name prefix of the entries we own.
pub(crate) const ENTRY_PREFIX: &str = "MineDed — ";
const FILE: &str = "servers.dat";
const TAG_COMPOUND: u8 = 10;

/// Serialises read-modify-write cycles within this process.
static EDIT: Mutex<()> = Mutex::new(());

pub(crate) fn servers_dat(game_dir: &Path) -> PathBuf {
    game_dir.join(FILE)
}

fn is_ours(entry: &Tag) -> bool {
    let prefix = nbt::to_mutf8(ENTRY_PREFIX);
    entry
        .get_str("name")
        .is_some_and(|n| n.starts_with(&prefix))
        && entry
            .get_str("ip")
            .is_some_and(|ip| ip.starts_with(b"127.0.0.1:"))
}

fn invalid(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// `servers` list of the root, created when missing or empty-typed.
fn servers_list(root: &mut Tag) -> io::Result<&mut Vec<Tag>> {
    let Tag::Compound(entries) = root else {
        return Err(invalid(nbt::NbtError::new("root is not a compound")));
    };
    let key = nbt::to_mutf8("servers");
    let idx = match entries.iter().position(|(k, _)| *k == key) {
        Some(i) => i,
        None => {
            entries.push((key, Tag::List(TAG_COMPOUND, Vec::new())));
            entries.len() - 1
        }
    };
    match &mut entries[idx].1 {
        Tag::List(elem, items) if *elem == TAG_COMPOUND || items.is_empty() => {
            *elem = TAG_COMPOUND;
            Ok(items)
        }
        _ => Err(invalid(nbt::NbtError::new(
            "servers is not a list of compounds",
        ))),
    }
}

fn parse(data: Option<&[u8]>) -> io::Result<(Vec<u8>, Tag)> {
    match data {
        Some(d) if !d.is_empty() => nbt::read(d).map_err(invalid),
        _ => Ok((Vec::new(), Tag::Compound(Vec::new()))),
    }
}

/// File contents with our entry for `addr` on top (older entries of ours dropped).
pub(crate) fn with_entry(
    data: Option<&[u8]>,
    host_name: &str,
    addr: SocketAddr,
) -> io::Result<Vec<u8>> {
    let (name, mut root) = parse(data)?;
    let list = servers_list(&mut root)?;
    list.retain(|e| !is_ours(e));
    let host: String = host_name.chars().filter(|c| !c.is_control()).collect();
    list.insert(
        0,
        Tag::Compound(vec![
            (
                b"name".to_vec(),
                Tag::string(&format!("{ENTRY_PREFIX}{host}")),
            ),
            (b"ip".to_vec(), Tag::string(&addr.to_string())),
            (b"hidden".to_vec(), Tag::Byte(0)),
        ]),
    );
    nbt::write(&name, &root).map_err(invalid)
}

/// File contents without our entries; `None` when there is nothing to remove.
pub(crate) fn without_entries(data: &[u8]) -> io::Result<Option<Vec<u8>>> {
    let (name, mut root) = parse(Some(data))?;
    let list = servers_list(&mut root)?;
    let before = list.len();
    list.retain(|e| !is_ours(e));
    if list.len() == before {
        return Ok(None);
    }
    nbt::write(&name, &root).map(Some).map_err(invalid)
}

fn read_opt(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(d) => Ok(Some(d)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Write to a sibling temp file, then rename over the target.
fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("dat.mineded.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Adds (or replaces) our entry at the top of `path`. Blocking.
pub(crate) fn add_entry(path: &Path, host_name: &str, addr: SocketAddr) -> io::Result<()> {
    let _g = EDIT.lock().unwrap_or_else(|e| e.into_inner());
    let data = read_opt(path)?;
    let out = with_entry(data.as_deref(), host_name, addr)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_atomic(path, &out)
}

/// Removes our entries from `path` (missing file: nothing to do). Blocking.
pub(crate) fn remove_entries(path: &Path) -> io::Result<bool> {
    let _g = EDIT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(data) = read_opt(path)? else {
        return Ok(false);
    };
    match without_entries(&data)? {
        Some(out) => write_atomic(path, &out).map(|()| true),
        None => Ok(false),
    }
}

/// Removes our entries from the `servers.dat` of each game dir (startup).
pub(crate) fn cleanup(game_dirs: &[PathBuf]) {
    for dir in game_dirs {
        let path = servers_dat(dir);
        match remove_entries(&path) {
            Ok(true) => tracing::info!("rooms: removed a stale room server entry"),
            Ok(false) => {}
            Err(e) => tracing::debug!("rooms: servers.dat cleanup skipped: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(out: &mut Vec<u8>, v: &str) {
        out.extend_from_slice(&u16::try_from(v.len()).unwrap().to_be_bytes());
        out.extend_from_slice(v.as_bytes());
    }

    /// Byte layout written by vanilla 1.21 `ServerList.save` (built by hand,
    /// independent of our writer): root "" { servers: [ {name, ip, icon,
    /// acceptTextures, hidden}, {name, ip, hidden} ] }.
    fn vanilla_sample() -> Vec<u8> {
        let mut b = vec![10];
        s(&mut b, "");
        b.push(9);
        s(&mut b, "servers");
        b.push(10);
        b.extend_from_slice(&2i32.to_be_bytes());
        // Entry 1.
        b.push(8);
        s(&mut b, "name");
        s(&mut b, "Мой сервер");
        b.push(8);
        s(&mut b, "ip");
        s(&mut b, "play.example.org");
        b.push(8);
        s(&mut b, "icon");
        s(&mut b, "iVBORw0KGgo=");
        b.push(1);
        s(&mut b, "acceptTextures");
        b.push(1);
        b.push(1);
        s(&mut b, "hidden");
        b.push(0);
        b.push(0);
        // Entry 2.
        b.push(8);
        s(&mut b, "name");
        s(&mut b, "Minecraft Server");
        b.push(8);
        s(&mut b, "ip");
        s(&mut b, "127.0.0.1:25565");
        b.push(1);
        s(&mut b, "hidden");
        b.push(0);
        b.push(0);
        // End of root.
        b.push(0);
        b
    }

    fn names(data: &[u8]) -> Vec<String> {
        let (_, mut root) = nbt::read(data).unwrap();
        servers_list(&mut root)
            .unwrap()
            .iter()
            .map(|e| String::from_utf8(e.get_str("name").unwrap().to_vec()).unwrap())
            .collect()
    }

    #[test]
    fn vanilla_file_round_trips() {
        let data = vanilla_sample();
        let (name, root) = nbt::read(&data).unwrap();
        assert_eq!(nbt::write(&name, &root).unwrap(), data);
        assert_eq!(without_entries(&data).unwrap(), None);
    }

    #[test]
    fn add_then_remove_keeps_other_servers() {
        let data = vanilla_sample();
        let addr: SocketAddr = "127.0.0.1:50123".parse().unwrap();
        let added = with_entry(Some(&data), "Steve\n", addr).unwrap();
        assert_eq!(
            names(&added),
            ["MineDed — Steve", "Мой сервер", "Minecraft Server"]
        );
        let (_, mut root) = nbt::read(&added).unwrap();
        let top = &servers_list(&mut root).unwrap()[0];
        assert_eq!(top.get_str("ip").unwrap(), b"127.0.0.1:50123");
        assert_eq!(top.get("hidden"), Some(&Tag::Byte(0)));

        // Rejoin replaces, never duplicates.
        let again = with_entry(Some(&added), "Alex", "127.0.0.1:50999".parse().unwrap()).unwrap();
        assert_eq!(
            names(&again),
            ["MineDed — Alex", "Мой сервер", "Minecraft Server"]
        );
        // Removal restores the original bytes exactly.
        assert_eq!(without_entries(&again).unwrap().unwrap(), data);
    }

    #[test]
    fn missing_or_bare_file() {
        let addr: SocketAddr = "127.0.0.1:4000".parse().unwrap();
        let fresh = with_entry(None, "Host", addr).unwrap();
        assert_eq!(names(&fresh), ["MineDed — Host"]);
        // Root without a servers list (or an empty list typed End).
        let bare = nbt::write(
            b"",
            &Tag::Compound(vec![(b"servers".to_vec(), Tag::List(0, vec![]))]),
        )
        .unwrap();
        assert_eq!(
            names(&with_entry(Some(&bare), "H", addr).unwrap()),
            ["MineDed — H"]
        );
        // Not NBT: refuse rather than clobber.
        assert!(with_entry(Some(&[0x1f, 0x8b, 8, 0]), "H", addr).is_err());
    }

    #[test]
    fn file_edits_are_atomic_and_cleanable() {
        let dir = std::env::temp_dir().join(format!("lc-p2p-servers-{}", uuid::Uuid::new_v4()));
        let path = servers_dat(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, vanilla_sample()).unwrap();
        add_entry(&path, "Host", "127.0.0.1:4000".parse().unwrap()).unwrap();
        assert_eq!(names(&std::fs::read(&path).unwrap())[0], "MineDed — Host");
        cleanup(std::slice::from_ref(&dir));
        assert_eq!(std::fs::read(&path).unwrap(), vanilla_sample());
        assert!(!remove_entries(&path).unwrap());
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(leftovers.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
