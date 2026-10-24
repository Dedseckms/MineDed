//! `CoreServices`: one object implementing every core trait, wired by the lead.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use lc_accounts::AuthProvider;
use lc_http::Downloader;
use lc_loaders::{InstallCtx, LoaderRegistry};
use lc_runtime::{CancelToken, LaunchContributor, Paths, ProgressSink};
use lc_types::events::{InstanceLogPayload, InstanceStatePayload};
use lc_types::instance::{
    Instance, InstancePatch, InstanceSpec, JavaCfg, LaunchOpts, LoaderRef, MemoryCfg, RunState,
};
use lc_types::java::JavaRuntime;
use lc_types::system::{MemoryRecommendation, SystemInfo};
use lc_types::version::{LoaderKind, VersionEntry, VersionFilter};
use lc_types::{AppError, AppResult, ErrorCode, InstanceId};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;

use crate::contract::{
    GameLauncher, GameObserver, InstanceService, JavaManager, SystemProbe, VersionService,
};
use crate::instances::{apply_patch, InstanceFile, InstanceStore};
use crate::java::{command, game_executable, JavaService};
use crate::launch::{
    build_plan, extract_natives, redacted, render_args, supports_quick_play, PlanInputs,
};
use crate::logparse::Log4jParser;
use crate::meta::{Env, LibUse, VersionJson};
use crate::prepare::{prepare, Prepared};
use crate::system;
use crate::util::{join, now_rfc3339};
use crate::versions::{Versions, DEFAULT_JAVA_MAJOR};

const LOG_BATCH: Duration = Duration::from_millis(100);
const LOG_BATCH_MAX: usize = 500;
/// Longer game log lines are truncated.
const MAX_LOG_LINE: usize = 64 * 1024;

struct Run {
    state: RunState,
    kill: Option<CancelToken>,
}

struct Inner {
    paths: Paths,
    dl: Arc<dyn Downloader>,
    auth: Arc<dyn AuthProvider>,
    loaders: Arc<LoaderRegistry>,
    observer: Arc<dyn GameObserver>,
    contributors: Vec<Arc<dyn LaunchContributor>>,
    versions: Versions,
    java: JavaService,
    store: InstanceStore,
    runs: Mutex<HashMap<InstanceId, Run>>,
    env: Env,
}

/// Real implementation of `VersionService`, `JavaManager`, `InstanceService`,
/// `GameLauncher` and `SystemProbe`. Cheap to clone; share one instance.
#[derive(Clone)]
pub struct CoreServices {
    inner: Arc<Inner>,
}

impl CoreServices {
    /// `contributors` are applied in the given order (accounts → mods → vpn → p2p).
    pub fn new(
        paths: Paths,
        downloader: Arc<dyn Downloader>,
        auth: Arc<dyn AuthProvider>,
        loaders: Arc<LoaderRegistry>,
        observer: Arc<dyn GameObserver>,
        contributors: Vec<Arc<dyn LaunchContributor>>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                versions: Versions::new(paths.clone(), downloader.clone()),
                java: JavaService::new(paths.clone(), downloader.clone()),
                store: InstanceStore::new(paths.clone()),
                paths,
                dl: downloader,
                auth,
                loaders,
                observer,
                contributors,
                runs: Mutex::new(HashMap::new()),
                env: Env::current(),
            }),
        }
    }
}

impl Inner {
    fn set_state(
        &self,
        id: InstanceId,
        state: RunState,
        kill: Option<CancelToken>,
        exit_code: Option<i32>,
        crash_report: Option<String>,
    ) {
        if let Ok(mut runs) = self.runs.lock() {
            if state == RunState::Stopped || state == RunState::Crashed {
                runs.remove(&id);
            } else {
                runs.insert(id, Run { state, kill });
            }
        }
        self.observer.on_state(InstanceStatePayload {
            id,
            state,
            exit_code,
            crash_report,
        });
    }

