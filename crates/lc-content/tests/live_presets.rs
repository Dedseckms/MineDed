//! Live check (network, ignored by default): applies the Potato preset against the
//! real Modrinth API for each modded loader and checks that optimization mods land
//! in `mods/`, `options.txt` is written and the preset is persisted.
//!
//! `cargo test -p lc-content --test live_presets -- --ignored --nocapture`
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lc_accounts::{AccountsService, MemoryTokenStore};
use lc_content::{ContentServices, PresetService};
use lc_core::{CoreServices, GameObserver, InstanceService};
use lc_http::{DownloaderConfig, HttpDownloader};
use lc_runtime::{CancelToken, Paths, ProgressSink};
use lc_types::events::{InstanceLogPayload, InstanceStatePayload};
use lc_types::instance::{
    Instance, InstancePatch, InstanceSpec, JavaCfg, LoaderRef, MemoryCfg, PerfPreset,
};
use lc_types::task::TaskEvent;
use lc_types::version::LoaderKind;
use lc_types::{AppError, AppResult, ErrorCode, InstanceId, TaskId};

struct Obs;

impl GameObserver for Obs {
    fn on_state(&self, _p: InstanceStatePayload) {}
    fn on_log(&self, _p: InstanceLogPayload) {}
}

/// Instances kept in memory; no vanilla/loader download needed for this check.
struct Mem {
    root: PathBuf,
    map: Mutex<HashMap<InstanceId, Instance>>,
}

#[async_trait]
impl InstanceService for Mem {
    async fn list(&self) -> AppResult<Vec<Instance>> {
        Ok(self.map.lock().unwrap().values().cloned().collect())
    }
    async fn get(&self, id: InstanceId) -> AppResult<Instance> {
        self.map
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "instance"))
    }
    async fn create(&self, _s: InstanceSpec, _p: &dyn ProgressSink) -> AppResult<Instance> {
        Err(AppError::not_implemented("create"))
    }
    async fn clone_instance(
        &self,
        _id: InstanceId,
        _n: &str,
        _s: &dyn ProgressSink,
    ) -> AppResult<Instance> {
        Err(AppError::not_implemented("clone"))
    }
    async fn update(&self, id: InstanceId, patch: InstancePatch) -> AppResult<Instance> {
        let mut m = self.map.lock().unwrap();
        let i = m.get_mut(&id).unwrap();
        if let Some(j) = patch.jvm_args {
            i.jvm_args = j;
        }
        if let Some(mem) = patch.memory {
            i.memory = mem;
        }
        if let Some(p) = patch.preset {
            i.preset = Some(p);
        }
        Ok(i.clone())
    }
    async fn delete(&self, _id: InstanceId) -> AppResult<()> {
        Ok(())
    }
    async fn repair(&self, _id: InstanceId, _s: &dyn ProgressSink) -> AppResult<()> {
        Ok(())
    }
    fn game_dir(&self, id: InstanceId) -> PathBuf {
        self.root.join(id.to_string()).join("game")
    }
}

/// Prints task log lines.
struct Print(TaskId, CancelToken);

impl ProgressSink for Print {
    fn task_id(&self) -> TaskId {
        self.0
    }
    fn emit(&self, ev: TaskEvent) {
        if let TaskEvent::Log { line, .. } = ev {
            println!("  {line}");
        }
    }
    fn cancel_token(&self) -> &CancelToken {
        &self.1
    }
}

#[tokio::test]
#[ignore = "network: queries Modrinth and downloads optimization mods"]
async fn presets_install_real_mods() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    let dl: Arc<dyn lc_http::Downloader> =
        Arc::new(HttpDownloader::new(DownloaderConfig::default()).unwrap());
    let accounts = Arc::new(AccountsService::new(
        &paths,
        dl.clone(),
        Arc::new(MemoryTokenStore::new()),
    ));
    let core = Arc::new(CoreServices::new(
        paths.clone(),
        dl.clone(),
        accounts,
        Arc::new(lc_loaders::default_registry(dl.clone())),
        Arc::new(Obs),
        Vec::new(),
    ));
    let mem = Arc::new(Mem {
        root: paths.instance(InstanceId::new()),
        map: Mutex::new(HashMap::new()),
    });
    let content = ContentServices::new(paths.clone(), dl, mem.clone(), core);

    for (mc, kind) in [
        ("1.21.1", LoaderKind::Fabric),
        ("1.20.1", LoaderKind::Forge),
        ("1.21.1", LoaderKind::NeoForge),
    ] {
        let id = InstanceId::new();
        mem.map.lock().unwrap().insert(
            id,
            Instance {
                id,
                name: format!("{kind:?}"),
                mc: mc.into(),
                loader: LoaderRef {
                    kind,
                    version: Some("live".into()),
                },
                java: JavaCfg::default(),
                memory: MemoryCfg {
                    min_mb: 512,
                    max_mb: 2048,
                    auto: true,
                },
                jvm_args: Vec::new(),
                preset: None,
                icon: None,
                created_at: String::new(),
                last_played: None,
                playtime_sec: 0,
            },
        );
        println!("{kind:?} {mc}:");
        content
            .apply(
                id,
                PerfPreset::Potato,
                &Print(TaskId::new(), CancelToken::new()),
            )
            .await
            .unwrap();
        let game = mem.game_dir(id);
        let jars = std::fs::read_dir(game.join("mods")).unwrap().count();
        assert!(jars >= 3, "{kind:?}: {jars} jars");
        assert!(game.join("options.txt").exists());
        assert_eq!(mem.get(id).await.unwrap().preset, Some(PerfPreset::Potato));
    }
}
