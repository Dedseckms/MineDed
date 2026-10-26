//! What rooms need from the rest of the launcher: the local player and the
//! instance signature. `CoreRoomEnv` adapts the core/accounts services.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use lc_accounts::{AuthProvider, Secret, UserType};
use lc_core::InstanceService;
use lc_types::room::InstanceSig;
use lc_types::{AppError, AppResult, ErrorCode, InstanceId};
use sha2::{Digest, Sha256};

/// Local player shown to peers.
#[derive(Debug, Clone)]
pub struct Identity {
    pub name: String,
    pub uuid: Option<String>,
    /// Yggdrasil access token for hub `Authorization: Bearer`; `None` for
    /// offline accounts.
    pub hub_token: Option<Secret>,
}

#[async_trait]
pub trait RoomEnv: Send + Sync {
    /// Signature guests must match (MC version, loader, mod set).
    async fn instance_sig(&self, instance: InstanceId) -> AppResult<InstanceSig>;
    /// Default account; falls back to an anonymous player when none is set up.
    async fn identity(&self) -> AppResult<Identity>;
    /// Game directory of `instance` (guests add the room to its
    /// `servers.dat`). `None`: leave the server list alone.
    fn game_dir(&self, _instance: InstanceId) -> Option<PathBuf> {
        None
    }
}

/// `RoomEnv` on top of `InstanceService` + `AuthProvider`. The instance
/// service is attached after construction because core is built with the
/// rooms launch contributor (construction cycle).
pub struct CoreRoomEnv {
    auth: Arc<dyn AuthProvider>,
    instances: OnceLock<Arc<dyn InstanceService>>,
}

impl CoreRoomEnv {
    pub fn new(auth: Arc<dyn AuthProvider>) -> Self {
        Self {
            auth,
            instances: OnceLock::new(),
        }
    }

    /// Call once core is built. Later calls are ignored. Also removes room
    /// server entries a previous run left in the instances' `servers.dat`.
    pub fn attach_instances(&self, instances: Arc<dyn InstanceService>) {
        if self.instances.set(instances.clone()).is_ok() {
            spawn_servers_cleanup(instances);
        }
    }
}

/// Startup cleanup in the background; works with or without a tokio runtime
/// on the calling thread.
fn spawn_servers_cleanup(instances: Arc<dyn InstanceService>) {
    let job = async move {
        let list = match instances.list().await {
            Ok(l) => l,
            Err(e) => {
                tracing::debug!("rooms: servers.dat cleanup skipped: {}", e.message);
                return;
            }
        };
        let dirs: Vec<_> = list.iter().map(|i| instances.game_dir(i.id)).collect();
        let _ = tokio::task::spawn_blocking(move || crate::servers::cleanup(&dirs)).await;
    };
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(job);
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("rooms-cleanup".into())
        .spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt.block_on(job),
                Err(e) => tracing::debug!("rooms: servers.dat cleanup skipped: {e}"),
            }
        });
    if let Err(e) = spawned {
        tracing::debug!("rooms: servers.dat cleanup skipped: {e}");
    }
}

#[async_trait]
impl RoomEnv for CoreRoomEnv {
    async fn instance_sig(&self, instance: InstanceId) -> AppResult<InstanceSig> {
        let svc = self
            .instances
            .get()
            .ok_or_else(|| AppError::internal("rooms: instance service is not attached"))?;
        let inst = svc.get(instance).await?;
        let mods_hash = mods_hash(&svc.game_dir(instance).join("mods")).await?;
        Ok(InstanceSig {
            mc: inst.mc,
            loader: inst.loader.kind,
            loader_version: inst.loader.version,
            mods_hash,
        })
    }

    fn game_dir(&self, instance: InstanceId) -> Option<PathBuf> {
        self.instances.get().map(|s| s.game_dir(instance))
    }

    async fn identity(&self) -> AppResult<Identity> {
        match self.auth.launch_auth(None).await {
            Ok(a) => Ok(Identity {
                name: a.name,
                uuid: Some(a.uuid),
                hub_token: (a.user_type == UserType::Mojang).then_some(a.access_token),
            }),
            // No account at all: play anonymously.
            Err(e) if e.code == ErrorCode::AuthRequired => {
                tracing::info!("rooms: no default account, joining anonymously");
                Ok(Identity {
                    name: "Player".into(),
                    uuid: None,
                    hub_token: None,
                })
            }
            // An account exists but can't be used (session expired, auth
            // server down): say so instead of silently going anonymous, which
            // then failed later with a misleading auth-mode / hub error.
            Err(e) => Err(e.detail("source", "accounts")),
        }
    }
}

/// SHA-256 (hex) over the sorted SHA-256 hashes of enabled mod jars
/// (`mods/*.jar`), one per line. A missing folder hashes like an empty one.
pub async fn mods_hash(mods_dir: &Path) -> AppResult<String> {
    let mut hashes = Vec::new();
    match tokio::fs::read_dir(mods_dir).await {
        Ok(mut rd) => {
            while let Some(entry) = rd.next_entry().await? {
                let path = entry.path();
                let is_jar = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("jar"));
                if !is_jar || !entry.file_type().await?.is_file() {
                    continue;
                }
                let bytes = tokio::fs::read(&path).await?;
                hashes.push(hex::encode(Sha256::digest(&bytes)));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    hashes.sort();
    let mut h = Sha256::new();
    for x in &hashes {
        h.update(x.as_bytes());
        h.update(b"\n");
    }
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mods_hash_ignores_order_and_non_jars() {
        let base = std::env::temp_dir().join(format!("lc-p2p-mods-{}", uuid::Uuid::new_v4()));
        let a = base.join("a");
        let b = base.join("b");
        for d in [&a, &b] {
            tokio::fs::create_dir_all(d).await.unwrap();
        }
        tokio::fs::write(a.join("x.jar"), b"one").await.unwrap();
        tokio::fs::write(a.join("y.JAR"), b"two").await.unwrap();
        tokio::fs::write(a.join("z.jar.disabled"), b"off")
            .await
            .unwrap();
        tokio::fs::write(b.join("renamed.jar"), b"two")
            .await
            .unwrap();
        tokio::fs::write(b.join("other.jar"), b"one").await.unwrap();
        let ha = mods_hash(&a).await.unwrap();
        assert_eq!(ha, mods_hash(&b).await.unwrap());
        tokio::fs::write(b.join("extra.jar"), b"3").await.unwrap();
        assert_ne!(ha, mods_hash(&b).await.unwrap());
        let missing = mods_hash(&base.join("nope")).await.unwrap();
        assert_eq!(missing, hex::encode(Sha256::digest(b"")));
        let _ = tokio::fs::remove_dir_all(&base).await;
    }
}