    /// Reserves `id` in the run table as `Preparing`; `Conflict` when it is
    /// already launching, running or being repaired.
    fn reserve(&self, id: InstanceId, notify: bool) -> AppResult<RunGuard<'_>> {
        {
            let mut runs = self
                .runs
                .lock()
                .map_err(|_| AppError::internal("run table poisoned"))?;
            if runs.contains_key(&id) {
                return Err(AppError::new(ErrorCode::Conflict, "instance is busy")
                    .detail("id", id.to_string()));
            }
            runs.insert(
                id,
                Run {
                    state: RunState::Preparing,
                    kill: None,
                },
            );
        }
        if notify {
            self.observer.on_state(InstanceStatePayload {
                id,
                state: RunState::Preparing,
                exit_code: None,
                crash_report: None,
            });
        }
        Ok(RunGuard {
            inner: self,
            id,
            armed: true,
            notify,
        })
    }

    fn run_state(&self, id: InstanceId) -> RunState {
        self.runs
            .lock()
            .ok()
            .and_then(|r| r.get(&id).map(|r| r.state))
            .unwrap_or(RunState::Stopped)
    }

    /// Java for loader installers (Forge processors) and the game.
    async fn java_for(
        &self,
        inst: &Instance,
        profile: Option<&VersionJson>,
        sink: &dyn ProgressSink,
    ) -> AppResult<PathBuf> {
        if let Some(p) = inst.java.path.as_deref().filter(|p| !p.trim().is_empty()) {
            let p = PathBuf::from(p);
            if !tokio::fs::try_exists(&p).await.unwrap_or(false) {
                return Err(
                    AppError::new(ErrorCode::JavaMissing, "configured java not found")
                        .detail("path", p.to_string_lossy()),
                );
            }
            return Ok(p);
        }
        let jv = profile.and_then(|p| p.java_version.clone());
        let major = inst
            .java
            .major
            .or_else(|| jv.as_ref().map(|j| j.major_version))
            .unwrap_or(DEFAULT_JAVA_MAJOR);
        // An explicit major override ignores the profile's component.
        let component = if inst.java.major.is_some() {
            None
        } else {
            jv.and_then(|j| j.component)
        };
        let rt = self
            .java
            .ensure_component(component.as_deref(), major, sink)
            .await?;
        Ok(PathBuf::from(rt.path))
    }

    /// Installs the loader (if any) and returns the version profile id.
    async fn install_loader(
        &self,
        inst: &Instance,
        sink: &dyn ProgressSink,
    ) -> AppResult<Option<String>> {
        if inst.loader.kind == LoaderKind::Vanilla {
            return Ok(None);
        }
        let installer = self.loaders.get(inst.loader.kind)?;
        let version = match &inst.loader.version {
            Some(v) => v.clone(),
            None => {
                let list = installer.versions(&inst.mc).await?;
                list.iter()
                    .find(|v| v.recommended)
                    .or_else(|| list.iter().find(|v| v.stable))
                    .or_else(|| list.first())
                    .map(|v| v.version.clone())
                    .ok_or_else(|| {
                        AppError::new(ErrorCode::LoaderInstall, "no loader builds for version")
                            .detail("mc", inst.mc.as_str())
                    })?
            }
        };
        let vanilla = self.versions.load_profile(&inst.mc, sink).await?;
        let java = self.java_for(inst, Some(&vanilla), sink).await?;
        let ctx = InstallCtx {
            paths: self.paths.clone(),
            downloader: self.dl.clone(),
            java,
        };
        sink.check_cancelled()?;
        let profile = installer.install(&ctx, &inst.mc, &version, sink).await?;
        Ok(Some(profile))
    }

    async fn prepare_instance(
        &self,
        file: &InstanceFile,
        verify: bool,
        sink: &dyn ProgressSink,
    ) -> AppResult<Prepared> {
        let profile = self.versions.load_profile(file.profile_id(), sink).await?;
        let game_dir = self.store.game_dir(file.instance.id);
        prepare(
            &self.paths,
            self.dl.as_ref(),
            profile,
            &game_dir,
            &self.env,
            verify,
            sink,
        )
        .await
    }

    async fn create(&self, spec: InstanceSpec, sink: &dyn ProgressSink) -> AppResult<Instance> {
        let name = spec.name.trim();
        if name.is_empty() || spec.mc.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "name and mc are required",
            ));
        }
        let (total, free) = system::ram_mb().await.unwrap_or((8192, 4096));
        let rec = system::recommend(total, free, 0, spec.preset);
        let instance = Instance {
            id: InstanceId::new(),
            name: name.to_owned(),
            mc: spec.mc.trim().to_owned(),
            loader: LoaderRef {
                kind: spec.loader.kind,
                version: if spec.loader.kind == LoaderKind::Vanilla {
                    None
                } else {
                    spec.loader.version.clone()
                },
            },
            java: JavaCfg::default(),
            memory: MemoryCfg {
                min_mb: rec.min_mb,
                max_mb: rec.max_mb,
                auto: true,
            },
            jvm_args: Vec::new(),
            preset: spec.preset,
            icon: spec.icon,
            created_at: now_rfc3339(),
            last_played: None,
            playtime_sec: 0,
        };
        let id = instance.id;
        let result = async {
            let mut file = InstanceFile {
                instance,
                version_profile: None,
            };
            file.version_profile = self.install_loader(&file.instance, sink).await?;
            if let Some(p) = &file.version_profile {
                let v = &mut file.instance.loader.version;
                if v.is_none() {
                    *v = loader_version_from_profile(p, &file.instance.mc);
                }
            }
            self.prepare_instance(&file, true, sink).await?;
            self.store.save(&file).await?;
            Ok(file.instance)
        }
        .await;
        if result.is_err() {
            self.store.discard(id).await;
        }
        result
    }

    async fn repair(&self, id: InstanceId, sink: &dyn ProgressSink) -> AppResult<()> {
        let _guard = self.reserve(id, false)?;
        let mut file = self.store.load(id).await?;
        if let Some(profile) = self.install_loader(&file.instance, sink).await? {
            file.version_profile = Some(profile);
        }
        let prepared = self.prepare_instance(&file, true, sink).await?;
        self.java_for(&file.instance, Some(&prepared.profile), sink)
            .await?;
        let profile = file.version_profile.clone();
        self.store
            .modify(id, |f| {
                f.version_profile = profile;
                Ok(())
            })
            .await?;
        Ok(())
    }

    fn ensure_not_running(&self, id: InstanceId) -> AppResult<()> {
        match self.run_state(id) {
            RunState::Running | RunState::Preparing => {
                Err(AppError::new(ErrorCode::Conflict, "instance is running")
                    .detail("id", id.to_string()))
            }
            _ => Ok(()),
        }
    }

    async fn memory_for(&self, inst: &Instance) -> (u32, u32) {
        if !inst.memory.auto {
            return (
                inst.memory.min_mb.min(inst.memory.max_mb),
                inst.memory.max_mb,
            );
        }
        let mods = system::count_mods(&self.store.game_dir(inst.id)).await;
        let (total, free) = system::ram_mb().await.unwrap_or((8192, 4096));
        let r = system::recommend(total, free, mods, inst.preset);
        (r.min_mb, r.max_mb)
    }

    async fn launch_inner(
        self: &Arc<Self>,
        id: InstanceId,
        opts: LaunchOpts,
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        let file = self.store.load(id).await?;
        let inst = &file.instance;
        let prepared = self
            .prepare_instance(&file, !opts.offline_fast, sink)
            .await?;
        sink.check_cancelled()?;
        let java = self.java_for(inst, Some(&prepared.profile), sink).await?;
        sink.check_cancelled()?;

        let auth = self.auth.launch_auth(opts.account).await?;
        let natives_dir = self.store.natives_dir(id);
        let game_dir = self.store.game_dir(id);
        tokio::fs::create_dir_all(&game_dir)
            .await
            .map_err(|e| crate::util::io_err(&game_dir, e))?;
        let mut jars: Vec<(PathBuf, Vec<String>)> = Vec::new();
        for l in &prepared.libs {
            if let LibUse::Native { exclude } = &l.usage {
                jars.push((l.local(&self.paths.libraries())?, exclude.clone()));
            }
        }
        {
            let dir = natives_dir.clone();
            join(tokio::task::spawn_blocking(move || extract_natives(&jars, &dir)).await)?;
        }

        let (min_mb, max_mb) = self.memory_for(inst).await;
        let mut plan = build_plan(PlanInputs {
            prepared: &prepared,
            env: &self.env,
            libraries_dir: &self.paths.libraries(),
            assets_root: &self.paths.assets(),
            game_dir: &game_dir,
            natives_dir: &natives_dir,
            java: game_executable(&java),
            auth: &auth,
            min_mb,
            max_mb,
            extra_jvm: &inst.jvm_args,
            join_server: opts.join_server.as_deref(),
        })?;
        for c in &self.contributors {
            c.contribute(inst, &opts, &mut plan).map_err(|e| {
                let name = c.name();
                e.detail("contributor", name)
            })?;
        }
        let args = render_args(&plan, supports_quick_play(&prepared.profile));
        sink.log(&format!(
            "{} {}",
            plan.java.to_string_lossy(),
            redacted(&args, auth.access_token.expose())
        ));
        sink.check_cancelled()?;

        let mut cmd = command(&plan.java);
        cmd.args(&args)
            .current_dir(&plan.working_dir)
            .envs(&plan.env)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| {
            AppError::new(ErrorCode::JavaMissing, format!("failed to start java: {e}"))
                .detail("path", plan.java.to_string_lossy())
        })?;

        let kill = CancelToken::new();
        self.set_state(id, RunState::Running, Some(kill.clone()), None, None);
        let started_at = SystemTime::now();
        let started = Instant::now();

        // No `.await` between spawn and handing the child to its monitor task:
        // a dropped launch future must never orphan a running process.
        let (tx, rx) = mpsc::channel::<String>(4096);
        if let Some(out) = child.stdout.take() {
            tokio::spawn(pump(out, tx.clone()));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(pump(err, tx.clone()));
        }
        drop(tx);
        let batcher = tokio::spawn(batch_logs(id, rx, self.observer.clone()));

        let me = self.clone();
        tokio::spawn(async move {
            let _ = me
                .store
                .modify(id, |f| {
                    f.instance.last_played = Some(now_rfc3339());
                    Ok(())
                })
                .await;
            let mut killed = false;
            let status = tokio::select! {
                s = child.wait() => s,
                _ = kill.cancelled() => {
                    killed = true;
                    let _ = child.kill().await;
                    child.wait().await
                }
            };
            let _ = batcher.await;
            let code = status.ok().and_then(|s| s.code());
            let secs = started.elapsed().as_secs();
            let _ = me
                .store
                .modify(id, |f| {
                    f.instance.playtime_sec = f.instance.playtime_sec.saturating_add(secs);
                    Ok(())
                })
                .await;
            let crash = find_crash_report(&me.store.game_dir(id), started_at).await;
            let crashed = !killed && (code != Some(0) || crash.is_some());
            let state = if crashed {
                RunState::Crashed
            } else {
                RunState::Stopped
            };
            me.set_state(id, state, None, code, if crashed { crash } else { None });
        });
        Ok(())
    }
}

