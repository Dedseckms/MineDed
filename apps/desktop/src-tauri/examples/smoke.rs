//! Headless end-to-end smoke test (lead): offline account → create vanilla
//! instance → launch → wait until the game is up → kill.
//!
//! `cargo run -p desktop --example smoke -- 1.21 1.12.2`
//! An argument like `1.21` picks the newest release with that prefix.
//! Uses the same data dir as the app (`%APPDATA%/ru.dedseckms.mineup`).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use desktop_lib::state::AppState;
use lc_core::GameObserver;
use lc_runtime::{CancelToken, JsonSettingsStore, Paths, ProgressSink, SettingsStore};
use lc_types::events::{InstanceLogPayload, InstanceStatePayload};
use lc_types::instance::{InstanceSpec, LaunchOpts, LoaderRef, RunState};
use lc_types::task::TaskEvent;
use lc_types::version::{LoaderKind, VersionFilter, VersionKind};
use lc_types::TaskId;

#[derive(Default)]
struct Obs {
    state: Mutex<Option<RunState>>,
    lines: Mutex<Vec<String>>,
}

impl GameObserver for Obs {
    fn on_state(&self, p: InstanceStatePayload) {
        println!(
            "  [state] {:?} exit={:?} crash={:?}",
            p.state, p.exit_code, p.crash_report
        );
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = Some(p.state);
    }
    fn on_log(&self, p: InstanceLogPayload) {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(p.lines);
    }
}

struct PrintSink {
    id: TaskId,
    cancel: CancelToken,
    last: Mutex<Instant>,
}

impl ProgressSink for PrintSink {
    fn task_id(&self) -> TaskId {
        self.id
    }
    fn emit(&self, event: TaskEvent) {
        if let TaskEvent::Progress {
            done, total, stage, ..
        } = &event
        {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            if last.elapsed() < Duration::from_secs(3) {
                return;
            }
            *last = Instant::now();
            println!(
                "  [{stage}] {} / {:?} MB",
                done / 1_048_576,
                total.map(|t| t / 1_048_576)
            );
        }
    }
    fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
}

struct NoRoomObs;

impl lc_p2p::RoomObserver for NoRoomObs {
    fn on_state(&self, _state: Option<lc_types::room::RoomState>) {}
    fn on_peer(&self, _peer: lc_types::room::Peer, _joined: bool) {}
    fn on_chat(&self, _message: lc_types::voice::ChatMessage) {}
}

impl lc_voice::VoiceObserver for NoRoomObs {
    fn on_speaking(&self, _payload: lc_types::events::VoiceSpeakingPayload) {}
}

fn sink() -> PrintSink {
    PrintSink {
        id: TaskId::new(),
        cancel: CancelToken::new(),
        last: Mutex::new(Instant::now() - Duration::from_secs(10)),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let wanted: Vec<String> = std::env::args().skip(1).collect();
    let appdata = std::env::var("APPDATA").map_err(|_| "APPDATA not set")?;
    let root = std::path::Path::new(&appdata).join("ru.dedseckms.mineup");
    let store: Arc<dyn SettingsStore> =
        Arc::new(JsonSettingsStore::new(root.join("settings.json")));
    let settings = store.get().await?;
    let obs = Arc::new(Obs::default());
    let state = AppState::real(
        Paths::new(&root),
        store,
        &settings,
        obs.clone(),
        Arc::new(NoRoomObs),
        Arc::new(NoRoomObs),
    )?;

    let accounts = state.accounts.list().await?;
    if !accounts.iter().any(|a| a.name == "SmokeTest") {
        state.accounts.offline_add("SmokeTest").await?;
    }
    let account = state
        .accounts
        .list()
        .await?
        .into_iter()
        .find(|a| a.name == "SmokeTest")
        .map(|a| a.id);

    let versions = state
        .versions
        .list(VersionFilter {
            kinds: vec![VersionKind::Release],
            search: None,
        })
        .await?;
    let mut ok = true;
    for want in &wanted {
        let Some(v) = versions
            .iter()
            .find(|v| v.id == *want || v.id.starts_with(&format!("{want}.")))
        else {
            println!("== {want}: no such release");
            ok = false;
            continue;
        };
        println!("== {} ==", v.id);
        let name = format!("Smoke {}", v.id);
        let existing = state
            .instances
            .list()
            .await?
            .into_iter()
            .find(|i| i.name == name);
        let inst = match existing {
            Some(i) => i,
            None => {
                let t = Instant::now();
                let i = state
                    .instances
                    .create(
                        InstanceSpec {
                            name,
                            mc: v.id.clone(),
                            loader: LoaderRef {
                                kind: LoaderKind::Vanilla,
                                version: None,
                            },
                            preset: None,
                            icon: None,
                        },
                        &sink(),
                    )
                    .await?;
                println!("  created in {:.0}s", t.elapsed().as_secs_f32());
                i
            }
        };

        obs.lines.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let t = Instant::now();
        state
            .launcher
            .launch(
                inst.id,
                LaunchOpts {
                    account,
                    ..Default::default()
                },
                &sink(),
            )
            .await?;
        println!(
            "  launched in {:.0}s, waiting 45s for the game",
            t.elapsed().as_secs_f32()
        );
        tokio::time::sleep(Duration::from_secs(45)).await;

        let st = state.launcher.state(inst.id);
        let lines = obs.lines.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let alive = st == RunState::Running;
        println!("  state after 45s: {st:?}, log lines: {}", lines.len());
        for l in lines.iter().rev().take(8).rev() {
            println!("    | {}", l.chars().take(160).collect::<String>());
        }
        if alive {
            state.launcher.kill(inst.id).await?;
            println!("  RESULT {}: OK (running, killed)", v.id);
        } else {
            println!("  RESULT {}: FAILED", v.id);
            ok = false;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if ok {
        Ok(())
    } else {
        Err("smoke failed".into())
    }
}
