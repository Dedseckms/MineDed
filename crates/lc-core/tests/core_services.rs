//! End-to-end tests of `CoreServices` with a mock `Downloader` (no network, no Java).

// Test-only helpers (mock, fixtures) live outside `#[test]` fns.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lc_accounts::{AuthProvider, LaunchAuth, Secret, UserType};
use lc_core::{
    CoreServices, GameLauncher, GameObserver, InstanceService, SystemProbe, VersionService,
};
use lc_http::{DownloadReq, Downloader, ResourceKind, Route, StubDownloader};
use lc_loaders::LoaderRegistry;
use lc_runtime::{CancelToken, Paths, ProgressSink};
use lc_types::events::{InstanceLogPayload, InstanceStatePayload};
use lc_types::instance::{InstancePatch, InstanceSpec, JavaCfg, LaunchOpts, LoaderRef, RunState};
use lc_types::settings::ProxyCfg;
use lc_types::task::TaskEvent;
use lc_types::version::{LoaderKind, VersionFilter, VersionKind};
use lc_types::{AccountId, AppError, AppResult, ErrorCode, TaskId};
use url::Url;

const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn sha1(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    hex::encode(Sha1::digest(bytes))
}

#[derive(Default)]
struct MockDl {
    files: Mutex<HashMap<String, Vec<u8>>>,
    requested: Mutex<Vec<DownloadReq>>,
    offline: AtomicBool,
}

impl MockDl {
    fn serve(&self, url: &str, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(url.to_owned(), bytes);
    }

    fn offline_err() -> AppError {
        AppError::new(ErrorCode::Network, "offline")
    }
}

#[async_trait]
impl Downloader for MockDl {
    async fn fetch_many(&self, reqs: Vec<DownloadReq>, sink: &dyn ProgressSink) -> AppResult<()> {
        if self.offline.load(Ordering::SeqCst) {
            return Err(Self::offline_err());
        }
        let total = reqs.len() as u64;
        for (i, r) in reqs.iter().enumerate() {
            let body = self
                .files
                .lock()
                .unwrap()
                .get(r.urls[0].as_str())
                .cloned()
                .unwrap_or_else(|| b"stub".to_vec());
            std::fs::create_dir_all(r.dest.parent().unwrap()).unwrap();
            std::fs::write(&r.dest, body).unwrap();
            sink.progress(i as u64 + 1, Some(total), "downloader.internal");
        }
        self.requested.lock().unwrap().extend(reqs);
        Ok(())
    }

    async fn get_bytes(&self, url: &Url, _kind: ResourceKind) -> AppResult<Vec<u8>> {
        if self.offline.load(Ordering::SeqCst) {
            return Err(Self::offline_err());
        }
        self.files
            .lock()
            .unwrap()
            .get(url.as_str())
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, url.to_string()))
    }

    /// Delegates to the stub; the mock never builds its own HTTP client.
    fn client(&self, route: Route) -> reqwest::Client {
        StubDownloader.client(route)
    }

    fn mirrors_for(&self, original: &Url, _kind: ResourceKind) -> Vec<Url> {
        vec![original.clone()]
    }

    fn set_manual_proxy(&self, _proxy: Option<ProxyCfg>) {}
}

struct Auth;

#[async_trait]
impl AuthProvider for Auth {
    async fn launch_auth(&self, _account: Option<AccountId>) -> AppResult<LaunchAuth> {
        Ok(LaunchAuth {
            name: "Steve".into(),
            uuid: "00000000000000000000000000000001".into(),
            access_token: Secret::new("0"),
            user_type: UserType::Legacy,
            authlib: None,
        })
    }
}

#[derive(Default)]
struct Obs {
    states: Mutex<Vec<RunState>>,
}

impl GameObserver for Obs {
    fn on_state(&self, p: InstanceStatePayload) {
        self.states.lock().unwrap().push(p.state);
    }
    fn on_log(&self, _p: InstanceLogPayload) {}
}