/// A `Preparing` reservation in the run table. Dropping it (error, or the
/// future being cancelled/dropped) removes the entry unless it was disarmed.
struct RunGuard<'a> {
    inner: &'a Inner,
    id: InstanceId,
    armed: bool,
    /// Emit `Stopped` to the observer on release (launch, not repair).
    notify: bool,
}

impl RunGuard<'_> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let removed = match self.inner.runs.lock() {
            Ok(mut runs) => {
                let preparing = runs
                    .get(&self.id)
                    .map(|r| r.state == RunState::Preparing)
                    .unwrap_or(false);
                preparing && runs.remove(&self.id).is_some()
            }
            Err(_) => false,
        };
        if removed && self.notify {
            self.inner.observer.on_state(InstanceStatePayload {
                id: self.id,
                state: RunState::Stopped,
                exit_code: None,
                crash_report: None,
            });
        }
    }
}

/// Loader version from an installed profile id (best effort, for display only):
/// `fabric-loader-0.16.9-1.21.1`, `quilt-loader-0.26.4-1.21.1` → `0.16.9`/`0.26.4`;
/// `1.20.1-forge-47.4.28` → `47.4.28`; `neoforge-21.1.77` → `21.1.77`;
/// legacy `1.7.10-Forge10.13.4.1614-1.7.10` → `10.13.4.1614`.
fn loader_version_from_profile(profile: &str, mc: &str) -> Option<String> {
    let mut s = profile;
    let mut had_mc = false;
    if let Some(t) = s.strip_suffix(&format!("-{mc}")) {
        s = t;
        had_mc = true;
    }
    if let Some(t) = s.strip_prefix(&format!("{mc}-")) {
        s = t;
        had_mc = true;
    }
    // ASCII lowercasing keeps byte offsets, so indices map back onto `s`.
    let lower = s.to_ascii_lowercase();
    const MARKERS: [&str; 5] = [
        "fabric-loader-",
        "quilt-loader-",
        "neoforge-",
        "forge-",
        "forge",
    ];
    let version = MARKERS
        .iter()
        .find_map(|m| lower.find(m).map(|i| &s[i + m.len()..]))
        .or_else(|| had_mc.then(|| s.rsplit('-').next().unwrap_or(s)))?;
    let version = version.trim_matches('-');
    (!version.is_empty() && version != mc).then(|| version.to_owned())
}

