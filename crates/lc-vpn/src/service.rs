//! `MihomoVpn`: the real `VpnController` + `LaunchContributor`.
//!
//! mihomo runs as a child process listening only on loopback ports chosen at
//! start. Without admin rights or TUN, except `VpnMode::Apps` (see
//! [`crate::tun`]). A supervisor task restarts it when it crashes;
//! `shutdown` / `kill_now` stop it when the app exits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lc_http::{Downloader, Route};
use lc_runtime::{
    CancelToken, LaunchContributor, LaunchPlan, NullSink, Paths, ProgressSink, SettingsStore,
};
use lc_types::instance::{Instance, LaunchOpts};
use lc_types::vpn::{VpnAccess, VpnApp, VpnMode, VpnNode, VpnSource, VpnState};
use lc_types::{AppError, AppResult, ErrorCode};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use url::Url;

use crate::apps;
use crate::binary;
use crate::config::{self, RunParams, TunRules, GROUP};
use crate::contract::VpnController;
use crate::controller::Controller;
use crate::launch::jvm_proxy_args;
use crate::mineded;
use crate::ports;
use crate::store::{write_private, Files, SourceStore, Stored};
use crate::subscription::{self, name_of, type_of, ProxyMap};
use crate::tun;

const READY_TIMEOUT: Duration = Duration::from_secs(15);
/// Elevated start includes the time the user needs to answer UAC.
#[cfg(windows)]
const ELEVATED_READY_TIMEOUT: Duration = Duration::from_secs(120);
/// How long the elevated helper gets to stop mihomo after the stop flag.
const HELPER_STOP_GRACE: Duration = Duration::from_secs(5);
const MAX_SUBSCRIPTION: usize = 8 * 1024 * 1024;
/// Subscription download without proxy; short so a blocked host falls back
/// to the proxy quickly.
const DIRECT_FETCH_TIMEOUT: Duration = Duration::from_secs(8);
const PROXIED_FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// Crash restarts allowed within [`RESTART_WINDOW`] before giving up.
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(120);

/// Built-in VPN on a mihomo sidecar. Cheap to clone (shared state).
#[derive(Clone)]
pub struct MihomoVpn {
    inner: Arc<Inner>,
}

struct Inner {
    paths: Paths,
    files: Files,
    downloader: Arc<dyn Downloader>,
    binary_override: Option<PathBuf>,
    /// Source of `hub_url`; `None` → the default hub.
    settings: Option<Arc<dyn SettingsStore>>,
    shared: Mutex<Shared>,
    /// Serialises start/stop/reconfigure.
    op: tokio::sync::Mutex<()>,
}

struct Shared {
    store: Stored,
    proxies: Vec<ProxyMap>,
    run: Option<Running>,
    supervisor: Option<(CancelToken, JoinHandle<()>)>,
    latency: HashMap<String, u32>,
    error: Option<AppError>,
}

#[derive(Clone)]
struct Running {
    mixed_port: u16,
    controller_port: u16,
    secret: String,
    controller: Controller,
    /// Our direct child: mihomo, or the PowerShell wrapper of the elevated helper.
    pid: Option<u32>,
    exe: PathBuf,
    /// Config has TUN (`VpnMode::Apps`).
    tun: bool,
    /// Set when mihomo runs under the elevated helper (Windows).
    stop_flag: Option<PathBuf>,
    /// Source whose nodes the running config holds.
    source: Option<VpnSource>,
}

impl Running {
    fn proxy_url(&self) -> Option<Url> {
        Url::parse(&format!("http://127.0.0.1:{}", self.mixed_port)).ok()
    }
}

impl std::fmt::Debug for MihomoVpn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MihomoVpn").finish_non_exhaustive()
    }
}

impl MihomoVpn {
    /// Loads persisted state; does not start anything (see [`Self::restore`]).
    pub fn new(paths: Paths, downloader: Arc<dyn Downloader>) -> Self {
        let files = Files::new(&paths);
        let store = files.load_state();
        let proxies = files.load_nodes(store.source);
        Self {
            inner: Arc::new(Inner {
                paths,
                files,
                downloader,
                binary_override: None,
                settings: None,
                shared: Mutex::new(Shared {
                    store,
                    proxies,
                    run: None,
                    supervisor: None,
                    latency: HashMap::new(),
                    error: None,
                }),
                op: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Uses an explicit mihomo executable instead of bundled/downloaded ones.
    /// Must be called before the instance is shared.
    pub fn with_binary(mut self, path: impl Into<PathBuf>) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.binary_override = Some(path.into());
        }
        self
    }

    /// Reads `hub_url` (for MineDed VPN access and TUN loop protection) from
    /// `settings`. Must be called before the instance is shared.
    pub fn with_settings(mut self, settings: Arc<dyn SettingsStore>) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.settings = Some(settings);
        }
        self
    }

    /// Re-applies the persisted mode at app start (starts mihomo if the VPN
    /// was on). Failures are kept in `state().error` and returned.
    /// `apps` mode is never elevated at startup: it is restored as off and
    /// remembered in the store (`resume`), so one click turns it on again.
    pub async fn restore(&self) -> AppResult<()> {
        let mode = self.shared().store.mode;
        if mode == VpnMode::Apps {
            {
                let mut s = self.shared();
                s.store.mode = VpnMode::Off;
                s.store.resume = Some(VpnMode::Apps);
                s.error = None;
            }
            return self.save();
        }
        if mode == VpnMode::Off || self.shared().proxies.is_empty() {
            return Ok(());
        }
        let _op = self.inner.op.lock().await;
        start_locked(&self.inner, mode, &NullSink::default()).await
    }

    /// Stops mihomo and waits for it to exit. Call on app exit.
    pub async fn shutdown(&self) {
        let _op = self.inner.op.lock().await;
        stop_locked(&self.inner).await;
    }

    /// Synchronous best-effort kill for exit hooks without an async context.
    pub fn kill_now(&self) {
        let (run, sup) = {
            let mut s = self.shared();
            (s.run.take(), s.supervisor.take())
        };
        if let Some((stop, _)) = sup {
            stop.cancel();
        }
        if let Some(run) = run {
            if let Some(flag) = &run.stop_flag {
                tun::signal_stop(flag);
            }
            if let Some(pid) = run.pid {
                kill_pid(pid, &run.exe);
            }
        }
        let _ = std::fs::remove_file(self.inner.files.pid());
    }