#[derive(Default)]
struct RecSink {
    id: TaskId,
    cancel: CancelToken,
    stages: Mutex<Vec<String>>,
}

impl ProgressSink for RecSink {
    fn task_id(&self) -> TaskId {
        self.id
    }
    fn emit(&self, event: TaskEvent) {
        if let TaskEvent::Progress { stage, .. } = event {
            let mut s = self.stages.lock().unwrap();
            if s.last() != Some(&stage) {
                s.push(stage);
            }
        }
    }
    fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
}

const OLD_JSON: &str = r#"{"id":"a1.0.4","type":"old_alpha","mainClass":"net.minecraft.launchwrapper.Launch",
  "minecraftArguments":"${auth_player_name} ${auth_session} --gameDir ${game_directory} --assetsDir ${game_assets}",
  "assetIndex":{"id":"legacy","url":"https://piston-meta.mojang.com/legacy.json"},
  "assets":"legacy",
  "downloads":{"client":{"url":"https://piston-data.mojang.com/a1.0.4.jar"}},
  "libraries":[]}"#;

struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
    dl: Arc<MockDl>,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let dl = Arc::new(MockDl::default());
    let v121 = fixture("1.21.1.json");
    let v112 = fixture("1.12.2.json");
    let manifest = format!(
        r#"{{"latest":{{"release":"1.21.1","snapshot":"1.21.1"}},"versions":[
        {{"id":"1.21.1","type":"release","url":"https://piston-meta.mojang.com/v1/1.21.1.json","sha1":"{}","releaseTime":"2024-08-08T12:24:45+00:00"}},
        {{"id":"1.12.2","type":"release","url":"https://piston-meta.mojang.com/v1/1.12.2.json","sha1":"{}","releaseTime":"2017-09-18T08:39:46+00:00"}},
        {{"id":"a1.0.4","type":"old_alpha","url":"https://piston-meta.mojang.com/v1/a1.0.4.json","sha1":"{}","releaseTime":"2010-07-09T00:00:00+00:00"}}]}}"#,
        sha1(&v121),
        sha1(&v112),
        sha1(OLD_JSON.as_bytes())
    );
    dl.serve(MANIFEST, manifest.into_bytes());
    dl.serve("https://piston-meta.mojang.com/v1/1.21.1.json", v121);
    dl.serve("https://piston-meta.mojang.com/v1/1.12.2.json", v112);
    dl.serve(
        "https://piston-meta.mojang.com/v1/a1.0.4.json",
        OLD_JSON.as_bytes().to_vec(),
    );
    let index = r#"{"objects":{"icons/icon_16x16.png":{"hash":"bdf48ef6b5d0d23bbb02e17d04865216179f510a","size":4}}}"#;
    dl.serve(
        "https://piston-meta.mojang.com/v1/packages/15200809663f45954c2c9a33ce88849e61aaaecc/17.json",
        index.as_bytes().to_vec(),
    );
    dl.serve(
        "https://piston-meta.mojang.com/v1/packages/a21e1ded1a24ea1548dd8db0cf30b6acb02655a9/1.12.json",
        index.as_bytes().to_vec(),
    );
    let legacy = r#"{"virtual":true,"objects":{"sound/step/grass1.ogg":{"hash":"aa11223344556677889900aabbccddeeff001122","size":4}}}"#;
    dl.serve(
        "https://piston-meta.mojang.com/legacy.json",
        legacy.as_bytes().to_vec(),
    );
    World {
        _dir: dir,
        root,
        dl,
    }
}

fn services(w: &World, obs: Arc<Obs>) -> CoreServices {
    CoreServices::new(
        Paths::new(&w.root),
        w.dl.clone(),
        Arc::new(Auth),
        Arc::new(LoaderRegistry::default()),
        obs,
        Vec::new(),
    )
}

fn spec(name: &str, mc: &str) -> InstanceSpec {
    InstanceSpec {
        name: name.into(),
        mc: mc.into(),
        loader: LoaderRef {
            kind: LoaderKind::Vanilla,
            version: None,
        },
        preset: None,
        icon: None,
    }
}