async fn pump(stream: impl AsyncRead + Unpin, tx: mpsc::Sender<String>) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    let mut parser = Log4jParser::default();
    while let Ok(true) = read_line_capped(&mut reader, &mut buf, MAX_LOG_LINE).await {
        let line = String::from_utf8_lossy(&buf);
        for out in parser.feed(line.trim_end_matches(['\r', '\n'])) {
            if tx.send(out).await.is_err() {
                return;
            }
        }
    }
    for out in parser.finish() {
        if tx.send(out).await.is_err() {
            return;
        }
    }
}

/// Reads one line into `buf` (cleared first), keeping at most `max` bytes and
/// discarding the rest of an over-long line without buffering it.
/// Returns `false` at EOF with nothing read.
pub(crate) async fn read_line_capped<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<bool> {
    buf.clear();
    let mut any = false;
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok(any);
        }
        any = true;
        let (take, done) = match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        let room = max.saturating_sub(buf.len());
        buf.extend_from_slice(&chunk[..take.min(room)]);
        reader.consume(take);
        if done {
            return Ok(true);
        }
    }
}

/// Forwards log lines to the observer in batches every ~100 ms.
pub(crate) async fn batch_logs(
    id: InstanceId,
    mut rx: mpsc::Receiver<String>,
    observer: Arc<dyn GameObserver>,
) {
    let mut buf: Vec<String> = Vec::new();
    let mut tick = tokio::time::interval(LOG_BATCH);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let flush = |buf: &mut Vec<String>| {
        if !buf.is_empty() {
            observer.on_log(InstanceLogPayload {
                id,
                lines: std::mem::take(buf),
            });
        }
    };
    loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(l) => {
                    buf.push(l);
                    if buf.len() >= LOG_BATCH_MAX {
                        flush(&mut buf);
                    }
                }
                None => break,
            },
            _ = tick.tick() => flush(&mut buf),
        }
    }
    flush(&mut buf);
}

