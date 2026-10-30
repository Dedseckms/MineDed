//! Service-level tests with fake catalogues, instances and downloader (no network).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lc_core::{InstanceService, SystemProbe};
use lc_http::{DownloadReq, Downloader, ResourceKind, Route};
use lc_runtime::{NullSink, Paths, ProgressSink};
use lc_types::content::{ContentKind, ContentSourceKind, ContentVersionRef, SearchQuery, SortBy};
use lc_types::instance::{
    Instance, InstancePatch, InstanceSpec, JavaCfg, LoaderRef, MemoryCfg, PackFormat, PerfPreset,
};
use lc_types::settings::ProxyCfg;
use lc_types::system::{MemoryRecommendation, SystemInfo};
use lc_types::version::LoaderKind;
use lc_types::{AppError, AppResult, ErrorCode, InstanceId};
use sha1::Digest;
use url::Url;

use crate::api::fake::FakeHttp;
use crate::contract::{ContentService, PackIo, PresetService};
use crate::service::ContentServices;
use crate::source::Target;
use crate::util::sha1_hex;

const MR: &str = "https://api.modrinth.com/v2";

#[derive(Default)]
struct Dl {
    files: Mutex<HashMap<String, Vec<u8>>>,
}

#[async_trait]
impl Downloader for Dl {
    async fn fetch_many(&self, reqs: Vec<DownloadReq>, _s: &dyn ProgressSink) -> AppResult<()> {
        for r in reqs {
            let url = r.urls[0].to_string();
            let bytes = self
                .files
                .lock()
                .unwrap()
                .get(&url)
                .cloned()
                .ok_or_else(|| AppError::new(ErrorCode::NotFound, "404").detail("url", &url))?;
            if let Some(w) = &r.sha1 {
                if &sha1_hex(&bytes) != w {
                    return Err(AppError::new(ErrorCode::HashMismatch, "sha1"));
                }
            }
            if r.size.is_some_and(|size| size != bytes.len() as u64) {
                return Err(AppError::new(ErrorCode::HashMismatch, "size"));
            }
            std::fs::create_dir_all(r.dest.parent().unwrap()).unwrap();
            std::fs::write(&r.dest, bytes).unwrap();
        }
        Ok(())
    }
    async fn get_bytes(&self, url: &Url, _k: ResourceKind) -> AppResult<Vec<u8>> {
        self.files
            .lock()
            .unwrap()
            .get(url.as_str())
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "404").detail("url", url.as_str()))
    }
    fn client(&self, route: Route) -> reqwest::Client {
        lc_http::StubDownloader.client(route)
    }
    fn mirrors_for(&self, original: &Url, _k: ResourceKind) -> Vec<Url> {
        vec![original.clone()]
    }
    fn set_manual_proxy(&self, _p: Option<ProxyCfg>) {}
}

struct Instances {
    paths: Paths,
    map: Mutex<HashMap<InstanceId, Instance>>,
}

fn instance(mc: &str, kind: LoaderKind) -> Instance {
    Instance {
        id: InstanceId::new(),
        name: "t".into(),
        mc: mc.into(),
        loader: LoaderRef {
            kind,
            version: (kind != LoaderKind::Vanilla).then(|| "1.0".to_owned()),
        },
        java: JavaCfg::default(),
        memory: MemoryCfg {
            min_mb: 512,
            max_mb: 2048,
            auto: true,
        },
        jvm_args: vec!["-Duser=1".into()],
        preset: None,
        icon: None,
        created_at: String::new(),
        last_played: None,
        playtime_sec: 0,
    }
}

#[async_trait]
impl InstanceService for Instances {
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
    async fn create(&self, spec: InstanceSpec, _s: &dyn ProgressSink) -> AppResult<Instance> {
        let mut i = instance(&spec.mc, spec.loader.kind);
        i.loader = spec.loader;
        i.name = spec.name;
        self.map.lock().unwrap().insert(i.id, i.clone());
        Ok(i)
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
        if let Some(j) = patch.java {
            i.java = j;
        }
        if let Some(icon) = patch.icon {
            i.icon = Some(icon);
        }
        if let Some(p) = patch.preset {
            i.preset = Some(p);
        }
        Ok(i.clone())
    }
    async fn delete(&self, id: InstanceId) -> AppResult<()> {
        self.map.lock().unwrap().remove(&id);
        Ok(())
    }
    async fn repair(&self, _id: InstanceId, _s: &dyn ProgressSink) -> AppResult<()> {
        Ok(())
    }
    fn game_dir(&self, id: InstanceId) -> PathBuf {
        self.paths.instance(id).join("game")
    }
}

struct Sys;

#[async_trait]
impl SystemProbe for Sys {
    async fn info(&self) -> AppResult<SystemInfo> {
        Ok(SystemInfo {
            os: "windows".into(),
            os_version: "11".into(),
            arch: "x86_64".into(),
            cpu_model: "cpu".into(),
            cpu_threads: 4,
            total_ram_mb: 8192,
            free_ram_mb: 4096,
            gpu: None,
            weak_hardware: true,
            wayland: false,
        })
    }
    async fn recommend_memory(&self, _id: InstanceId) -> AppResult<MemoryRecommendation> {
        Err(AppError::not_implemented("x"))
    }
}

struct Env {
    _dir: tempfile::TempDir,
    http: Arc<FakeHttp>,
    dl: Arc<Dl>,
    inst: Arc<Instances>,
    svc: ContentServices,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    let http = Arc::new(FakeHttp::default());
    let dl = Arc::new(Dl::default());
    let inst = Arc::new(Instances {
        paths: paths.clone(),
        map: Mutex::new(HashMap::new()),
    });
    let svc =
        ContentServices::with_parts(paths, dl.clone(), inst.clone(), Arc::new(Sys), http.clone());
    Env {
        _dir: dir,
        http,
        dl,
        inst,
        svc,
    }
}

impl Env {
    fn add_instance(&self, mc: &str, kind: LoaderKind) -> InstanceId {
        let i = instance(mc, kind);
        let id = i.id;
        self.inst.map.lock().unwrap().insert(id, i);
        id
    }

    fn project(&self, id: &str, slug: &str, ptype: &str) {
        let body = serde_json::json!({"id": id, "slug": slug, "title": slug, "project_type": ptype, "loaders": []})
            .to_string();
        self.http.route(&format!("{MR}/project/{id}"), body.clone());
        self.http.route(&format!("{MR}/project/{slug}"), body);
    }