    fn shared(&self) -> MutexGuard<'_, Shared> {
        self.inner.shared()
    }

    fn save(&self) -> AppResult<()> {
        let store = self.shared().store.clone();
        self.inner.files.save_state(&store)
    }

    fn effective_selected(s: &Shared) -> Option<String> {
        s.store
            .active()
            .and_then(|a| a.selected.clone())
            .filter(|sel| s.proxies.iter().any(|p| name_of(p) == sel))
            .or_else(|| s.proxies.first().map(|p| name_of(p).to_owned()))
    }

    fn build_state(s: &Shared) -> VpnState {
        let node = Self::effective_selected(s);
        VpnState {
            mode: s.store.mode,
            running: s.run.is_some(),
            latency_ms: node.as_ref().and_then(|n| s.latency.get(n).copied()),
            node,
            subscription_updated: s.store.active().and_then(|a| a.updated.clone()),
            error: s.error.clone(),
            source: s.store.source,
            has_mineded: Some(s.store.mineded.configured()),
            has_own: Some(s.store.own.configured()),
        }
    }

    fn build_nodes(s: &Shared) -> Vec<VpnNode> {
        let selected = Self::effective_selected(s);
        s.proxies
            .iter()
            .map(|p| {
                let name = name_of(p).to_owned();
                VpnNode {
                    latency_ms: s.latency.get(&name).copied(),
                    selected: selected.as_deref() == Some(name.as_str()),
                    protocol: type_of(p).to_owned(),
                    name,
                }
            })
            .collect()
    }

    /// Stores freshly parsed nodes of `source`. When it is the active source
    /// and the nodes changed, a running mihomo reloads them in place.
    async fn install_proxies(&self, source: VpnSource, proxies: Vec<ProxyMap>) -> AppResult<()> {
        let _op = self.inner.op.lock().await;
        self.inner.files.save_nodes(source, &proxies)?;
        let changed = {
            let mut s = self.shared();
            s.store.get_mut(source).updated = Some(now_rfc3339());
            let changed = s.store.source == Some(source) && s.proxies != proxies;
            if changed {
                s.proxies = proxies;
                s.latency.clear();
            }
            changed
        };
        self.save()?;
        self.apply_locked(changed).await
    }

    /// Makes `source` active (its cached nodes become current) and applies
    /// them to a running mihomo. Caller holds `op`.
    async fn switch_locked(&self, source: VpnSource) -> AppResult<()> {
        {
            let mut s = self.shared();
            if s.store.source != Some(source) {
                s.store.source = Some(source);
                s.proxies = self.inner.files.load_nodes(Some(source));
                s.latency.clear();
            }
        }
        self.save()?;
        self.apply_locked(false).await
    }

    /// Brings a running mihomo in line with the active nodes: nothing when
    /// it already runs them (`changed` false, same source), otherwise a
    /// config reload in place (no UAC prompt for TUN); a full restart only
    /// when a plain mixed-port mihomo rejects the reload. Without nodes the
    /// running config is kept until a refresh brings some. Caller holds `op`.
    async fn apply_locked(&self, changed: bool) -> AppResult<()> {
        let (tun, mode) = {
            let s = self.shared();
            let Some(run) = &s.run else { return Ok(()) };
            if s.proxies.is_empty() || (!changed && run.source == s.store.source) {
                return Ok(());
            }
            (run.tun, s.store.mode)
        };
        match reload_locked(&self.inner).await {
            Ok(()) => Ok(()),
            Err(e) if tun => Err(e),
            Err(e) => {
                tracing::warn!(error = %e, "mihomo reload failed; restarting");
                stop_locked(&self.inner).await;
                start_locked(&self.inner, mode, &NullSink::default()).await
            }
        }
    }

    /// Forgets the user's own subscription. If it was active, MineDed takes
    /// over when configured; otherwise the VPN stops. Caller holds `op`.
    async fn remove_own_locked(&self) -> AppResult<()> {
        let _ = std::fs::remove_file(self.inner.files.nodes(VpnSource::Own));
        let (was_active, has_mineded) = {
            let mut s = self.shared();
            s.store.own = SourceStore::default();
            (
                s.store.source == Some(VpnSource::Own),
                s.store.mineded.configured(),
            )
        };
        if !was_active {
            return self.save();
        }
        if has_mineded {
            return self.switch_locked(VpnSource::Mineded).await;
        }
        stop_locked(&self.inner).await;
        {
            let mut s = self.shared();
            s.store.source = None;
            s.store.mode = VpnMode::Off;
            s.proxies.clear();
            s.latency.clear();
        }
        self.save()
    }

    /// Downloads (or re-reads) the nodes of `source`. An imported file that
    /// is gone keeps the cached nodes.
    async fn fetch_source(
        &self,
        source: VpnSource,
        sink: &dyn ProgressSink,
    ) -> AppResult<Vec<ProxyMap>> {
        let SourceStore { urls, file, .. } = self.shared().store.get(source).clone();
        if !urls.is_empty() {
            return self.fetch_subscription(&urls, sink).await;
        }
        if let Some(f) = file.filter(|f| std::path::Path::new(f).is_file()) {
            let bytes = tokio::fs::read(&f)
                .await
                .map_err(|e| AppError::new(ErrorCode::Io, e.to_string()).detail("path", f))?;
            return subscription::parse(&bytes);
        }
        let cached = self.inner.files.load_nodes(Some(source));
        if cached.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "no VPN subscription configured",
            ));
        }
        Ok(cached)
    }

    async fn hub_url(&self) -> String {
        match &self.inner.settings {
            Some(s) => s.get().await.map(|s| s.hub_url).unwrap_or_default(),
            None => String::new(),
        }
    }

    /// Downloads the first subscription mirror that yields nodes. Errors
    /// never include the URL (it carries the access token).
    async fn fetch_subscription(
        &self,
        urls: &[String],
        sink: &dyn ProgressSink,
    ) -> AppResult<Vec<ProxyMap>> {
        let mut last = None;
        for (i, raw) in urls.iter().enumerate() {
            sink.check_cancelled()?;
            let Ok(url) = Url::parse(raw) else { continue };
            let mut got = None;
            // Direct first with a short timeout (a blocked host must not
            // stall the UI), then whatever proxy lc-http has (VPN/manual/OS;
            // fails at once when there is none).
            for (route, timeout) in [
                (Route::Direct, DIRECT_FETCH_TIMEOUT),
                (Route::Proxied, PROXIED_FETCH_TIMEOUT),
            ] {
                match self.get_text(&url, route, timeout).await {
                    Ok(b) => {
                        got = Some(b);
                        break;
                    }
                    Err(e) => last = Some(e.detail("mirror", i.to_string())),
                }
            }
            let Some(body) = got else { continue };
            match subscription::parse(&body) {
                Ok(p) => return Ok(p),
                Err(e) => last = Some(e.detail("mirror", i.to_string())),
            }
        }
        Err(last.unwrap_or_else(|| AppError::new(ErrorCode::InvalidInput, "no subscription URL")))
    }

    async fn get_text(&self, url: &Url, route: Route, timeout: Duration) -> AppResult<Vec<u8>> {
        let net = |reason: String| {
            AppError::new(ErrorCode::Network, "subscription download failed")
                .detail("reason", reason)
        };
        let resp = self
            .inner
            .downloader
            .client(route)
            .get(url.clone())
            .timeout(timeout)
            .send()
            .await
            // `without_url` keeps the token out of the message.
            .map_err(|e| net(e.without_url().to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(net(format!("HTTP {}", status.as_u16())));
        }
        crate::response::read_limited(
            resp,
            MAX_SUBSCRIPTION,
            || net("subscription too large".into()),
            |e| net(e.without_url().to_string()),
        )
        .await
    }
}