/// Newest `crash-reports/*.txt` written after `since`.
async fn find_crash_report(game_dir: &std::path::Path, since: SystemTime) -> Option<String> {
    let dir = game_dir.join("crash-reports");
    let mut rd = tokio::fs::read_dir(&dir).await.ok()?;
    let mut best: Option<(SystemTime, PathBuf)> = None;
    while let Ok(Some(e)) = rd.next_entry().await {
        let Ok(meta) = e.metadata().await else {
            continue;
        };
        let Ok(mtime) = meta.modified() else { continue };
        if mtime >= since && best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
            best = Some((mtime, e.path()));
        }
    }
    best.map(|(_, p)| p.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

#[async_trait]
impl VersionService for CoreServices {
    async fn list(&self, filter: VersionFilter) -> AppResult<Vec<VersionEntry>> {
        self.inner.versions.list(filter).await
    }

    async fn java_major(&self, mc: &str) -> AppResult<u32> {
        self.inner.versions.java_major(mc).await
    }
}

#[async_trait]
impl JavaManager for CoreServices {
    async fn list(&self) -> AppResult<Vec<JavaRuntime>> {
        self.inner.java.list().await
    }

    async fn detect(&self) -> AppResult<Vec<JavaRuntime>> {
        self.inner.java.detect().await
    }

    async fn install(&self, major: u32, sink: &dyn ProgressSink) -> AppResult<JavaRuntime> {
        self.inner.java.install(major, sink).await
    }

    async fn ensure(&self, major: u32, sink: &dyn ProgressSink) -> AppResult<JavaRuntime> {
        self.inner.java.ensure(major, sink).await
    }
}

#[async_trait]
impl InstanceService for CoreServices {
    async fn list(&self) -> AppResult<Vec<Instance>> {
        self.inner.store.list().await
    }

    async fn get(&self, id: InstanceId) -> AppResult<Instance> {
        Ok(self.inner.store.load(id).await?.instance)
    }

    async fn create(&self, spec: InstanceSpec, sink: &dyn ProgressSink) -> AppResult<Instance> {
        self.inner.create(spec, sink).await
    }

    async fn clone_instance(
        &self,
        id: InstanceId,
        name: &str,
        sink: &dyn ProgressSink,
    ) -> AppResult<Instance> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "empty instance name",
            ));
        }
        let store = &self.inner.store;
        let src = store.load(id).await?;
        let mut file = src.clone();
        file.instance.id = InstanceId::new();
        file.instance.name = name.to_owned();
        file.instance.created_at = now_rfc3339();
        file.instance.last_played = None;
        file.instance.playtime_sec = 0;
        let new_id = file.instance.id;
        let res = async {
            store.copy_dir(id, new_id, sink).await?;
            store.save(&file).await?;
            Ok(file.instance)
        }
        .await;
        if res.is_err() {
            store.discard(new_id).await;
        }
        res
    }

    async fn update(&self, id: InstanceId, patch: InstancePatch) -> AppResult<Instance> {
        Ok(self
            .inner
            .store
            .modify(id, |f| apply_patch(&mut f.instance, patch))
            .await?
            .instance)
    }

    async fn delete(&self, id: InstanceId) -> AppResult<()> {
        self.inner.ensure_not_running(id)?;
        self.inner.store.delete(id).await
    }

    async fn repair(&self, id: InstanceId, sink: &dyn ProgressSink) -> AppResult<()> {
        self.inner.repair(id, sink).await
    }

    fn game_dir(&self, id: InstanceId) -> PathBuf {
        self.inner.store.game_dir(id)
    }
}

