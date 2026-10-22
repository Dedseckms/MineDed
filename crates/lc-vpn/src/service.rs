//! `MihomoVpn`: the real `VpnController` + `LaunchContributor`.
//!
//! mihomo runs as a child process without admin rights or TUN, listening only
//! on loopback ports chosen at start. A supervisor task restarts it when it
//! crashes; `shutdown` / `kill_now` stop it when the app exits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lc_http::{Downloader, Route};
use lc_runtime::{CancelToken, LaunchContributor, LaunchPlan, NullSink, Paths, ProgressSink};
use lc_types::instance::{Instance, LaunchOpts};
use lc_types::vpn::{VpnMode, VpnNode, VpnState};
use lc_types::{AppError, AppResult, ErrorCode};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use url::Url;

use crate::binary;
use crate::config::{self, RunParams, GROUP};
use crate::contract::VpnController;
use crate::controller::Controller;
use crate::launch::jvm_proxy_args;
use crate::ports;
use crate::store::{write_private, Files, Stored};
use crate::subscription::{self, name_of, type_of, ProxyMap};

const READY_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SUBSCRIPTION: usize = 8 * 1024 * 1024;
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
    controller: Controller,
    pid: Option<u32>,
    exe: PathBuf,
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
        let proxies = files.load_nodes();
        Self {
            inner: Arc::new(Inner {
                paths,
                files,
                downloader,
                binary_override: None,
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

    /// Re-applies the persisted mode at app start (starts mihomo if the VPN
    /// was on). Failures are kept in `state().error` and returned.
    pub async fn restore(&self) -> AppResult<()> {
        let mode = self.shared().store.mode;
        if mode == VpnMode::Off || self.shared().proxies.is_empty() {
            return Ok(());
        }
        let _op = self.inner.op.lock().await;
        start_locked(&self.inner, &NullSink::default()).await
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
        if let Some(Running {
            pid: Some(pid),
            exe,
            ..
        }) = run
        {
            kill_pid(pid, &exe);
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
            .selected
            .clone()
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
            subscription_updated: s.store.updated.clone(),
            error: s.error.clone(),
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

    /// Stores a freshly parsed subscription and restarts mihomo if running.
    async fn install_proxies(&self, proxies: Vec<ProxyMap>) -> AppResult<()> {
        let _op = self.inner.op.lock().await;
        self.inner.files.save_nodes(&proxies)?;
        let running = {
            let mut s = self.shared();
            s.proxies = proxies;
            s.latency.clear();
            s.store.updated = Some(now_rfc3339());
            s.run.is_some()
        };
        self.save()?;
        if running {
            stop_locked(&self.inner).await;
            start_locked(&self.inner, &NullSink::default()).await?;
        }
        Ok(())
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
            // Direct first, then whatever proxy lc-http has (VPN/manual/OS).
            for route in [Route::Direct, Route::Proxied] {
                match self.get_text(&url, route).await {
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

    async fn get_text(&self, url: &Url, route: Route) -> AppResult<Vec<u8>> {
        let net = |reason: String| {
            AppError::new(ErrorCode::Network, "subscription download failed")
                .detail("reason", reason)
        };
        let resp = self
            .inner
            .downloader
            .client(route)
            .get(url.clone())
            .timeout(Duration::from_secs(30))
            .send()
            .await
            // `without_url` keeps the token out of the message.
            .map_err(|e| net(e.without_url().to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(net(format!("HTTP {}", status.as_u16())));
        }
        if resp
            .content_length()
            .is_some_and(|l| l > MAX_SUBSCRIPTION as u64)
        {
            return Err(net("subscription too large".into()));
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| net(e.without_url().to_string()))?;
        if body.len() > MAX_SUBSCRIPTION {
            return Err(net("subscription too large".into()));
        }
        Ok(body.to_vec())
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

/// Spawns mihomo on fresh ports and waits until its controller answers.
async fn spawn_mihomo(inner: &Inner, sink: &dyn ProgressSink) -> AppResult<(Child, Running)> {
    let (proxies, selected) = {
        let s = inner.shared();
        (s.proxies.clone(), MihomoVpn::effective_selected(&s))
    };
    if proxies.is_empty() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "VPN subscription has no nodes; refresh or import it first",
        ));
    }
    let exe = binary::ensure(
        &inner.paths,
        inner.downloader.as_ref(),
        sink,
        inner.binary_override.as_deref(),
    )
    .await?;
    let secret = random_secret()?;
    // Ports may be grabbed between probing and mihomo's bind: retry.
    let mut last_err = None;
    for _ in 0..3 {
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
        );
        write_private(&inner.files.config(), cfg.as_bytes())?;
        let log = std::fs::File::create(inner.files.log()).map_err(AppError::from)?;
        let log2 = log.try_clone().map_err(AppError::from)?;
        let mut cmd = Command::new(&exe);
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
        let mut child = cmd.spawn().map_err(|e| {
            AppError::new(ErrorCode::Io, format!("cannot start mihomo: {e}"))
                .detail("path", exe.display().to_string())
        })?;
        let pid = child.id();
        if let Some(pid) = pid {
            let line = format!("{pid}\n{}", exe.display());
            let _ = write_private(&inner.files.pid(), line.as_bytes());
        }
        let controller = Controller::new(
            inner.downloader.client(Route::Direct),
            controller_port,
            secret.clone(),
        )?;
        match wait_ready(&mut child, &controller).await {
            Ok(()) => {
                tracing::info!(mixed_port, controller_port, "mihomo started");
                return Ok((
                    child,
                    Running {
                        mixed_port,
                        controller,
                        pid,
                        exe: exe.clone(),
                    },
                ));
            }
            Err(e) => {
                let _ = child.kill().await;
                last_err = Some(e.detail("log", inner.files.log().display().to_string()));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| AppError::internal("mihomo did not start")))
}

async fn wait_ready(child: &mut Child, controller: &Controller) -> AppResult<()> {
    let deadline = Instant::now() + READY_TIMEOUT;
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

/// Starts mihomo and its supervisor. Caller holds `op`.
async fn start_locked(inner: &Arc<Inner>, sink: &dyn ProgressSink) -> AppResult<()> {
    if inner.shared().run.is_some() {
        return Ok(());
    }
    kill_stale(inner);
    let (child, running) = match spawn_mihomo(inner, sink).await {
        Ok(v) => v,
        Err(e) => {
            inner.shared().error = Some(e.clone());
            return Err(e);
        }
    };
    let proxy = running.proxy_url();
    let stop = CancelToken::new();
    {
        let mut s = inner.shared();
        s.run = Some(running);
        s.error = None;
    }
    inner.downloader.set_vpn_proxy(proxy);
    let handle = tokio::spawn(supervise(inner.clone(), child, stop.clone()));
    inner.shared().supervisor = Some((stop, handle));
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

/// Watches the child; restarts it after a crash (bounded), kills it on stop.
async fn supervise(inner: Arc<Inner>, mut child: Child, stop: CancelToken) {
    let mut crashes: Vec<Instant> = Vec::new();
    loop {
        tokio::select! {
            () = stop.cancelled() => {
                let _ = child.kill().await;
                return;
            }
            status = child.wait() => {
                tracing::warn!(status = ?status.ok(), "mihomo exited unexpectedly");
            }
        }
        inner.shared().run = None;
        inner.downloader.set_vpn_proxy(None);
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
        match spawn_mihomo(&inner, &NullSink::default()).await {
            Ok((c, running)) => {
                let proxy = running.proxy_url();
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
        {
            let mut s = self.shared();
            s.store.urls = clean;
            s.store.file = None;
        }
        self.save()
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
            s.store.urls.clear();
            s.store.file = Some(path.to_owned());
        }
        self.install_proxies(proxies).await
    }

    async fn refresh(&self, sink: &dyn ProgressSink) -> AppResult<()> {
        sink.progress(0, None, "vpn.subscription");
        let (urls, file) = {
            let s = self.shared();
            (s.store.urls.clone(), s.store.file.clone())
        };
        let proxies = if !urls.is_empty() {
            self.fetch_subscription(&urls, sink).await?
        } else if let Some(f) = file.filter(|f| std::path::Path::new(f).is_file()) {
            let bytes = tokio::fs::read(&f)
                .await
                .map_err(|e| AppError::new(ErrorCode::Io, e.to_string()).detail("path", f))?;
            subscription::parse(&bytes)?
        } else if !self.shared().proxies.is_empty() {
            // Imported file is gone: keep the cached nodes.
            self.shared().proxies.clone()
        } else {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "no VPN subscription configured",
            ));
        };
        sink.check_cancelled()?;
        self.install_proxies(proxies).await?;
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
            start_locked(&self.inner, &NullSink::default()).await?;
        }
        self.shared().store.mode = mode;
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
        self.shared().store.selected = Some(node.to_owned());
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