    /// Registers versions (newest first) of a project and their files.
    fn versions(&self, id: &str, slug: &str, versions: &[serde_json::Value]) {
        let body = serde_json::Value::Array(versions.to_vec()).to_string();
        self.http
            .route(&format!("{MR}/project/{id}/version"), body.clone());
        self.http
            .route(&format!("{MR}/project/{slug}/version"), body);
        for v in versions {
            self.http.route(
                &format!("{MR}/version/{}", v["id"].as_str().unwrap()),
                v.to_string(),
            );
        }
    }

    fn mr_version(
        &self,
        project: &str,
        vid: &str,
        file: &str,
        loaders: &[&str],
        deps: &[&str],
    ) -> serde_json::Value {
        let bytes = format!("{project}:{vid}").into_bytes();
        let url = format!("https://cdn.modrinth.com/data/{project}/versions/{vid}/{file}");
        let sha512 = hex::encode(sha2::Sha512::digest(&bytes));
        let v = serde_json::json!({
            "id": vid, "project_id": project, "name": vid, "version_number": vid,
            "game_versions": ["1.21.1", "1.20.1"], "loaders": loaders,
            "date_published": "2025-01-01T00:00:00Z", "version_type": "release",
            "files": [{"hashes": {"sha1": sha1_hex(&bytes), "sha512": sha512},
                "url": url, "filename": file, "primary": true, "size": bytes.len()}],
            "dependencies": deps.iter().map(|d| serde_json::json!({"project_id": d, "dependency_type": "required"})).collect::<Vec<_>>(),
        });
        self.dl.files.lock().unwrap().insert(url, bytes);
        v
    }
}

fn mr_pick(p: &str) -> ContentVersionRef {
    ContentVersionRef {
        source: ContentSourceKind::Modrinth,
        project_id: p.into(),
        version_id: None,
    }
}

#[tokio::test]
async fn shader_pulls_iris_and_full_lifecycle() {
    let e = env();
    let id = e.add_instance("1.21.1", LoaderKind::Fabric);
    e.project("SHADERxx", "complementary", "shader");
    e.project("YL57xq9U", "iris", "mod");
    e.project("AANobbMI", "sodium", "mod");
    let shader = e.mr_version("SHADERxx", "sh1", "complementary.zip", &["iris"], &[]);
    let iris = e.mr_version("YL57xq9U", "ir1", "iris.jar", &["fabric"], &["AANobbMI"]);
    let sodium1 = e.mr_version("AANobbMI", "so1", "sodium-1.jar", &["fabric"], &[]);
    e.versions("SHADERxx", "complementary", &[shader]);
    e.versions("YL57xq9U", "iris", &[iris]);
    e.versions("AANobbMI", "sodium", std::slice::from_ref(&sodium1));

    let plan = e.svc.resolve(id, vec![mr_pick("SHADERxx")]).await.unwrap();
    let mut ids: Vec<_> = plan.install.iter().map(|v| v.project_id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["AANobbMI", "SHADERxx", "YL57xq9U"]);
    assert!(plan.manual.is_empty());

    let sink = NullSink::default();
    e.svc.install(id, plan, &sink).await.unwrap();
    let game = e.inst.game_dir(id);
    assert!(game.join("shaderpacks/complementary.zip").exists());
    assert!(game.join("mods/iris.jar").exists());
    let list = e.svc.list(id).await.unwrap();
    assert_eq!(list.len(), 3);
    assert!(list.iter().all(|c| c.source == ContentSourceKind::Modrinth));

    // Second shader install: Iris is already there.
    let plan = e.svc.resolve(id, vec![mr_pick("SHADERxx")]).await.unwrap();
    assert!(plan.install.is_empty());

    // Disable, then update keeps the disabled state.
    e.svc.set_enabled(id, "sodium-1.jar", false).await.unwrap();
    assert!(game.join("mods/sodium-1.jar.disabled").exists());
    let sodium2 = e.mr_version("AANobbMI", "so2", "sodium-2.jar", &["fabric"], &[]);
    e.http
        .route(&format!("{MR}/version/so2"), sodium2.to_string());
    let mut upd = serde_json::Map::new();
    upd.insert(
        sodium1["files"][0]["hashes"]["sha1"]
            .as_str()
            .unwrap()
            .into(),
        sodium2,
    );
    e.http.route(
        &format!("POST {MR}/version_files/update"),
        serde_json::Value::Object(upd).to_string(),
    );
    let list = e.svc.list(id).await.unwrap();
    assert!(list
        .iter()
        .any(|c| c.file_name == "sodium-1.jar" && c.update_available && !c.enabled));
    e.svc.update_all(id, &sink).await.unwrap();
    assert!(!game.join("mods/sodium-1.jar.disabled").exists());
    assert!(game.join("mods/sodium-2.jar.disabled").exists());
    e.svc.set_enabled(id, "sodium-2.jar", true).await.unwrap();
    assert!(game.join("mods/sodium-2.jar").exists());

    // Remove.
    e.svc.remove(id, "iris.jar").await.unwrap();
    assert!(!game.join("mods/iris.jar").exists());
    assert!(e.svc.remove(id, "../x.jar").await.is_err());
    assert_eq!(e.svc.list(id).await.unwrap().len(), 2);

    // Export as .mrpack and import into a new instance.
    std::fs::create_dir_all(game.join("config")).unwrap();
    std::fs::write(game.join("config/a.toml"), "x=1").unwrap();
    std::fs::write(game.join("mods/local.jar"), "local").unwrap();
    let out = e._dir.path().join("out.mrpack");
    e.svc
        .export(id, PackFormat::Mrpack, out.to_str().unwrap(), &sink)
        .await
        .unwrap();
    let imported = e.svc.import(out.to_str().unwrap(), &sink).await.unwrap();
    assert_eq!(imported.loader.kind, LoaderKind::Fabric);
    let g2 = e.inst.game_dir(imported.id);
    assert!(g2.join("mods/sodium-2.jar").exists());
    assert!(g2.join("mods/local.jar").exists());
    assert!(g2.join("config/a.toml").exists());
    let l2 = e.svc.list(imported.id).await.unwrap();
    let sodium = l2.iter().find(|c| c.file_name == "sodium-2.jar").unwrap();
    assert_eq!(sodium.source, ContentSourceKind::Modrinth);
    assert_eq!(sodium.version_id.as_deref(), Some("so2"));
    let local = l2.iter().find(|c| c.file_name == "local.jar").unwrap();
    assert_eq!(local.source, ContentSourceKind::Local);

    // Launcher zip round-trip keeps JVM args.
    let out = e._dir.path().join("out.zip");
    e.svc
        .export(id, PackFormat::LauncherZip, out.to_str().unwrap(), &sink)
        .await
        .unwrap();
    let back = e.svc.import(out.to_str().unwrap(), &sink).await.unwrap();
    assert_eq!(back.jvm_args, vec!["-Duser=1".to_owned()]);
    assert!(e.inst.game_dir(back.id).join("config/a.toml").exists());
}

