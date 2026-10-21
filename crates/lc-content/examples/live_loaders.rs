//! Manual network check (mods): installs Fabric 1.21.x and Forge 1.20.1 through
//! the real `CoreServices` + loader registry into a separate data dir, adds
//! Sodium from Modrinth to the Fabric instance, then launches each instance for
//! ~60 s with an offline account.
//!
//! `cargo run -p lc-content --example live_loaders -- [data_dir] [--no-launch]`
//! Default data dir: `%TEMP%/mineup-live` (or `$TMPDIR`).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lc_accounts::{AccountService, AccountsService, MemoryTokenStore};
use lc_content::{ContentService, ContentServices};
use lc_core::{CoreServices, GameLauncher, GameObserver, InstanceService};
use lc_http::{DownloaderConfig, HttpDownloader};
use lc_runtime::{CancelToken, Paths, ProgressSink};
use lc_types::content::{ContentKind, ContentSourceKind, ContentVersionRef, SearchQuery, SortBy};
use lc_types::events::{InstanceLogPayload, InstanceStatePayload};
use lc_types::instance::{InstanceSpec, LaunchOpts, LoaderRef, RunState};
use lc_types::task::TaskEvent;
use lc_types::version::LoaderKind;
use lc_types::TaskId;

#[derive(Default)]
struct Obs {
    lines: Mutex<Vec<String>>,
}

impl GameObserver for Obs {
    fn on_state(&self, p: InstanceStatePayload) {
        println!("  [state] {:?} exit={:?}", p.state, p.exit_code);
    }
    fn on_log(&self, p: InstanceLogPayload) {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(p.lines);
    }
}

struct Sink {
    id: TaskId,
    cancel: CancelToken,
    last: Mutex<Instant>,
}

impl ProgressSink for Sink {
    fn task_id(&self) -> TaskId {
        self.id
    }
    fn emit(&self, event: TaskEvent) {
        match &event {
            TaskEvent::Progress {
                done, total, stage, ..
            } => {
                let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed() >= Duration::from_secs(5) {
                    *last = Instant::now();
                    println!("  [{stage}] {done} / {total:?}");
                }
            }
            TaskEvent::Log { line, .. } => println!("  | {line}"),
            _ => {}
        }
    }
    fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
}

fn sink() -> Sink {
    Sink {
        id: TaskId::new(),
        cancel: CancelToken::new(),
        last: Mutex::new(Instant::now() - Duration::from_secs(10)),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let launch = !args.iter().any(|a| a == "--no-launch");
    let root = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("mineup-live"));
    println!("data dir: {}", root.display());
    let paths = Paths::new(&root);
    let dl: Arc<dyn lc_http::Downloader> =
        Arc::new(HttpDownloader::new(DownloaderConfig::default())?);
    let accounts = Arc::new(AccountsService::new(
        &paths,
        dl.clone(),
        Arc::new(MemoryTokenStore::new()),
    ));
    let account = accounts.offline_add("LiveTest").await?.id;
    let loaders = Arc::new(lc_loaders::default_registry(dl.clone()));
    let obs = Arc::new(Obs::default());
    let core = Arc::new(CoreServices::new(
        paths.clone(),
        dl.clone(),
        accounts,
        loaders.clone(),
        obs.clone(),
        Vec::new(),
    ));
    let content = ContentServices::new(paths.clone(), dl.clone(), core.clone(), core.clone());

    let fabric_mc = "1.21.1";
    let mut ok = true;
    for (mc, kind) in [
        (fabric_mc, LoaderKind::Fabric),
        ("1.20.1", LoaderKind::Forge),
    ] {
        println!("== {kind:?} {mc} ==");
        let versions = loaders.get(kind)?.versions(mc).await?;
        let rec = versions
            .iter()
            .find(|v| v.recommended)
            .or_else(|| versions.first())
            .map(|v| v.version.clone());
        println!("  {} builds, recommended {rec:?}", versions.len());
        let t = Instant::now();
        let inst = core
            .create(
                InstanceSpec {
                    name: format!("Live {kind:?} {mc}"),
                    mc: mc.into(),
                    loader: LoaderRef { kind, version: rec },
                    preset: None,
                    icon: None,
                },
                &sink(),
            )
            .await;
        let inst = match inst {
            Ok(i) => i,
            Err(e) => {
                println!("  RESULT install {kind:?}: FAILED {e:?}");
                ok = false;
                continue;
            }
        };
        println!(
            "  installed {:?} in {:.0}s",
            inst.loader,
            t.elapsed().as_secs_f32()
        );

        if kind == LoaderKind::Fabric {
            let page = content
                .search(SearchQuery {
                    source: ContentSourceKind::Modrinth,
                    kind: ContentKind::Mod,
                    text: "sodium".into(),
                    mc: Some(mc.into()),
                    loader: Some(kind),
                    categories: vec![],
                    sort: SortBy::Relevance,
                    offset: 0,
                    limit: 5,
                })
                .await?;
            println!("  modrinth search: {} hits", page.total);
            let plan = content
                .resolve(
                    inst.id,
                    vec![ContentVersionRef {
                        source: ContentSourceKind::Modrinth,
                        project_id: "AANobbMI".into(),
                        version_id: None,
                    }],
                )
                .await?;
            println!(
                "  plan: install {:?}",
                plan.install
                    .iter()
                    .map(|v| &v.file_name)
                    .collect::<Vec<_>>()
            );
            content.install(inst.id, plan, &sink()).await?;
            println!("  content: {:?}", content.list(inst.id).await?);
        }

        if !launch {
            continue;
        }
        obs.lines.lock().unwrap_or_else(|e| e.into_inner()).clear();
        core.launch(
            inst.id,
            LaunchOpts {
                account: Some(account),
                ..Default::default()
            },
            &sink(),
        )
        .await?;
        tokio::time::sleep(Duration::from_secs(60)).await;
        let st = core.state(inst.id);
        let lines = obs.lines.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for l in lines.iter().rev().take(6).rev() {
            println!("    | {}", l.chars().take(160).collect::<String>());
        }
        if st == RunState::Running {
            core.kill(inst.id).await?;
            println!("  RESULT {kind:?} {mc}: OK (running after 60s, killed)");
        } else {
            println!("  RESULT {kind:?} {mc}: FAILED ({st:?})");
            ok = false;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    if ok {
        Ok(())
    } else {
        Err("live check failed".into())
    }
}