impl Inner {
    fn shared(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn random_secret() -> AppResult<String> {
    let mut buf = [0u8; 24];
    getrandom::fill(&mut buf).map_err(|e| AppError::internal(format!("rng failed: {e}")))?;
    Ok(hex::encode(buf))
}

/// Same file, tolerant of `\\?\` prefixes and (on Windows) case.
fn same_exe(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (a, b) = (canon(a), canon(b));
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// Kills `pid` only if it still runs `exe` (guards against PID reuse).
fn kill_pid(pid: u32, exe: &Path) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let pid = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    let Some(p) = sys.process(pid) else { return };
    if p.exe().is_some_and(|actual| same_exe(actual, exe)) {
        p.kill();
    }
}

/// Pid file: `<pid>\n<exe path>`.
fn parse_pid_file(text: &str) -> Option<(u32, PathBuf)> {
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let exe = lines.next().map(str::trim).filter(|s| !s.is_empty())?;
    Some((pid, PathBuf::from(exe)))
}

/// Kills a mihomo left behind by a previous launcher run that crashed.
fn kill_stale(inner: &Inner) {
    let Ok(text) = std::fs::read_to_string(inner.files.pid()) else {
        return;
    };
    if let Some((pid, exe)) = parse_pid_file(&text) {
        if pid != std::process::id() {
            kill_pid(pid, &exe);
        }
    }
    let _ = std::fs::remove_file(inner.files.pid());
}

fn host_of(url: &str) -> Option<String> {
    Url::parse(url.trim())
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
}

/// Per-app rules: the launcher, Java and the user's programs through the VPN;
/// our hub/relay never through the tunnel.
async fn tun_rules(inner: &Inner) -> TunRules {
    let mut processes = apps::always_routed();
    processes.extend(inner.shared().store.apps.iter().map(|a| a.process.clone()));
    let mut bypass_hosts: Vec<String> = host_of(lc_types::settings::DEFAULT_HUB_URL)
        .into_iter()
        .collect();
    if let Some(s) = &inner.settings {
        if let Some(h) = s.get().await.ok().and_then(|s| host_of(&s.hub_url)) {
            bypass_hosts.push(h);
        }
    }
    TunRules {
        processes,
        bypass_hosts,
    }
}

/// What to start and how to recognise it later.
struct Spawned {
    child: Child,
    /// Executable of `child` (pid-reuse guard).
    exe: PathBuf,
    stop_flag: Option<PathBuf>,
}

fn spawn_plain(inner: &Inner, exe: &Path) -> AppResult<Spawned> {
    let log = std::fs::File::create(inner.files.log()).map_err(AppError::from)?;
    let log2 = log.try_clone().map_err(AppError::from)?;
    let mut cmd = Command::new(exe);
    cmd.arg("-d")
        .arg(&inner.files.dir)
        .arg("-f")
        .arg(inner.files.config())
        .current_dir(&inner.files.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2))
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let child = cmd.spawn().map_err(|e| {
        AppError::new(ErrorCode::Io, format!("cannot start mihomo: {e}"))
            .detail("path", exe.display().to_string())
    })?;
    Ok(Spawned {
        child,
        exe: exe.to_path_buf(),
        stop_flag: None,
    })
}

/// Starts our own executable elevated as the TUN helper (UAC prompt).
#[cfg(windows)]
fn spawn_elevated(inner: &Inner, exe: &Path) -> AppResult<Spawned> {
    let launcher = std::env::current_exe().map_err(AppError::from)?;
    let stop = inner.files.stop_flag();
    let _ = std::fs::remove_file(&stop);
    let h = tun::HelperArgs {
        mihomo: exe.to_path_buf(),
        dir: inner.files.dir.clone(),
        config: inner.files.config(),
        log: inner.files.log(),
        stop: stop.clone(),
        parent: std::process::id(),
    };
    let (ps, mut cmd) = tun::wrapper_command(&launcher, &h);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| {
        AppError::new(
            ErrorCode::PermissionDenied,
            format!("cannot request administrator rights: {e}"),
        )
    })?;
    Ok(Spawned {
        child,
        exe: ps,
        stop_flag: Some(stop),
    })
}

/// Error for a failed elevated start, from the wrapper's exit code.
#[cfg(windows)]
fn elevated_error(code: Option<i32>, fallback: AppError) -> AppError {
    match code {
        Some(tun::EXIT_DECLINED) => AppError::new(
            ErrorCode::PermissionDenied,
            "administrator rights were declined; the apps VPN mode needs them",
        )
        .detail("reason", "uacDeclined"),
        Some(tun::EXIT_SPAWN_FAILED) => AppError::new(
            ErrorCode::PermissionDenied,
            "cannot start the elevated VPN helper",
        )
        .detail("reason", "helperFailed"),
        Some(tun::EXIT_BAD_BINARY) => AppError::new(
            ErrorCode::HashMismatch,
            "mihomo executable does not match the pinned build; refusing to run it with \
             elevated rights",
        )
        .detail("reason", "binaryMismatch"),
        _ => fallback,
    }
}