#[tokio::test]
async fn shader_loader_choice_per_loader() {
    let e = env();
    e.project("GchcoXML", "oculus", "mod");
    e.project("sk9rgfiA", "embeddium", "mod");
    let oc = e.mr_version("GchcoXML", "oc1", "oculus.jar", &["forge"], &[]);
    let em = e.mr_version("sk9rgfiA", "em1", "embeddium.jar", &["forge"], &[]);
    e.versions("GchcoXML", "oculus", &[oc]);
    e.versions("sk9rgfiA", "embeddium", &[em]);
    let forge = Target {
        mc: "1.20.1".into(),
        loader: LoaderKind::Forge,
    };
    let (extra, manual) = e.svc.inner.shader_loader(&forge).await;
    let ids: Vec<_> = extra.iter().map(|r| r.project_id.as_str()).collect();
    assert_eq!(ids, vec!["GchcoXML", "sk9rgfiA"]);
    assert!(manual.is_empty());

    // NeoForge: Iris has no NeoForge build here → falls back to Oculus.
    e.project("YL57xq9U", "iris", "mod");
    let iris = e.mr_version("YL57xq9U", "ir1", "iris.jar", &["fabric"], &[]);
    e.versions("YL57xq9U", "iris", &[iris]);
    let oc_neo = e.mr_version("GchcoXML", "oc2", "oculus-neo.jar", &["neoforge"], &[]);
    e.versions("GchcoXML", "oculus", &[oc_neo]);
    let neo = Target {
        mc: "1.21.1".into(),
        loader: LoaderKind::NeoForge,
    };
    let (extra, _) = e.svc.inner.shader_loader(&neo).await;
    assert_eq!(extra[0].version_id.as_deref(), Some("oc2"));

    // Vanilla / no loader build: OptiFine is a manual link only.
    let (extra, manual) = e
        .svc
        .inner
        .shader_loader(&Target {
            mc: "1.21.1".into(),
            loader: LoaderKind::Vanilla,
        })
        .await;
    assert!(extra.is_empty());
    assert_eq!(manual[0].url, crate::shaders::OPTIFINE_URL);
    let (_, manual) = e
        .svc
        .inner
        .shader_loader(&Target {
            mc: "1.12.2".into(),
            loader: LoaderKind::Forge,
        })
        .await;
    assert_eq!(manual[0].project_id, "optifine");
}

