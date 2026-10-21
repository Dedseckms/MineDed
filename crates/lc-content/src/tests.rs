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
const CF: &str = "https://api.curseforge.com";

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
            std::fs::create_dir_all(r.dest.parent().unwrap()).unwrap();
            std::fs::write(&r.dest, bytes).unwrap();
        }
        Ok(())
    }
    async fn get_bytes(&self, url: &Url, _k: ResourceKind) -> AppResult<Vec<u8>> {
        Err(AppError::new(ErrorCode::NotFound, "unused").detail("url", url.as_str()))
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
        Ok(i.clone())
    }
    async fn delete(&self, _id: InstanceId) -> AppResult<()> {
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

fn env(cf_key: Option<&str>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    let http = Arc::new(FakeHttp::default());
    let dl = Arc::new(Dl::default());
    let inst = Arc::new(Instances {
        paths: paths.clone(),
        map: Mutex::new(HashMap::new()),
    });
    let svc = ContentServices::with_parts(
        paths,
        dl.clone(),
        inst.clone(),
        Arc::new(Sys),
        http.clone(),
        cf_key.map(str::to_owned),
    );
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
    let e = env(None);
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
    let e = env(None);
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
async fn curseforge_disabled_without_key_modrinth_works() {
    let e = env(None);
    assert!(!e.svc.curseforge_enabled());
    let q = SearchQuery {
        source: ContentSourceKind::CurseForge,
        kind: ContentKind::Mod,
        text: "jei".into(),
        mc: None,
        loader: None,
        categories: vec![],
        sort: SortBy::Relevance,
        offset: 0,
        limit: 10,
    };
    let err = e.svc.search(q.clone()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthRequired);
    e.http.route(
        &format!("{MR}/search"),
        r#"{"hits":[{"project_id":"P","slug":"p","title":"P","project_type":"mod"}],"offset":0,"limit":10,"total_hits":1}"#,
    );
    let page = e
        .svc
        .search(SearchQuery {
            source: ContentSourceKind::Modrinth,
            ..q
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.total, 1);
}

#[tokio::test]
async fn curseforge_opt_out_becomes_manual_download() {
    let e = env(Some("test-key"));
    let id = e.add_instance("1.20.1", LoaderKind::Forge);
    let mods: serde_json::Value =
        serde_json::from_str(include_str!("../tests/fixtures/curseforge/mods.json")).unwrap();
    let denied = serde_json::json!({"data": mods["data"][1]});
    e.http
        .route(&format!("{CF}/v1/mods/1001"), denied.to_string());
    e.http.route(
        &format!("{CF}/v1/mods/1001/files"),
        include_str!("../tests/fixtures/curseforge/files.json"),
    );
    e.http.route(
        &format!("POST {CF}/v1/mods/files"),
        include_str!("../tests/fixtures/curseforge/files.json"),
    );
    let plan = e
        .svc
        .resolve(
            id,
            vec![ContentVersionRef {
                source: ContentSourceKind::CurseForge,
                project_id: "1001".into(),
                version_id: None,
            }],
        )
        .await
        .unwrap();
    assert!(plan.install.is_empty());
    assert_eq!(plan.manual.len(), 1);
    assert_eq!(
        plan.manual[0].url,
        "https://www.curseforge.com/minecraft/mc-mods/denied-mod/download/4711"
    );
    // Required dependency 2002 is unknown → reported, not fatal.
    assert!(plan.conflicts.iter().any(|c| c.project_id == "2002"));
    // Installing the plan never downloads the opted-out file.
    e.svc.install(id, plan, &NullSink::default()).await.unwrap();
    assert!(!e.inst.game_dir(id).join("mods/somemod-1.2.3.jar").exists());
}

#[tokio::test]
async fn preset_installs_mods_and_flags() {
    let e = env(None);
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
}