/// Spawns mihomo on fresh ports and waits until its controller answers.
async fn spawn_mihomo(
    inner: &Inner,
    mode: VpnMode,
    sink: &dyn ProgressSink,
) -> AppResult<(Child, Running)> {
    let (proxies, selected, source) = {
        let s = inner.shared();
        (
            s.proxies.clone(),
            MihomoVpn::effective_selected(&s),
            s.store.source,
        )
    };
    if proxies.is_empty() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "VPN subscription has no nodes; refresh or import it first",
        ));
    }
    let tun = mode == VpnMode::Apps;
    if tun && !cfg!(any(windows, target_os = "linux")) {
        return Err(AppError::new(
            ErrorCode::NotImplemented,
            "apps VPN mode is not supported on this OS",
        ));
    }
    let exe = binary::ensure(
        &inner.paths,
        inner.downloader.as_ref(),
        sink,
        inner.binary_override.as_deref(),
    )
    .await?;
    #[cfg(target_os = "linux")]
    let exe = if tun {
        tun::linux_prepare(&exe, &inner.files.dir).await?
    } else {
        exe
    };
    let rules = if tun {
        Some(tun_rules(inner).await)
    } else {
        None
    };
    let elevated = tun && cfg!(windows);
    if elevated {
        // Fail with a clear error before the UAC prompt; the helper re-checks.
        let e = exe.clone();
        tokio::task::spawn_blocking(move || binary::verify_exe(&e))
            .await
            .map_err(|e| AppError::internal(format!("verify task failed: {e}")))??;
    }
    let secret = random_secret()?;
    // Ports may be grabbed between probing and mihomo's bind: retry (but
    // never ask for admin rights twice).
    let attempts = if elevated { 1 } else { 3 };
    let mut last_err = None;
    for _ in 0..attempts {
        let picked = ports::pick_free(2)?;
        let (mixed_port, controller_port) = (picked[0], picked[1]);
        let cfg = config::render(
            &RunParams {
                mixed_port,
                controller_port,
                secret: &secret,
            },
            &proxies,
            selected.as_deref(),
            rules.as_ref(),
        );
        write_private(&inner.files.config(), cfg.as_bytes())?;
        #[cfg(windows)]
        let spawned = if elevated {
            spawn_elevated(inner, &exe)?
        } else {
            spawn_plain(inner, &exe)?
        };
        #[cfg(not(windows))]
        let spawned = spawn_plain(inner, &exe)?;
        let Spawned {
            mut child,
            exe: child_exe,
            stop_flag,
        } = spawned;
        let pid = child.id();
        if let Some(pid) = pid {
            let line = format!("{pid}\n{}", child_exe.display());
            let _ = write_private(&inner.files.pid(), line.as_bytes());
        }
        let controller = Controller::new(
            inner.downloader.client(Route::Direct),
            controller_port,
            secret.clone(),
        )?;
        #[cfg(windows)]
        let timeout = if elevated {
            ELEVATED_READY_TIMEOUT
        } else {
            READY_TIMEOUT
        };
        #[cfg(not(windows))]
        let timeout = READY_TIMEOUT;
        match wait_ready(&mut child, &controller, timeout).await {
            Ok(()) => {
                tracing::info!(mixed_port, controller_port, tun, "mihomo started");
                return Ok((
                    child,
                    Running {
                        mixed_port,
                        controller_port,
                        secret: secret.clone(),
                        controller,
                        pid,
                        exe: child_exe,
                        tun,
                        stop_flag,
                        source,
                    },
                ));
            }
            Err(e) => {
                if let Some(flag) = &stop_flag {
                    tun::signal_stop(flag);
                }
                let code = child.try_wait().ok().flatten().and_then(|s| s.code());
                let _ = child.kill().await;
                let e = e.detail("log", inner.files.log().display().to_string());
                #[cfg(windows)]
                let e = if elevated { elevated_error(code, e) } else { e };
                #[cfg(not(windows))]
                let _ = code;
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| AppError::internal("mihomo did not start")))
}

async fn wait_ready(
    child: &mut Child,
    controller: &Controller,
    timeout: Duration,
) -> AppResult<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(AppError::internal("mihomo exited during startup")
                .detail("status", status.to_string()));
        }
        if controller.version().await.is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(AppError::internal("mihomo did not become ready in time"));
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Starts mihomo for `mode` and its supervisor. Caller holds `op`.
async fn start_locked(inner: &Arc<Inner>, mode: VpnMode, sink: &dyn ProgressSink) -> AppResult<()> {
    if inner.shared().run.is_some() {
        return Ok(());
    }
    kill_stale(inner);
    let (child, running) = match spawn_mihomo(inner, mode, sink).await {
        Ok(v) => v,
        Err(e) => {
            inner.shared().error = Some(e.clone());
            return Err(e);
        }
    };
    let proxy = running.proxy_url();
    let stop_flag = running.stop_flag.clone();
    let stop = CancelToken::new();
    {
        let mut s = inner.shared();
        s.run = Some(running);
        s.error = None;
    }
    inner.downloader.set_vpn_proxy(proxy);
    let handle = tokio::spawn(supervise(inner.clone(), child, stop_flag, stop.clone()));
    inner.shared().supervisor = Some((stop, handle));
    Ok(())
}

/// Re-renders the config for the running TUN mihomo (same ports and secret)
/// and makes it reload. Caller holds `op`.
async fn reload_locked(inner: &Inner) -> AppResult<()> {
    let Some(run) = inner.shared().run.clone() else {
        return Ok(());
    };
    let rules = if run.tun {
        Some(tun_rules(inner).await)
    } else {
        None
    };
    let (proxies, selected, source) = {
        let s = inner.shared();
        (
            s.proxies.clone(),
            MihomoVpn::effective_selected(&s),
            s.store.source,
        )
    };
    let cfg = config::render(
        &RunParams {
            mixed_port: run.mixed_port,
            controller_port: run.controller_port,
            secret: &run.secret,
        },
        &proxies,
        selected.as_deref(),
        rules.as_ref(),
    );
    write_private(&inner.files.config(), cfg.as_bytes())?;
    run.controller.reload(&inner.files.config()).await?;
    if let Some(node) = &selected {
        // Selection cached by mihomo may outlive a reload.
        let _ = run.controller.select(GROUP, node).await;
    }
    if let Some(r) = inner.shared().run.as_mut() {
        r.source = source;
    }
    Ok(())
}