#[tokio::test]
async fn modrinth_search_still_works() {
    let e = env();
    let q = SearchQuery {
        source: ContentSourceKind::Modrinth,
        kind: ContentKind::Mod,
        text: "test".into(),
        mc: None,
        loader: None,
        categories: vec![],
        sort: SortBy::Relevance,
        offset: 0,
        limit: 10,
    };
    e.http.route(
        &format!("{MR}/search"),
        r#"{"hits":[{"project_id":"P","slug":"p","title":"P","project_type":"mod"}],"offset":0,"limit":10,"total_hits":1}"#,
    );
    let page = e
        .svc
        .search(SearchQuery {
            source: ContentSourceKind::Modrinth,
            kind: ContentKind::Mod,
            ..q
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.total, 1);
}

#[tokio::test]
async fn preset_installs_mods_and_flags() {
    let e = env();
    let id = e.add_instance("1.21.1", LoaderKind::Fabric);
    e.project("AANobbMI", "sodium", "mod");
    let so = e.mr_version("AANobbMI", "so1", "sodium.jar", &["fabric"], &[]);
    e.versions("AANobbMI", "sodium", &[so]);
    e.project("gvQqBUqZ", "lithium", "mod");
    let li = e.mr_version("gvQqBUqZ", "li1", "lithium.jar", &["fabric"], &[]);
    e.versions("gvQqBUqZ", "lithium", &[li]);
    // Other preset mods have no fixtures → skipped with a log line.
    e.svc
        .apply(id, PerfPreset::Potato, &NullSink::default())
        .await
        .unwrap();
    let game = e.inst.game_dir(id);
    assert!(game.join("mods/sodium.jar").exists());
    assert!(game.join("mods/lithium.jar").exists());
    let inst = e.inst.get(id).await.unwrap();
    assert!(inst.jvm_args.contains(&"-Duser=1".to_owned()));
    assert!(inst.jvm_args.contains(&"-XX:+UseG1GC".to_owned()));
    assert!(!inst.memory.auto);
    assert_eq!(inst.memory.max_mb, 2048);
    assert_eq!(inst.preset, Some(PerfPreset::Potato));
    let opts = std::fs::read_to_string(game.join("options.txt")).unwrap();
    assert!(opts.contains("renderDistance:6"));
}

#[tokio::test]
async fn preset_on_vanilla_writes_options() {
    let e = env();
    let id = e.add_instance("1.12.2", LoaderKind::Vanilla);
    let game = e.inst.game_dir(id);
    std::fs::create_dir_all(&game).unwrap();
    std::fs::write(game.join("options.txt"), "lang:en_us\nrenderDistance:12\n").unwrap();
    e.svc
        .apply(id, PerfPreset::Balanced, &NullSink::default())
        .await
        .unwrap();
    let opts = std::fs::read_to_string(game.join("options.txt")).unwrap();
    assert!(opts.starts_with("lang:en_us\nrenderDistance:10\n"));
    assert!(opts.contains("fancyGraphics:false"));
    assert!(!opts.contains("simulationDistance"));
    let inst = e.inst.get(id).await.unwrap();
    assert_eq!(inst.preset, Some(PerfPreset::Balanced));
    assert!(inst.jvm_args.contains(&"-XX:+UseG1GC".to_owned()));
}

#[tokio::test]
async fn preset_without_any_mod_fails_visibly() {
    let e = env();
    let id = e.add_instance("1.21.1", LoaderKind::Fabric);
    // No fixtures at all → every slug unavailable.
    let err = e
        .svc
        .apply(id, PerfPreset::Potato, &NullSink::default())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(err.details.unwrap().contains_key("reason"));
    let inst = e.inst.get(id).await.unwrap();
    // Settings still applied, but the preset is not marked as applied.
    assert_eq!(inst.preset, None);
    assert!(inst.jvm_args.contains(&"-XX:+UseG1GC".to_owned()));
    assert!(e.inst.game_dir(id).join("options.txt").exists());
}

// ---------------------------------------------------------------------------
// Modpacks from catalogues
// ---------------------------------------------------------------------------

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nicon-bytes";
const PACK: &str = "PACKfo01";
const ICON_URL: &str = "https://cdn.modrinth.com/data/PACKfo01/icon.png";

fn zip_bytes(dir: &std::path::Path, entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let out = dir.join(format!("{}.zip", InstanceId::new()));
    let entries: Vec<(String, Vec<u8>)> = entries
        .iter()
        .map(|(n, b)| ((*n).to_owned(), b.clone()))
        .collect();
    crate::packs::write_zip(&out, &entries, &[]).unwrap();
    std::fs::read(out).unwrap()
}

#[tokio::test]
async fn mrpack_rejects_download_quota_before_creating_an_instance() {
    let e = env();
    let file = |name: &str, size: u64| {
        serde_json::json!({
            "path": name, "fileSize": size,
            "hashes": {"sha1": "00".repeat(20), "sha512": "00".repeat(64)},
            "downloads": ["https://cdn.modrinth.com/any"]
        })
    };
    let files = vec![
        vec![
            file("mods/a.jar", 2 * 1024 * 1024 * 1024 + 1),
            file("mods/b.jar", 2 * 1024 * 1024 * 1024),
        ],
        vec![file("mods/a.jar", 0); 20_001],
        vec![file("mods/a.jar", u64::MAX)],
    ];
    for files in files {
        let manifest = serde_json::json!({
            "formatVersion": 1, "game": "minecraft", "versionId": "1", "name": "quota",
            "files": files, "dependencies": {"minecraft": "1.21.1"}
        });
        let bytes = zip_bytes(
            e._dir.path(),
            &[("modrinth.index.json", manifest.to_string().into_bytes())],
        );
        let path = e._dir.path().join("quota.mrpack");
        std::fs::write(&path, bytes).unwrap();
        let err = e
            .svc
            .import(&path.to_string_lossy(), &NullSink::default())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(e.inst.list().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn mrpack_zero_size_cannot_disable_download_size_validation() {
    let e = env();
    let url = "https://cdn.modrinth.com/data/empty";
    let payload = b"unexpected nonempty payload";
    e.dl.files
        .lock()
        .unwrap()
        .insert(url.into(), payload.to_vec());
    let manifest = serde_json::json!({
        "formatVersion": 1, "game": "minecraft", "versionId": "1", "name": "zero",
        "files": [{"path": "config/empty.txt", "fileSize": 0,
            "hashes": {"sha1": sha1_hex(payload), "sha512": sha512_hex(payload)}, "downloads": [url]}],
        "dependencies": {"minecraft": "1.21.1"}
    });
    let bytes = zip_bytes(
        e._dir.path(),
        &[("modrinth.index.json", manifest.to_string().into_bytes())],
    );
    let path = e._dir.path().join("zero.mrpack");
    std::fs::write(&path, bytes).unwrap();
    let err = e
        .svc
        .import(&path.to_string_lossy(), &NullSink::default())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::HashMismatch);
    assert!(e.inst.list().await.unwrap().is_empty());
}

fn sha512_hex(b: &[u8]) -> String {
    hex::encode(sha2::Sha512::digest(b))
}

/// A small sink that can be cancelled up front.
fn cancelled_sink() -> NullSink {
    let s = NullSink::default();
    s.cancel_token().cancel();
    s
}

impl Env {
    /// Modrinth modpack `PACK` with versions `pv3` (beta) and `pv2` (featured
    /// release, the pick for "latest"); `pv2` contains Sodium + overrides.
    /// Returns the `.mrpack` URL of `pv2`.
    fn mr_modpack(&self, bad_sha512: bool) -> String {
        let project = serde_json::json!({"id": PACK, "slug": "tiny", "title": "Tiny Pack",
            "project_type": "modpack", "loaders": ["fabric"], "icon_url": ICON_URL});
        self.http
            .route(&format!("{MR}/project/{PACK}"), project.to_string());
        let sodium = self.mr_version("AANobbMI", "so1", "sodium.jar", &["fabric"], &[]);
        let file = &sodium["files"][0];
        let index = serde_json::json!({
            "formatVersion": 1, "game": "minecraft", "versionId": "1.0", "name": "inner name",
            "files": [
                {"path": "mods/sodium.jar", "hashes": file["hashes"], "downloads": [file["url"]],
                 "fileSize": file["size"]},
                {"path": "mods/server-only.jar", "hashes": {"sha1": "x", "sha512": "y"},
                 "downloads": ["https://cdn.modrinth.com/x.jar"], "env": {"client": "unsupported"}}
            ],
            "dependencies": {"minecraft": "1.21.1", "fabric-loader": "0.16.9"}
        });
        let bytes = zip_bytes(
            self._dir.path(),
            &[
                ("modrinth.index.json", index.to_string().into_bytes()),
                ("overrides/config/a.toml", b"x=1".to_vec()),
                ("overrides/mods/extra.jar", b"extra".to_vec()),
                ("client-overrides/options.txt", b"gamma:1".to_vec()),
            ],
        );
        let url = format!("https://cdn.modrinth.com/data/{PACK}/versions/pv2/tiny-1.0.mrpack");
        let sha512 = if bad_sha512 {
            "00".repeat(64)
        } else {
            sha512_hex(&bytes)
        };
        let pv2 = serde_json::json!({
            "id": "pv2", "project_id": PACK, "name": "1.0", "version_number": "1.0",
            "game_versions": ["1.21.1"], "loaders": ["fabric"], "featured": true,
            "date_published": "2025-01-01T00:00:00Z", "version_type": "release",
            "files": [{"hashes": {"sha1": sha1_hex(&bytes), "sha512": sha512}, "url": url,
                "filename": "tiny-1.0.mrpack", "primary": true, "size": bytes.len()}],
            "dependencies": []
        });
        let pv3 = serde_json::json!({
            "id": "pv3", "project_id": PACK, "name": "1.1-beta", "version_number": "1.1-beta",
            "game_versions": ["1.21.1"], "loaders": ["fabric"], "featured": false,
            "date_published": "2025-02-01T00:00:00Z", "version_type": "beta",
            "files": [{"hashes": {"sha1": "00", "sha512": "00"},
                "url": "https://cdn.modrinth.com/data/PACKfo01/versions/pv3/b.mrpack",
                "filename": "b.mrpack", "primary": true, "size": 1}],
            "dependencies": []
        });
        self.http.route(
            &format!("{MR}/project/{PACK}/version"),
            serde_json::json!([pv3, pv2]).to_string(),
        );
        self.http
            .route(&format!("{MR}/version/pv2"), pv2.to_string());
        let mut files = self.dl.files.lock().unwrap();
        files.insert(url.clone(), bytes);
        files.insert(ICON_URL.into(), PNG.to_vec());
        url
    }

    fn staging_is_empty(&self) -> bool {
        let dir = self._dir.path().join("cache/content/staging");
        std::fs::read_dir(dir).map_or(true, |mut d| d.next().is_none())
    }
}

#[tokio::test]
async fn modrinth_modpack_installs_from_catalog() {
    let e = env();
    e.mr_modpack(false);

    // Search and versions for modpacks.
    e.http.route(
        &format!("{MR}/search"),
        r#"{"hits":[{"project_id":"PACKfo01","slug":"tiny","title":"Tiny Pack","project_type":"modpack"}],"offset":0,"total_hits":1}"#,
    );
    let page = e
        .svc
        .search(SearchQuery {
            source: ContentSourceKind::Modrinth,
            kind: ContentKind::Modpack,
            text: String::new(),
            mc: None,
            loader: None,
            categories: vec![],
            sort: SortBy::Downloads,
            offset: 0,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(page.items[0].kind, ContentKind::Modpack);
    let versions = e
        .svc
        .versions(
            ContentSourceKind::Modrinth,
            PACK,
            lc_types::content::ContentFilter {
                mc: None,
                loader: Some(LoaderKind::Fabric),
            },
        )
        .await
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[1].mc_versions, vec!["1.21.1"]);
    assert_eq!(versions[1].loaders, vec![LoaderKind::Fabric]);

    let sink = NullSink::default();
    let inst = e
        .svc
        .install_remote(ContentSourceKind::Modrinth, PACK, None, &sink)
        .await
        .unwrap();
    assert_eq!(inst.name, "Tiny Pack");
    assert_eq!(inst.mc, "1.21.1");
    assert_eq!(inst.loader.kind, LoaderKind::Fabric);
    assert_eq!(inst.loader.version.as_deref(), Some("0.16.9"));
    assert_eq!(inst.icon.as_deref(), Some("icon.png"));
    let inst_dir = e._dir.path().join("instances").join(inst.id.to_string());
    assert_eq!(std::fs::read(inst_dir.join("icon.png")).unwrap(), PNG);
    let game = e.inst.game_dir(inst.id);
    assert!(game.join("mods/sodium.jar").exists());
    assert!(!game.join("mods/server-only.jar").exists());
    assert_eq!(
        std::fs::read(game.join("mods/extra.jar")).unwrap(),
        b"extra"
    );
    assert_eq!(std::fs::read(game.join("config/a.toml")).unwrap(), b"x=1");
    assert!(game.join("options.txt").exists());
    let list = e.svc.list(inst.id).await.unwrap();
    let so = list.iter().find(|c| c.file_name == "sodium.jar").unwrap();
    assert_eq!(so.source, ContentSourceKind::Modrinth);
    assert_eq!(so.version_id.as_deref(), Some("so1"));
    assert!(e.staging_is_empty());
    // The downloaded pack is not kept after a successful install.
    let packs = e._dir.path().join("cache/content/packs");
    assert!(std::fs::read_dir(packs).map_or(true, |mut d| d.next().is_none()));

    // Explicit version of another project is rejected.
    e.http.route(
        &format!("{MR}/version/other"),
        serde_json::json!({"id": "other", "project_id": "ZZZ", "files": []}).to_string(),
    );
    let err = e
        .svc
        .install_remote(ContentSourceKind::Modrinth, PACK, Some("other"), &sink)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    // Not a modpack.
    e.project("AANobbMI", "sodium", "mod");
    let err = e
        .svc
        .install_remote(ContentSourceKind::Modrinth, "AANobbMI", None, &sink)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn modpack_failures_leave_no_instance() {
    // Bad sha512 of the pack itself.
    let e = env();
    e.mr_modpack(true);
    let sink = NullSink::default();
    let err = e
        .svc
        .install_remote(ContentSourceKind::Modrinth, PACK, Some("pv2"), &sink)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::HashMismatch);
    assert!(e.inst.map.lock().unwrap().is_empty());

    // Cancelled before anything is created.
    let e = env();
    e.mr_modpack(false);
    let err = e
        .svc
        .install_remote(ContentSourceKind::Modrinth, PACK, None, &cancelled_sink())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert!(e.inst.map.lock().unwrap().is_empty());

    // A pack mod that fails to download: no instance, staging cleaned.
    let e = env();
    let url = e.mr_modpack(false);
    e.dl.files
        .lock()
        .unwrap()
        .retain(|k, _| *k == url || k == ICON_URL);
    let err = e
        .svc
        .install_remote(ContentSourceKind::Modrinth, PACK, None, &sink)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert!(e.inst.map.lock().unwrap().is_empty());
    assert!(e.staging_is_empty());
    // The verified pack stays cached for a retry.
    assert!(e._dir.path().join("cache/content/packs").exists());
}

#[tokio::test]
async fn add_local_copies_replaces_and_indexes() {
    let e = env();
    let id = e.add_instance("1.21.1", LoaderKind::Fabric);
    let src = e._dir.path().join("picked");
    std::fs::create_dir_all(src.join("MyPack/assets")).unwrap();
    std::fs::write(src.join("MyPack/pack.mcmeta"), "{}").unwrap();
    std::fs::write(src.join("MyPack/assets/a.png"), "png").unwrap();
    std::fs::write(src.join("cool.jar"), "v1").unwrap();
    std::fs::write(src.join("bad.zip"), "z").unwrap();
    let s = |p: PathBuf| p.to_string_lossy().into_owned();
    let game = e.inst.game_dir(id);

    // Wrong type for the kind: nothing copied.
    let err = e
        .svc
        .add_local(
            id,
            ContentKind::Mod,
            vec![s(src.join("cool.jar")), s(src.join("bad.zip"))],
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(err.details.unwrap()["reason"], "wrongFileType");
    assert!(!game.join("mods/cool.jar").exists());
    assert!(e
        .svc
        .add_local(id, ContentKind::Modpack, vec![s(src.join("bad.zip"))])
        .await
        .is_err());

    let added = e
        .svc
        .add_local(id, ContentKind::Mod, vec![s(src.join("cool.jar"))])
        .await
        .unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].source, ContentSourceKind::Local);
    assert_eq!(std::fs::read(game.join("mods/cool.jar")).unwrap(), b"v1");

    // Same name replaces the file, including a disabled copy.
    e.svc.set_enabled(id, "cool.jar", false).await.unwrap();
    std::fs::write(src.join("cool.jar"), "v2").unwrap();
    e.svc
        .add_local(id, ContentKind::Mod, vec![s(src.join("cool.jar"))])
        .await
        .unwrap();
    assert_eq!(std::fs::read(game.join("mods/cool.jar")).unwrap(), b"v2");
    assert!(!game.join("mods/cool.jar.disabled").exists());

    // Folder resource pack.
    let added = e
        .svc
        .add_local(id, ContentKind::ResourcePack, vec![s(src.join("MyPack"))])
        .await
        .unwrap();
    assert_eq!(added[0].file_name, "MyPack");
    assert!(game.join("resourcepacks/MyPack/assets/a.png").exists());
    assert!(src.join("MyPack/pack.mcmeta").exists());

    let list = e.svc.list(id).await.unwrap();
    assert_eq!(list.len(), 2);
    assert!(list
        .iter()
        .all(|c| c.source == ContentSourceKind::Local && c.project_id.is_none()));
    let idx = crate::index::ContentIndex::load(&e.inst.paths.instance(id))
        .await
        .unwrap();
    let jar = &idx.files["mods/cool.jar"];
    assert_eq!(jar.source, ContentSourceKind::Local);
    assert_eq!(jar.sha1.as_deref(), Some(sha1_hex(b"v2").as_str()));
    assert!(idx.files.contains_key("resourcepacks/MyPack"));

    // Folder packs can be disabled and removed.
    e.svc.set_enabled(id, "MyPack", false).await.unwrap();
    assert!(game.join("resourcepacks/MyPack.disabled").is_dir());
    e.svc.remove(id, "MyPack").await.unwrap();
    assert!(!game.join("resourcepacks/MyPack.disabled").exists());
    assert_eq!(e.svc.list(id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn check_local_reports_and_suggests() {
    use crate::localdeps::tests::jar;
    use lc_types::content::LocalDepIssueKind;

    let env = env();
    let id = env.add_instance("1.21.1", LoaderKind::Fabric);
    let mods = env.inst.game_dir(id).join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    let fmj = |v: serde_json::Value| vec![("fabric.mod.json", v.to_string().into_bytes())];
    std::fs::write(
        mods.join("a.jar"),
        jar(&fmj(
            serde_json::json!({"id": "a", "version": "1.0.0", "depends": {
                "fabric-api": ">=0.90", "cloth_config_api": "*", "lib": "*", "minecraft": "1.21.1"
            }}),
        )),
    )
    .unwrap();
    std::fs::write(
        mods.join("lib.jar.disabled"),
        jar(&fmj(serde_json::json!({"id": "lib", "version": "1"}))),
    )
    .unwrap();

    env.project("FAPI0001", "fabric-api", "mod");
    let v1 = env.mr_version("FAPI0001", "0.92.0", "fabric-api.jar", &["fabric"], &[]);
    env.versions("FAPI0001", "fabric-api", &[v1]);
    env.project("CLOTH001", "cloth-config", "mod");
    let v2 = env.mr_version("CLOTH001", "15.0.0", "cloth.jar", &["fabric"], &[]);
    env.versions("CLOTH001", "cloth-config", &[v2]);
    env.http.route(
        &format!("{MR}/search"),
        serde_json::json!({"hits": [{"project_id": "CLOTH001", "slug": "cloth-config",
            "title": "Cloth Config API", "project_type": "mod"}], "offset": 0, "total_hits": 1})
        .to_string(),
    );

    let mut issues = env.svc.check_local(id, Vec::new()).await.unwrap();
    issues.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
    let got: Vec<_> = issues
        .iter()
        .map(|i| {
            (
                i.mod_id.as_str(),
                i.kind,
                i.suggestion.as_ref().and_then(|s| s.version_id.as_deref()),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (
                "cloth_config_api",
                LocalDepIssueKind::Missing,
                Some("15.0.0")
            ),
            ("fabric-api", LocalDepIssueKind::Missing, Some("0.92.0")),
            ("lib", LocalDepIssueKind::Missing, None),
        ]
    );
    assert_eq!(issues[0].suggestion_title.as_deref(), Some("cloth-config"));
    assert_eq!(issues[1].version_req.as_deref(), Some(">=0.90"));

    // Explicit file list: the disabled jar has no deps; unknown names fail.
    let only = env
        .svc
        .check_local(id, vec!["lib.jar".into()])
        .await
        .unwrap();
    assert!(only.is_empty());
    let err = env
        .svc
        .check_local(id, vec!["nope.jar".into()])
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

const CF: &str = "https://relay.dedseckms.ru/api/v1/catalog/curseforge";

impl Env {
    fn cf_project(&self, project: u32, class: u32, allow: bool) -> serde_json::Value {
        let value = serde_json::json!({"id":project,"gameId":432,"classId":class,"slug":format!("p{project}"),"name":format!("Project {project}"),"allowModDistribution":allow});
        self.http.route(
            &format!("{CF}/mods/{project}"),
            serde_json::json!({"data":value}).to_string(),
        );
        value
    }
    fn cf_file(&self, project: u32, file: u32, deps: serde_json::Value) -> serde_json::Value {
        let bytes = format!("file:{file}").into_bytes();
        let url = format!("https://edge.forgecdn.net/files/1/1/{file}.jar");
        self.dl
            .files
            .lock()
            .unwrap()
            .insert(url.clone(), bytes.clone());
        let value = serde_json::json!({"id":file,"modId":project,"gameId":432,"displayName":format!("v{file}"),"fileName":format!("{file}.jar"),"fileDate":"2026-01-01T00:00:00Z","fileLength":bytes.len(),"isAvailable":true,"releaseType":1,"downloadUrl":url,"hashes":[{"algo":1,"value":sha1_hex(&bytes)}],"gameVersions":["1.20.1","Forge"],"dependencies":deps});
        self.cf_register(project, &value);
        value
    }
    fn cf_register(&self, project: u32, value: &serde_json::Value) {
        self.http.route(
            &format!("{CF}/files/{}", value["id"]),
            serde_json::json!({"data":value}).to_string(),
        );
        self.http.route(
            &format!("{CF}/mods/{project}/files"),
            serde_json::json!({"data":[value],"pagination":{"totalCount":1}}).to_string(),
        );
    }
}
fn cf_pick(project: &str) -> ContentVersionRef {
    ContentVersionRef {
        source: ContentSourceKind::CurseForge,
        project_id: project.into(),
        version_id: None,
    }
}
#[tokio::test]
async fn curseforge_search_versions_dependencies_install_and_update() {
    let e = env();
    let p = e.cf_project(1, 6, true);
    e.cf_project(2, 6, true);
    e.cf_project(3, 6, true);
    e.cf_file(
        1,
        11,
        serde_json::json!([{"modId":2,"relationType":3},{"modId":3,"relationType":5}]),
    );
    e.cf_file(2, 22, serde_json::json!([]));
    e.cf_file(3, 33, serde_json::json!([]));
    e.http.route(
        &format!("{CF}/mods/search"),
        serde_json::json!({"data":[p],"pagination":{"index":0,"totalCount":1}}).to_string(),
    );
    let page = e
        .svc
        .search(SearchQuery {
            source: ContentSourceKind::CurseForge,
            kind: ContentKind::Mod,
            text: "jei".into(),
            mc: Some("1.20.1".into()),
            loader: Some(LoaderKind::Forge),
            categories: vec!["423".into()],
            sort: SortBy::Downloads,
            offset: 0,
            limit: 100,
        })
        .await
        .unwrap();
    assert_eq!(page.items[0].project_id, "1");
    let calls = e.http.calls.lock().unwrap().clone();
    assert!(calls[0].contains("pageSize=50"));
    assert!(calls[0].contains("modLoaderType=1"));
    assert!(calls[0].contains("categoryId=423"));
    let id = e.add_instance("1.20.1", LoaderKind::Forge);
    let plan = e.svc.resolve(id, vec![cf_pick("1")]).await.unwrap();
    assert_eq!(plan.install.len(), 2);
    assert!(plan.conflicts.is_empty());
    let conflict = e
        .svc
        .resolve(id, vec![cf_pick("1"), cf_pick("3")])
        .await
        .unwrap();
    assert!(!conflict.conflicts.is_empty());
    e.svc.install(id, plan, &NullSink::default()).await.unwrap();
    assert!(e.inst.game_dir(id).join("mods/11.jar").exists());
    e.svc.set_enabled(id, "11.jar", false).await.unwrap();
    e.cf_file(1, 12, serde_json::json!([]));
    e.svc.update_all(id, &NullSink::default()).await.unwrap();
    assert!(e.inst.game_dir(id).join("mods/12.jar.disabled").exists());
    let none = e
        .svc
        .versions(
            ContentSourceKind::CurseForge,
            "2",
            lc_types::content::ContentFilter {
                mc: Some("1.19.4".into()),
                loader: Some(LoaderKind::Fabric),
            },
        )
        .await
        .unwrap();
    assert!(none.is_empty());
}
#[tokio::test]
async fn curseforge_distribution_null_hashes_and_untrusted_cdn() {
    let e = env();
    e.cf_project(1, 6, false);
    let mut f = e.cf_file(1, 11, serde_json::json!([]));
    let id = e.add_instance("1.20.1", LoaderKind::Forge);
    let plan = e.svc.resolve(id, vec![cf_pick("1")]).await.unwrap();
    assert_eq!(plan.manual.len(), 1);
    assert!(plan.install.is_empty());
    e.cf_project(1, 6, true);
    f["downloadUrl"] = serde_json::Value::Null;
    e.cf_register(1, &f);
    let cat = &e.svc.inner.curseforge;
    use crate::contract::ContentSource;
    use crate::source::Catalog;
    let v = cat.version("11").await.unwrap();
    assert!(v.manual_download);
    assert!(cat.file(&v).await.unwrap().url.is_none());
    f["downloadUrl"] = serde_json::json!("https://evil.example/file.jar");
    e.cf_register(1, &f);
    let v = cat.version("11").await.unwrap();
    assert_eq!(
        cat.file(&v).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
    f["hashes"] = serde_json::json!([{"algo":2,"value":"00000000000000000000000000000000"}]);
    e.cf_register(1, &f);
    assert!(cat.version("11").await.unwrap().manual_download);
    assert!(cat.version("../11").await.is_err());
    // Distribution permission changes are honored even after successful metadata reads.
    e.cf_project(1, 6, false);
    assert!(cat.file(&v).await.unwrap().url.is_none());
}
fn cf_manifest(overrides: &str) -> serde_json::Value {
    serde_json::json!({"minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.2.0","primary":true}]},"manifestType":"minecraftModpack","manifestVersion":1,"name":"CF Pack","files":[{"projectID":1,"fileID":11,"required":true}],"overrides":overrides})
}
#[tokio::test]
async fn curseforge_pack_import_remote_and_manual() {
    let e = env();
    e.cf_project(1, 6, true);
    e.cf_file(1, 11, serde_json::json!([]));
    let manifest = cf_manifest("overrides");
    let bytes = zip_bytes(
        e._dir.path(),
        &[
            ("manifest.json", serde_json::to_vec(&manifest).unwrap()),
            ("overrides/config/test.txt", b"setting".to_vec()),
        ],
    );
    let path = e._dir.path().join("cf.zip");
    std::fs::write(&path, &bytes).unwrap();
    let inst = e
        .svc
        .import(path.to_str().unwrap(), &NullSink::default())
        .await
        .unwrap();
    assert_eq!(inst.loader.version.as_deref(), Some("47.2.0"));
    assert!(e.inst.game_dir(inst.id).join("mods/11.jar").exists());
    assert!(e.inst.game_dir(inst.id).join("config/test.txt").exists());
    assert_eq!(
        e.svc.list(inst.id).await.unwrap()[0].source,
        ContentSourceKind::CurseForge
    );
    e.cf_project(9, 4471, true);
    let mut pack = e.cf_file(9, 99, serde_json::json!([]));
    pack["fileName"] = serde_json::json!("pack.zip");
    pack["fileLength"] = serde_json::json!(bytes.len());
    pack["hashes"] = serde_json::json!([{"algo":1,"value":sha1_hex(&bytes)}]);
    let url = pack["downloadUrl"].as_str().unwrap().to_owned();
    e.dl.files.lock().unwrap().insert(url, bytes);
    e.cf_register(9, &pack);
    e.svc
        .install_remote(
            ContentSourceKind::CurseForge,
            "9",
            Some("99"),
            &NullSink::default(),
        )
        .await
        .unwrap();
    e.cf_project(1, 6, false);
    let before = e.inst.map.lock().unwrap().len();
    let error = e
        .svc
        .import(path.to_str().unwrap(), &NullSink::default())
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ManualDownloadRequired);
    assert_eq!(e.inst.map.lock().unwrap().len(), before);
}
#[tokio::test]
async fn curseforge_pack_rejects_override_traversal_and_hash_mismatch() {
    let e = env();
    e.cf_project(1, 6, true);
    let f = e.cf_file(1, 11, serde_json::json!([]));
    for (prefix, entry) in [
        ("../escape", "overrides/a"),
        ("overrides", "overrides/../../escape.txt"),
    ] {
        let path = e._dir.path().join("bad.zip");
        std::fs::write(
            &path,
            zip_bytes(
                e._dir.path(),
                &[
                    (
                        "manifest.json",
                        serde_json::to_vec(&cf_manifest(prefix)).unwrap(),
                    ),
                    (entry, b"x".to_vec()),
                ],
            ),
        )
        .unwrap();
        assert_eq!(
            e.svc
                .import(path.to_str().unwrap(), &NullSink::default())
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        assert!(e.inst.map.lock().unwrap().is_empty());
    }
    let path = e._dir.path().join("hash.zip");
    std::fs::write(
        &path,
        zip_bytes(
            e._dir.path(),
            &[(
                "manifest.json",
                serde_json::to_vec(&cf_manifest("overrides")).unwrap(),
            )],
        ),
    )
    .unwrap();
    e.dl.files.lock().unwrap().insert(
        f["downloadUrl"].as_str().unwrap().to_owned(),
        b"corrupt".to_vec(),
    );
    assert_eq!(
        e.svc
            .import(path.to_str().unwrap(), &NullSink::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::HashMismatch
    );
    assert!(e.inst.map.lock().unwrap().is_empty());
}

#[tokio::test]
async fn curseforge_pack_rejects_compressed_manifest_and_total_download_quota() {
    let e = env();
    let path = e._dir.path().join("bomb.zip");
    std::fs::write(
        &path,
        zip_bytes(
            e._dir.path(),
            &[("manifest.json", vec![b' '; 8 * 1024 * 1024 + 1])],
        ),
    )
    .unwrap();
    assert_eq!(
        e.svc
            .import(path.to_str().unwrap(), &NullSink::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    e.cf_project(1, 6, true);
    let mut f = e.cf_file(1, 11, serde_json::json!([]));
    f["fileLength"] = serde_json::json!(4u64 * 1024 * 1024 * 1024 + 1);
    e.cf_register(1, &f);
    std::fs::write(
        &path,
        zip_bytes(
            e._dir.path(),
            &[(
                "manifest.json",
                serde_json::to_vec(&cf_manifest("overrides")).unwrap(),
            )],
        ),
    )
    .unwrap();
    assert_eq!(
        e.svc
            .import(path.to_str().unwrap(), &NullSink::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    assert!(e.inst.map.lock().unwrap().is_empty());
}

#[tokio::test]
async fn curseforge_large_pack_batches_are_bounded_and_fail_closed() {
    let e = env();
    let mut manifest = cf_manifest("overrides");
    let mut entries = Vec::new();
    for project in 1..=65 {
        e.cf_project(project, 6, true);
        e.cf_file(project, project + 100, serde_json::json!([]));
        entries.push(serde_json::json!({"projectID":project,"fileID":project+100,"required":true}));
    }
    manifest["files"] = serde_json::json!(entries);
    let path = e._dir.path().join("large.zip");
    std::fs::write(
        &path,
        zip_bytes(
            e._dir.path(),
            &[("manifest.json", serde_json::to_vec(&manifest).unwrap())],
        ),
    )
    .unwrap();
    let inst = e
        .svc
        .import(path.to_str().unwrap(), &NullSink::default())
        .await
        .unwrap();
    assert_eq!(
        e.svc
            .inner
            .scan(&e.inst.game_dir(inst.id))
            .await
            .unwrap()
            .len(),
        65
    );
    let calls = e.http.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 4);
    for call in &calls {
        assert!(call.starts_with("POST "));
        let body: serde_json::Value =
            serde_json::from_str(call.splitn(3, ' ').nth(2).unwrap()).unwrap();
        assert!(
            body.as_object()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .as_array()
                .unwrap()
                .len()
                <= 50
        );
    }
    // Successful previous responses must never mask missing files or changed permissions.
    e.http
        .routes
        .lock()
        .unwrap()
        .remove(&format!("{CF}/files/165"));
    assert_eq!(
        e.svc
            .import(path.to_str().unwrap(), &NullSink::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(e.inst.map.lock().unwrap().len(), 1);
    e.cf_file(65, 165, serde_json::json!([]));
    e.cf_project(65, 6, false);
    assert_eq!(
        e.svc
            .import(path.to_str().unwrap(), &NullSink::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::ManualDownloadRequired
    );
    assert_eq!(e.inst.map.lock().unwrap().len(), 1);
}