#[async_trait]
impl GameLauncher for CoreServices {
    async fn launch(
        &self,
        id: InstanceId,
        opts: LaunchOpts,
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        let inner = &self.inner;
        let mut guard = inner.reserve(id, true)?;
        let res = inner.launch_inner(id, opts, sink).await;
        if res.is_ok() {
            // The process is running and owned by its monitor task.
            guard.disarm();
        }
        res
    }

    async fn kill(&self, id: InstanceId) -> AppResult<()> {
        let token = self
            .inner
            .runs
            .lock()
            .ok()
            .and_then(|r| r.get(&id).and_then(|r| r.kill.clone()));
        match token {
            Some(t) => {
                t.cancel();
                Ok(())
            }
            None => Err(
                AppError::new(ErrorCode::NotFound, "instance is not running")
                    .detail("id", id.to_string()),
            ),
        }
    }

    fn state(&self, id: InstanceId) -> RunState {
        self.inner.run_state(id)
    }
}

#[async_trait]
impl SystemProbe for CoreServices {
    async fn info(&self) -> AppResult<SystemInfo> {
        system::info().await
    }

    async fn recommend_memory(&self, id: InstanceId) -> AppResult<MemoryRecommendation> {
        let inst = self.inner.store.load(id).await?.instance;
        let mods = system::count_mods(&self.inner.store.game_dir(id)).await;
        let (total, free) = system::ram_mb().await?;
        Ok(system::recommend(total, free, mods, inst.preset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_version_guess() {
        assert_eq!(
            loader_version_from_profile("fabric-loader-0.16.9-1.21.1", "1.21.1").as_deref(),
            Some("0.16.9")
        );
        assert_eq!(loader_version_from_profile("x", "1.21.1"), None);
        let cases = [
            ("quilt-loader-0.26.4-1.21.1", "1.21.1", "0.26.4"),
            ("1.20.1-forge-47.4.28", "1.20.1", "47.4.28"),
            ("1.12.2-forge-14.23.5.2860", "1.12.2", "14.23.5.2860"),
            ("1.7.10-Forge10.13.4.1614-1.7.10", "1.7.10", "10.13.4.1614"),
            ("neoforge-21.1.77", "1.21.1", "21.1.77"),
            ("1.20.1-neoforge-47.1.106", "1.20.1", "47.1.106"),
            ("1.21.1-custom-2.0", "1.21.1", "2.0"),
        ];
        for (profile, mc, want) in cases {
            assert_eq!(
                loader_version_from_profile(profile, mc).as_deref(),
                Some(want),
                "{profile}"
            );
        }
        assert_eq!(loader_version_from_profile("1.21.1", "1.21.1"), None);
    }

    struct Rec(std::sync::Mutex<Vec<Vec<String>>>);
    impl GameObserver for Rec {
        fn on_state(&self, _p: InstanceStatePayload) {}
        fn on_log(&self, p: InstanceLogPayload) {
            self.0.lock().unwrap().push(p.lines);
        }
    }

    #[tokio::test]
    async fn long_log_lines_are_truncated() {
        let mut data = vec![b'a'; 200 * 1024];
        data.extend_from_slice(b"\nshort\r\nlast");
        let mut reader = BufReader::new(&data[..]);
        let mut buf = Vec::new();
        assert!(read_line_capped(&mut reader, &mut buf, MAX_LOG_LINE)
            .await
            .unwrap());
        assert_eq!(buf.len(), MAX_LOG_LINE);
        assert!(read_line_capped(&mut reader, &mut buf, MAX_LOG_LINE)
            .await
            .unwrap());
        assert_eq!(buf, b"short\r\n");
        assert!(read_line_capped(&mut reader, &mut buf, MAX_LOG_LINE)
            .await
            .unwrap());
        assert_eq!(buf, b"last");
        assert!(!read_line_capped(&mut reader, &mut buf, MAX_LOG_LINE)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn pump_parses_log4j_xml() {
        let data = b"<log4j:Event logger=\"a\" timestamp=\"1\" level=\"INFO\" thread=\"Render thread\">\r\n  <log4j:Message><![CDATA[Hello]]></log4j:Message>\r\n</log4j:Event>\r\nplain\n";
        let (tx, mut rx) = mpsc::channel(16);
        pump(&data[..], tx).await;
        let mut got = Vec::new();
        while let Some(l) = rx.recv().await {
            got.push(l);
        }
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].ends_with("[Render thread/INFO]: Hello"), "{got:?}");
        assert_eq!(got[1], "plain");
    }

    #[tokio::test]
    async fn logs_are_batched() {
        let rec = Arc::new(Rec(std::sync::Mutex::new(Vec::new())));
        let (tx, rx) = mpsc::channel(100);
        let h = tokio::spawn(batch_logs(InstanceId::new(), rx, rec.clone()));
        for i in 0..10 {
            tx.send(format!("line {i}")).await.unwrap();
        }
        drop(tx);
        h.await.unwrap();
        let batches = rec.0.lock().unwrap();
        let all: Vec<_> = batches.iter().flatten().cloned().collect();
        assert_eq!(all.len(), 10);
        assert!(batches.len() < 10, "lines must be batched");
    }
}