/// Stops the supervisor (which kills mihomo). Caller holds `op`.
async fn stop_locked(inner: &Inner) {
    let sup = inner.shared().supervisor.take();
    if let Some((stop, handle)) = sup {
        stop.cancel();
        let _ = handle.await;
    }
    let had = inner.shared().run.take().is_some();
    if had {
        inner.downloader.set_vpn_proxy(None);
        tracing::info!("mihomo stopped");
    }
    let _ = std::fs::remove_file(inner.files.pid());
}

/// Stops our child; an elevated helper is asked via the stop flag first
/// (we cannot kill an elevated process).
async fn stop_child(child: &mut Child, stop_flag: Option<&Path>) {
    if let Some(flag) = stop_flag {
        tun::signal_stop(flag);
        if tokio::time::timeout(HELPER_STOP_GRACE, child.wait())
            .await
            .is_ok()
        {
            return;
        }
    }
    let _ = child.kill().await;
}

/// Watches the child; restarts it after a crash (bounded), kills it on stop.
/// An elevated helper is not restarted (that would prompt UAC again).
async fn supervise(
    inner: Arc<Inner>,
    mut child: Child,
    mut stop_flag: Option<PathBuf>,
    stop: CancelToken,
) {
    let mut crashes: Vec<Instant> = Vec::new();
    loop {
        tokio::select! {
            () = stop.cancelled() => {
                stop_child(&mut child, stop_flag.as_deref()).await;
                return;
            }
            status = child.wait() => {
                tracing::warn!(status = ?status.ok(), "mihomo exited unexpectedly");
            }
        }
        inner.shared().run = None;
        inner.downloader.set_vpn_proxy(None);
        if stop_flag.is_some() {
            let mut s = inner.shared();
            s.error = Some(AppError::internal(
                "elevated VPN helper stopped; VPN is off",
            ));
            s.supervisor = None;
            return;
        }
        crashes.retain(|t| t.elapsed() < RESTART_WINDOW);
        crashes.push(Instant::now());
        if crashes.len() > MAX_RESTARTS {
            inner.shared().error = Some(AppError::internal("mihomo keeps crashing; VPN stopped"));
            inner.shared().supervisor = None;
            return;
        }
        let delay = Duration::from_secs(1 << crashes.len().min(4));
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(delay) => {}
        }
        // Restart under `op` unless a stop arrives first.
        let _op = tokio::select! {
            () = stop.cancelled() => return,
            g = inner.op.lock() => g,
        };
        let mode = inner.shared().store.mode;
        match spawn_mihomo(&inner, mode, &NullSink::default()).await {
            Ok((c, running)) => {
                let proxy = running.proxy_url();
                stop_flag = running.stop_flag.clone();
                {
                    let mut s = inner.shared();
                    s.run = Some(running);
                    s.error = None;
                }
                inner.downloader.set_vpn_proxy(proxy);
                child = c;
            }
            Err(e) => {
                tracing::warn!(error = %e, "mihomo restart failed");
                let mut s = inner.shared();
                s.error = Some(e);
                s.supervisor = None;
                return;
            }
        }
    }
}

#[async_trait]
impl VpnController for MihomoVpn {
    async fn state(&self) -> AppResult<VpnState> {
        Ok(Self::build_state(&self.shared()))
    }