#[tokio::test]
async fn versions_list_works_offline_from_cache() {
    let w = world();
    let core = services(&w, Arc::default());
    let all = VersionService::list(&core, VersionFilter::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    let old = VersionService::list(
        &core,
        VersionFilter {
            kinds: vec![VersionKind::OldAlpha],
            search: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(old[0].id, "a1.0.4");
    assert_eq!(core.java_major("1.21.1").await.unwrap(), 21);
    assert_eq!(core.java_major("1.12.2").await.unwrap(), 8);
    assert_eq!(core.java_major("a1.0.4").await.unwrap(), 8);

    // New process, no network: manifest comes from the disk cache, java from local jsons.
    w.dl.offline.store(true, Ordering::SeqCst);
    let offline = services(&w, Arc::default());
    let list = VersionService::list(&offline, VersionFilter::default())
        .await
        .unwrap();
    assert_eq!(list.len(), 3);
    let v121 = list.iter().find(|v| v.id == "1.21.1").unwrap();
    assert_eq!(v121.java_major, Some(21));
    assert_eq!(offline.java_major("1.12.2").await.unwrap(), 8);
}

#[tokio::test]
async fn tampered_version_json_is_redownloaded() {
    let w = world();
    let core = services(&w, Arc::default());
    let p = w.root.join("versions/1.21.1/1.21.1.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, br#"{"id":"1.21.1","javaVersion":{"majorVersion":99}}"#).unwrap();
    assert_eq!(core.java_major("1.21.1").await.unwrap(), 21);
}

#[tokio::test]
async fn create_vanilla_and_crud() {
    let w = world();
    let core = services(&w, Arc::default());
    let sink = RecSink::default();
    let inst = core
        .create(spec("  Main  ", "1.21.1"), &sink)
        .await
        .unwrap();
    assert_eq!(inst.name, "Main");
    assert!(inst.memory.auto);
    assert!(inst.memory.max_mb >= 1024);

    let stages = sink.stages.lock().unwrap().clone();
    for s in [
        "stages.versionMeta",
        "stages.client",
        "stages.libraries",
        "stages.assets",
    ] {
        assert!(stages.iter().any(|x| x == s), "missing {s} in {stages:?}");
    }
    assert!(!stages.iter().any(|x| x == "downloader.internal"));

    let root = &w.root;
    assert!(root.join("versions/1.21.1/1.21.1.json").exists());
    assert!(root.join("versions/1.21.1/1.21.1.jar").exists());
    assert!(root.join("assets/indexes/17.json").exists());
    assert!(root
        .join("assets/objects/bd/bdf48ef6b5d0d23bbb02e17d04865216179f510a")
        .exists());
    assert!(root
        .join("libraries/org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar")
        .exists());
    let dir = root.join("instances").join(inst.id.to_string());
    assert!(dir.join("instance.json").exists());
    assert_eq!(core.game_dir(inst.id), dir.join("minecraft"));
    assert!(dir.join("minecraft").is_dir());

    // Client jar request carries the sha1 from the version json.
    let reqs = w.dl.requested.lock().unwrap().clone();
    let client = reqs
        .iter()
        .find(|r| r.kind == ResourceKind::ClientJar)
        .unwrap();
    assert!(client.sha1.is_some());
    assert!(reqs.iter().any(|r| r.kind == ResourceKind::Asset
        && r.urls[0].as_str()
            == "https://resources.download.minecraft.net/bd/bdf48ef6b5d0d23bbb02e17d04865216179f510a"));

    // get / update
    assert_eq!(core.get(inst.id).await.unwrap(), inst);
    let upd = core
        .update(
            inst.id,
            InstancePatch {
                name: Some("Renamed".into()),
                jvm_args: Some(vec!["-XX:+UseZGC".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(upd.name, "Renamed");
    assert_eq!(upd.jvm_args, vec!["-XX:+UseZGC".to_string()]);
    let bad = core
        .update(
            inst.id,
            InstancePatch {
                name: Some(" ".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(bad.code, ErrorCode::InvalidInput);

    // clone copies the game dir
    std::fs::create_dir_all(dir.join("minecraft/saves/w")).unwrap();
    std::fs::write(dir.join("minecraft/saves/w/level.dat"), b"lvl").unwrap();
    std::fs::create_dir_all(dir.join("natives")).unwrap();
    std::fs::write(dir.join("natives/x.dll"), b"n").unwrap();
    let copy = core
        .clone_instance(inst.id, "Copy", &RecSink::default())
        .await
        .unwrap();
    assert_ne!(copy.id, inst.id);
    assert_eq!(copy.name, "Copy");
    assert_eq!(copy.playtime_sec, 0);
    let copy_dir = root.join("instances").join(copy.id.to_string());
    assert_eq!(
        std::fs::read(copy_dir.join("minecraft/saves/w/level.dat")).unwrap(),
        b"lvl"
    );
    assert!(!copy_dir.join("natives").exists());

    let list = InstanceService::list(&core).await.unwrap();
    assert_eq!(list.len(), 2);

    // repair re-verifies (vanilla, no loader) — java is needed, give it a fake path.
    core.update(
        copy.id,
        InstancePatch {
            java: Some(JavaCfg {
                path: Some(
                    copy_dir
                        .join("instance.json")
                        .to_string_lossy()
                        .into_owned(),
                ),
                major: None,
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    core.repair(copy.id, &RecSink::default()).await.unwrap();

    core.delete(inst.id).await.unwrap();
    assert!(!dir.exists());
    assert_eq!(
        core.get(inst.id).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        core.delete(inst.id).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(InstanceService::list(&core).await.unwrap().len(), 1);

    let rec = core.recommend_memory(copy.id).await.unwrap();
    assert!(rec.max_mb >= rec.min_mb);
}

#[tokio::test]
async fn failed_create_leaves_nothing() {
    let w = world();
    let core = services(&w, Arc::default());
    let err = core
        .create(spec("x", "9.9.9"), &RecSink::default())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(InstanceService::list(&core).await.unwrap().is_empty());

    let mut s = spec("fabric", "1.21.1");
    s.loader.kind = LoaderKind::Fabric;
    let err = core.create(s, &RecSink::default()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotImplemented);
    assert!(InstanceService::list(&core).await.unwrap().is_empty());
}

#[tokio::test]
async fn legacy_virtual_assets_are_copied() {
    let w = world();
    let core = services(&w, Arc::default());
    core.create(spec("alpha", "a1.0.4"), &RecSink::default())
        .await
        .unwrap();
    let named = w.root.join("assets/virtual/legacy/sound/step/grass1.ogg");
    assert_eq!(std::fs::read(named).unwrap(), b"stub");
}

#[tokio::test]
async fn launch_without_java_reports_and_resets_state() {
    let w = world();
    let obs = Arc::new(Obs::default());
    let core = services(&w, obs.clone());
    let inst = core
        .create(spec("old", "1.12.2"), &RecSink::default())
        .await
        .unwrap();
    // 1.12.2 natives (old-style classifiers) were downloaded.
    assert!(w
        .root
        .join("libraries/org/lwjgl/lwjgl/lwjgl-platform/2.9.4-nightly-20150209")
        .read_dir()
        .unwrap()
        .any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(if cfg!(windows) {
                "natives-windows"
            } else {
                "natives-linux"
            })));

    core.update(
        inst.id,
        InstancePatch {
            java: Some(JavaCfg {
                path: Some(w.root.join("no-java").to_string_lossy().into_owned()),
                major: None,
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let err = core
        .launch(inst.id, LaunchOpts::default(), &RecSink::default())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::JavaMissing);
    assert_eq!(core.state(inst.id), RunState::Stopped);
    assert_eq!(
        *obs.states.lock().unwrap(),
        vec![RunState::Preparing, RunState::Stopped]
    );
    assert_eq!(
        core.kill(inst.id).await.unwrap_err().code,
        ErrorCode::NotFound
    );
}
