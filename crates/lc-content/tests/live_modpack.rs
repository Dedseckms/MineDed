//! Live check (network, ignored by default): installs a small real Modrinth modpack
//! through the real `CoreServices` + loader registry and check that the
//! instance has everything needed to launch.
//!
//! `cargo test -p lc-content --test live_modpack -- --ignored --nocapture`
//! Modrinth pack: `MINEUP_LIVE_PACK` (slug/id), default `fabulously-optimized`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use lc_accounts::{AccountsService, MemoryTokenStore};
use lc_content::{ContentServices, PackIo};
use lc_core::{CoreServices, GameObserver, InstanceService};
use lc_http::{DownloaderConfig, HttpDownloader};
use lc_runtime::{NullSink, Paths};
use lc_types::content::ContentSourceKind;
use lc_types::events::{InstanceLogPayload, InstanceStatePayload};
use lc_types::instance::Instance;
use lc_types::version::LoaderKind;

struct Obs;

impl GameObserver for Obs {
    fn on_state(&self, _p: InstanceStatePayload) {}
    fn on_log(&self, _p: InstanceLogPayload) {}
}

struct Live {
    _dir: tempfile::TempDir,
    paths: Paths,
    core: Arc<CoreServices>,
    content: ContentServices,
}

fn live() -> Live {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    let dl: Arc<dyn lc_http::Downloader> =
        Arc::new(HttpDownloader::new(DownloaderConfig::default()).unwrap());
    let accounts = Arc::new(AccountsService::new(
        &paths,
        dl.clone(),
        Arc::new(MemoryTokenStore::new()),
    ));
    let loaders = Arc::new(lc_loaders::default_registry(dl.clone()));
    let core = Arc::new(CoreServices::new(
        paths.clone(),
        dl.clone(),
        accounts,
        loaders,
        Arc::new(Obs),
        Vec::new(),
    ));
    let content = ContentServices::new(paths.clone(), dl, core.clone(), core.clone());
    Live {
        _dir: dir,
        paths,
        core,
        content,
    }
}

impl Live {
    async fn check_launch_ready(&self, inst: &Instance) {
        println!("installed {} {} {:?}", inst.name, inst.mc, inst.loader);
        assert_ne!(inst.loader.kind, LoaderKind::Vanilla);
        assert!(inst.loader.version.is_some());
        let game = self.core.game_dir(inst.id);
        let jars = std::fs::read_dir(game.join("mods"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".jar"))
            .count();
        println!("{jars} mod jar(s)");
        assert!(jars > 0);
        // Vanilla client + libraries + assets are downloaded.
        assert!(self
            .paths
            .versions()
            .join(&inst.mc)
            .join(format!("{}.jar", inst.mc))
            .exists());
        assert!(std::fs::read_dir(self.paths.libraries())
            .unwrap()
            .next()
            .is_some());
        assert!(self.paths.assets().join("indexes").exists());
        // `repair` re-verifies every file and the loader without errors.
        self.core
            .repair(inst.id, &NullSink::default())
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "network: downloads a real modpack, vanilla files and the loader"]
async fn installs_real_modrinth_pack() {
    let pack = std::env::var("MINEUP_LIVE_PACK").unwrap_or_else(|_| "fabulously-optimized".into());
    let l = live();
    let inst = l
        .content
        .install_remote(
            ContentSourceKind::Modrinth,
            &pack,
            None,
            &NullSink::default(),
        )
        .await
        .unwrap();
    l.check_launch_ready(&inst).await;
}