    async fn set_subscription(&self, urls: Vec<String>) -> AppResult<()> {
        let mut clean = Vec::with_capacity(urls.len());
        for (i, raw) in urls.iter().enumerate() {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            match Url::parse(raw) {
                Ok(u) if matches!(u.scheme(), "http" | "https") => clean.push(raw.to_owned()),
                _ => {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        "subscription URL must be http(s)",
                    )
                    .detail("index", i.to_string()))
                }
            }
        }
        let _op = self.inner.op.lock().await;
        if clean.is_empty() {
            return self.remove_own_locked().await;
        }
        let same = {
            let s = self.shared();
            s.store.own.urls == clean && s.store.own.file.is_none()
        };
        if !same {
            // Other subscription: cached nodes are stale until `refresh`.
            let _ = std::fs::remove_file(self.inner.files.nodes(VpnSource::Own));
            let mut s = self.shared();
            s.store.own = SourceStore {
                urls: clean,
                selected: s.store.own.selected.take(),
                ..SourceStore::default()
            };
            if s.store.source == Some(VpnSource::Own) {
                s.proxies.clear();
                s.latency.clear();
            }
        }
        self.switch_locked(VpnSource::Own).await
    }

    async fn import_subscription_file(&self, path: &str) -> AppResult<()> {
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| AppError::new(ErrorCode::NotFound, e.to_string()).detail("path", path))?;
        if meta.len() > MAX_SUBSCRIPTION as u64 {
            return Err(
                AppError::new(ErrorCode::InvalidInput, "subscription file too large")
                    .detail("path", path),
            );
        }
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| AppError::new(ErrorCode::Io, e.to_string()).detail("path", path))?;
        let proxies = subscription::parse(&bytes)?;
        {
            let mut s = self.shared();
            s.store.own.urls.clear();
            s.store.own.file = Some(path.to_owned());
        }
        self.install_proxies(VpnSource::Own, proxies).await?;
        let _op = self.inner.op.lock().await;
        self.switch_locked(VpnSource::Own).await
    }

    async fn refresh(&self, sink: &dyn ProgressSink) -> AppResult<()> {
        sink.progress(0, None, "vpn.subscription");
        let source = self.shared().store.source.ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput, "no VPN subscription configured")
        })?;
        let proxies = self.fetch_source(source, sink).await?;
        sink.check_cancelled()?;
        self.install_proxies(source, proxies).await?;
        // Pre-fetch the sidecar so enabling the VPN later is instant.
        binary::ensure(
            &self.inner.paths,
            self.inner.downloader.as_ref(),
            sink,
            self.inner.binary_override.as_deref(),
        )
        .await?;
        Ok(())
    }

    async fn set_mode(&self, mode: VpnMode) -> AppResult<VpnState> {
        let _op = self.inner.op.lock().await;
        if mode == VpnMode::Off {
            stop_locked(&self.inner).await;
            self.shared().error = None;
        } else {
            // TUN <-> mixed-port only needs a different mihomo process.
            let running_tun = self.shared().run.as_ref().map(|r| r.tun);
            let restart = running_tun.is_some_and(|t| t != (mode == VpnMode::Apps));
            if restart {
                stop_locked(&self.inner).await;
            }
            if let Err(e) = start_locked(&self.inner, mode, &NullSink::default()).await {
                // A failed apps mode (UAC/pkexec declined...) or a stopped
                // previous run leaves the VPN off.
                if restart || mode == VpnMode::Apps {
                    self.shared().store.mode = VpnMode::Off;
                    self.save()?;
                }
                return Err(e);
            }
        }
        {
            let mut s = self.shared();
            s.store.mode = mode;
            s.store.resume = None;
        }
        self.save()?;
        Ok(Self::build_state(&self.shared()))
    }

    async fn nodes(&self) -> AppResult<Vec<VpnNode>> {
        Ok(Self::build_nodes(&self.shared()))
    }

    async fn select(&self, node: &str) -> AppResult<()> {
        let controller = {
            let s = self.shared();
            if !s.proxies.iter().any(|p| name_of(p) == node) {
                return Err(AppError::new(ErrorCode::NotFound, "unknown VPN node"));
            }
            s.run.as_ref().map(|r| r.controller.clone())
        };
        if let Some(c) = controller {
            c.select(GROUP, node).await?;
        }
        {
            let mut s = self.shared();
            if let Some(src) = s.store.source {
                s.store.get_mut(src).selected = Some(node.to_owned());
            }
        }
        self.save()
    }

    async fn test_latency(&self) -> AppResult<Vec<VpnNode>> {
        let controller = self
            .shared()
            .run
            .as_ref()
            .map(|r| r.controller.clone())
            .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "VPN is not running"))?;
        let delays = controller.group_delay(GROUP).await?;
        let mut s = self.shared();
        s.latency = delays;
        Ok(Self::build_nodes(&s))
    }

    fn proxy_url(&self) -> Option<Url> {
        self.shared().run.as_ref().and_then(Running::proxy_url)
    }

    async fn mineded_access(&self, bearer: &str, provision: bool) -> AppResult<Option<VpnAccess>> {
        let url = mineded::endpoint(&self.hub_url().await)?;
        let client = self.inner.downloader.client(Route::Direct);
        let Some(access) = mineded::fetch(&client, &url, bearer, provision).await? else {
            return Ok(None);
        };
        let url = vec![access.subscription_url];
        let (changed, activate) = {
            let mut s = self.shared();
            let was = s.store.mineded.configured();
            if !was && s.store.own.urls == url && s.store.own.file.is_none() {
                // Pre-`VpnSource` state: MineDed access had overwritten the
                // single subscription, which migrated to `own`. Move it back.
                let own = std::mem::take(&mut s.store.own);
                s.store.mineded = own;
                let files = &self.inner.files;
                let _ =
                    std::fs::rename(files.nodes(VpnSource::Own), files.nodes(VpnSource::Mineded));
                if s.store.source == Some(VpnSource::Own) {
                    s.store.source = Some(VpnSource::Mineded);
                    if let Some(r) = s.run.as_mut() {
                        r.source = Some(VpnSource::Mineded);
                    }
                }
            }
            let changed = s.store.mineded.urls != url;
            if changed {
                s.store.mineded = SourceStore {
                    urls: url,
                    ..SourceStore::default()
                };
                if s.store.source == Some(VpnSource::Mineded) {
                    s.proxies.clear();
                    s.latency.clear();
                }
            }
            // Usage polling (`provision` false) must not take over a VPN the
            // user switched to their own subscription.
            let activate = provision || s.store.source.is_none();
            (changed, activate)
        };
        self.save()?;
        // Only a new subscription needs a download; polling must not disturb
        // a running VPN.
        if changed
            || self
                .inner
                .files
                .load_nodes(Some(VpnSource::Mineded))
                .is_empty()
        {
            let proxies = self
                .fetch_source(VpnSource::Mineded, &NullSink::default())
                .await?;
            self.install_proxies(VpnSource::Mineded, proxies).await?;
        }
        if activate {
            let _op = self.inner.op.lock().await;
            self.switch_locked(VpnSource::Mineded).await?;
        }
        Ok(Some(access.info))
    }

    async fn set_source(&self, source: VpnSource) -> AppResult<VpnState> {
        if !self.shared().store.get(source).configured() {
            return Err(
                AppError::new(ErrorCode::NotFound, "VPN subscription is not configured")
                    .detail("source", format!("{source:?}")),
            );
        }
        if self.inner.files.load_nodes(Some(source)).is_empty() {
            let proxies = self.fetch_source(source, &NullSink::default()).await?;
            self.install_proxies(source, proxies).await?;
        }
        let _op = self.inner.op.lock().await;
        self.switch_locked(source).await?;
        Ok(Self::build_state(&self.shared()))
    }

    async fn apps(&self) -> AppResult<Vec<VpnApp>> {
        Ok(self.shared().store.apps.clone())
    }

    async fn set_apps(&self, list: Vec<VpnApp>) -> AppResult<()> {
        let list = apps::normalize(list)?;
        let _op = self.inner.op.lock().await;
        let tun_running = {
            let mut s = self.shared();
            s.store.apps = list;
            s.run.as_ref().is_some_and(|r| r.tun)
        };
        self.save()?;
        if tun_running {
            reload_locked(&self.inner).await?;
        }
        Ok(())
    }

    async fn running_apps(&self) -> AppResult<Vec<VpnApp>> {
        tokio::task::spawn_blocking(apps::running)
            .await
            .map_err(|e| AppError::internal(format!("process scan failed: {e}")))
    }
}

impl LaunchContributor for MihomoVpn {
    fn name(&self) -> &'static str {
        "vpn"
    }

    /// `launcherAndGame` + running: points the game's Java at the mixed-port.
    /// Otherwise a no-op (a stopped VPN must not break the launch).
    fn contribute(
        &self,
        _instance: &Instance,
        _opts: &LaunchOpts,
        plan: &mut LaunchPlan,
    ) -> AppResult<()> {
        let s = self.shared();
        if s.store.mode != VpnMode::LauncherAndGame {
            return Ok(());
        }
        match &s.run {
            Some(r) => plan.jvm_args.extend(jvm_proxy_args(r.mixed_port)),
            None => tracing::warn!("VPN game mode is on but mihomo is not running"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_http::StubDownloader;

    fn tmp(tag: &str) -> Paths {
        let p = std::env::temp_dir().join(format!(
            "lc-vpn-svc-{tag}-{}-{}",
            std::process::id(),
            lc_types::TaskId::new().0
        ));
        Paths::new(p)
    }

    fn vpn(paths: &Paths) -> MihomoVpn {
        MihomoVpn::new(paths.clone(), Arc::new(StubDownloader))
    }

    #[tokio::test]
    async fn rejects_chunked_subscription_before_body_finishes() {
        let (url, server) = crate::response::tests::oversized_chunked(MAX_SUBSCRIPTION).await;
        let v = vpn(&tmp("oversized-subscription"));
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            v.get_text(&url, Route::Direct, Duration::from_secs(20)),
        )
        .await;
        server.abort();
        let err = result
            .expect("must reject without waiting for EOF")
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Network);
        assert_eq!(
            err.details.as_ref().unwrap()["reason"],
            "subscription too large"
        );
        assert!(!format!("{err:?}").contains("secret-subscription"));
    }

    #[tokio::test]
    async fn import_select_and_persist_without_running() {
        let paths = tmp("import");
        let dir = paths.root().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sub.txt");
        std::fs::write(
            &file,
            "trojan://pw@a.example:443#A\nvless://u@b.example:443?security=tls#B\n",
        )
        .unwrap();
        let v = vpn(&paths);
        assert!(v.nodes().await.unwrap().is_empty());
        v.import_subscription_file(file.to_str().unwrap())
            .await
            .unwrap();
        let nodes = v.nodes().await.unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(nodes[0].selected);
        assert_eq!(nodes[1].protocol, "vless");
        v.select("B").await.unwrap();
        assert_eq!(
            v.select("nope").await.unwrap_err().code,
            ErrorCode::NotFound
        );
        let st = v.state().await.unwrap();
        assert_eq!(st.node.as_deref(), Some("B"));
        assert!(st.subscription_updated.is_some());
        assert!(!st.running);
        assert!(v.proxy_url().is_none());
        assert_eq!(
            v.test_latency().await.unwrap_err().code,
            ErrorCode::InvalidInput
        );

        // A new instance sees the persisted subscription and selection.
        let again = vpn(&paths);
        let st = again.state().await.unwrap();
        assert_eq!(st.node.as_deref(), Some("B"));
        assert_eq!(again.nodes().await.unwrap().len(), 2);

        // URL subscription replaces the file one; bad URLs are rejected.
        again
            .set_subscription(vec!["https://sub.example/x?token=t".into(), " ".into()])
            .await
            .unwrap();
        let err = again
            .set_subscription(vec!["file:///etc/passwd".into()])
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(!format!("{err:?}").contains("passwd"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn enabling_without_nodes_fails_and_stays_off() {
        let paths = tmp("empty");
        let v = vpn(&paths);
        let err = v.set_mode(VpnMode::LauncherOnly).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        let st = v.state().await.unwrap();
        assert_eq!(st.mode, VpnMode::Off);
        assert!(st.error.is_some());
        assert_eq!(v.set_mode(VpnMode::Off).await.unwrap().mode, VpnMode::Off);
        v.restore().await.unwrap();
        let _ = std::fs::remove_dir_all(paths.root());
    }

    #[tokio::test]
    async fn apps_persist_and_failed_apps_mode_falls_back_off() {
        let paths = tmp("apps");
        let v = vpn(&paths);
        assert!(v.apps().await.unwrap().is_empty());
        let app = VpnApp {
            name: "Telegram".into(),
            process: "Telegram.exe".into(),
            path: None,
        };
        v.set_apps(vec![app.clone(), app.clone()]).await.unwrap();
        let bad = VpnApp {
            process: "x,DIRECT".into(),
            ..app.clone()
        };
        assert_eq!(
            v.set_apps(vec![bad]).await.unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(vpn(&paths).apps().await.unwrap(), vec![app]);

        // No nodes: apps mode fails before any elevation and stays off.
        v.shared().store.mode = VpnMode::LauncherOnly;
        assert!(v.set_mode(VpnMode::Apps).await.is_err());
        assert_eq!(v.state().await.unwrap().mode, VpnMode::Off);
        assert_eq!(vpn(&paths).state().await.unwrap().mode, VpnMode::Off);

        // Saved apps mode is not elevated at startup: off, remembered.
        v.shared().store.mode = VpnMode::Apps;
        v.save().unwrap();
        let again = vpn(&paths);
        again.restore().await.unwrap();
        let st = again.state().await.unwrap();
        assert_eq!((st.mode, st.running, st.error), (VpnMode::Off, false, None));
        let stored = Files::new(&paths).load_state();
        assert_eq!(
            (stored.mode, stored.resume),
            (VpnMode::Off, Some(VpnMode::Apps))
        );
        again.set_mode(VpnMode::Off).await.unwrap();
        assert_eq!(Files::new(&paths).load_state().resume, None);
        let _ = std::fs::remove_dir_all(paths.root());
    }

    #[tokio::test]
    async fn mineded_access_sets_hidden_subscription_once() {
        use lc_http::{DownloaderConfig, HttpDownloader, SystemProxyMode};
        use lc_runtime::MemorySettingsStore;

        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h = hits.clone();
        let app = axum::Router::new().route(
            "/sub/tok",
            axum::routing::get(move || {
                h.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { "trojan://pw@n.example:443#MineDed\n" }
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let sub = format!("http://127.0.0.1:{port}/sub/tok");
        let (hub, _calls) = crate::mineded::tests::spawn_hub(sub.clone()).await;

        let settings = Arc::new(MemorySettingsStore::default());
        let mut s = settings.get().await.unwrap();
        s.hub_url = hub;
        settings.set(s).await.unwrap();
        let d = HttpDownloader::new(DownloaderConfig {
            system_proxy: SystemProxyMode::Off,
            attempts_per_url: 1,
            ..DownloaderConfig::default()
        })
        .unwrap();
        let paths = tmp("mineded");
        let v = MihomoVpn::new(paths.clone(), Arc::new(d))
            .with_binary(std::env::current_exe().unwrap())
            .with_settings(settings);

        assert!(v.mineded_access("new", false).await.unwrap().is_none());
        let a = v.mineded_access("good", false).await.unwrap().unwrap();
        assert_eq!(a.quota_bytes, 274877906944);
        let nodes = v.nodes().await.unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "MineDed");
        // Polling usage again does not re-download the subscription.
        v.mineded_access("good", true).await.unwrap().unwrap();
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        // The URL never reaches the UI-facing state.
        let st = v.state().await.unwrap();
        assert!(!format!("{st:?}").contains("sub/tok"));
        let err = v.mineded_access("secret-xyz", false).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::AuthExpired);
        assert!(!format!("{err:?}").contains("secret-xyz"));
        let _ = std::fs::remove_dir_all(paths.root());
    }

    #[tokio::test]
    async fn mineded_and_own_subscriptions_are_separate() {
        use lc_http::{DownloaderConfig, HttpDownloader, SystemProxyMode};
        use lc_runtime::MemorySettingsStore;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let app = axum::Router::new()
            .route(
                "/sub/tok",
                axum::routing::get(move || {
                    h.fetch_add(1, Ordering::SeqCst);
                    async { "trojan://pw@n.example:443#MineDed\n" }
                }),
            )
            .route(
                "/own",
                axum::routing::get(|| async {
                    "trojan://pw@a.example:443#A\ntrojan://pw@b.example:443#B\n"
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let sub = format!("http://127.0.0.1:{port}/sub/tok");
        let own = format!("http://127.0.0.1:{port}/own");
        let (hub, _calls) = crate::mineded::tests::spawn_hub(sub.clone()).await;
        let settings = Arc::new(MemorySettingsStore::default());
        let mut s = settings.get().await.unwrap();
        s.hub_url = hub;
        settings.set(s).await.unwrap();
        let d: Arc<dyn Downloader> = Arc::new(
            HttpDownloader::new(DownloaderConfig {
                system_proxy: SystemProxyMode::Off,
                attempts_per_url: 1,
                ..DownloaderConfig::default()
            })
            .unwrap(),
        );
        let paths = tmp("sources");
        // Pre-VpnSource state where MineDed had overwritten the subscription.
        std::fs::create_dir_all(paths.mihomo()).unwrap();
        std::fs::write(
            paths.mihomo().join("state.json"),
            serde_json::to_vec(&serde_json::json!({"urls": [sub], "mode": "off"})).unwrap(),
        )
        .unwrap();
        std::fs::write(
            paths.mihomo().join("nodes.json"),
            serde_json::to_vec(&vec![subscription::parse_link(
                "trojan://pw@n.example:443#MineDed",
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap();
        let mk = || {
            MihomoVpn::new(paths.clone(), d.clone())
                .with_binary(std::env::current_exe().unwrap())
                .with_settings(settings.clone())
        };
        let v = mk();
        let st = v.state().await.unwrap();
        assert_eq!(
            (st.source, st.has_own, st.has_mineded),
            (Some(VpnSource::Own), Some(true), Some(false))
        );
        assert_eq!(
            v.set_source(VpnSource::Mineded).await.unwrap_err().code,
            ErrorCode::NotFound
        );

        // Access with the same URL moves it back to MineDed, no download.
        v.mineded_access("good", false).await.unwrap().unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        let st = v.state().await.unwrap();
        assert_eq!(
            (st.source, st.has_own, st.has_mineded),
            (Some(VpnSource::Mineded), Some(false), Some(true))
        );
        assert_eq!(v.nodes().await.unwrap()[0].name, "MineDed");

        // Adding an own subscription keeps MineDed and makes own active.
        v.set_subscription(vec![own.clone()]).await.unwrap();
        v.refresh(&NullSink::default()).await.unwrap();
        let st = v.state().await.unwrap();
        assert_eq!(
            (st.source, st.has_own, st.has_mineded),
            (Some(VpnSource::Own), Some(true), Some(true))
        );
        assert_eq!(v.nodes().await.unwrap().len(), 2);
        v.select("B").await.unwrap();
        // Setting the same URL again changes nothing.
        v.set_subscription(vec![own.clone()]).await.unwrap();
        assert_eq!(v.nodes().await.unwrap().len(), 2);

        // Usage polling does not take over the own subscription.
        v.mineded_access("good", false).await.unwrap().unwrap();
        assert_eq!(v.state().await.unwrap().source, Some(VpnSource::Own));
        assert_eq!(hits.load(Ordering::SeqCst), 0);

        // Switching keeps per-source selection; persisted across restarts.
        let st = v.set_source(VpnSource::Mineded).await.unwrap();
        assert_eq!(st.node.as_deref(), Some("MineDed"));
        let st = v.set_source(VpnSource::Own).await.unwrap();
        assert_eq!(st.node.as_deref(), Some("B"));
        let again = mk();
        assert_eq!(again.state().await.unwrap().node.as_deref(), Some("B"));
        // Explicit enabling (provision) activates MineDed.
        again.mineded_access("good", true).await.unwrap().unwrap();
        assert_eq!(
            again.state().await.unwrap().source,
            Some(VpnSource::Mineded)
        );
        assert_eq!(again.nodes().await.unwrap().len(), 1);

        // Removing the own subscription leaves MineDed alone.
        again.set_subscription(vec![]).await.unwrap();
        let st = again.state().await.unwrap();
        assert_eq!(
            (st.source, st.has_own, st.has_mineded),
            (Some(VpnSource::Mineded), Some(false), Some(true))
        );
        let _ = std::fs::remove_dir_all(paths.root());
    }

    #[test]
    fn pid_file_needs_exe_and_matches_paths() {
        assert_eq!(
            parse_pid_file("42\nC:/x/mihomo.exe\n"),
            Some((42, PathBuf::from("C:/x/mihomo.exe")))
        );
        assert_eq!(parse_pid_file("42\n"), None);
        assert_eq!(parse_pid_file("x\n/p"), None);
        let me = std::env::current_exe().unwrap();
        assert!(same_exe(&me, &me));
        assert!(!same_exe(&me, Path::new("/definitely/not/mihomo")));
        // A live process at another path is never killed.
        kill_pid(std::process::id(), Path::new("/definitely/not/mihomo"));
    }

    #[test]
    fn contributor_is_noop_unless_game_mode_running() {
        let paths = tmp("contrib");
        let v = vpn(&paths);
        let mut plan = LaunchPlan::default();
        let inst: Instance = serde_json::from_value(serde_json::json!({
            "id": lc_types::InstanceId::new(),
            "name": "x", "mc": "1.21.1",
            "loader": {"kind": "vanilla", "version": null},
            "java": {"path": null, "major": null},
            "memory": {"minMb": 512, "maxMb": 1024, "auto": false},
            "jvmArgs": [], "preset": null, "icon": null,
            "createdAt": "2026-01-01T00:00:00Z", "lastPlayed": null, "playtimeSec": 0
        }))
        .unwrap();
        v.contribute(&inst, &LaunchOpts::default(), &mut plan)
            .unwrap();
        assert!(plan.jvm_args.is_empty());
        v.shared().store.mode = VpnMode::LauncherAndGame;
        v.contribute(&inst, &LaunchOpts::default(), &mut plan)
            .unwrap();
        assert!(plan.jvm_args.is_empty(), "not running → no proxy args");
    }
}
