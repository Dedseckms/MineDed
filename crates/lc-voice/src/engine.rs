//! `VoiceEngine` on cpal + Opus over `DatagramTransport`.
//!
//! Threads/tasks while active (voice started or mic test on):
//! - `voice-audio` thread owning the cpal streams; the output callback pulls
//!   mixed audio from [`Mixer`] (decoding happens there, clocked by the device);
//! - a tokio tick task (10 ms): capture → [`TxPipeline`] → `send`, speaking
//!   indicators;
//! - a tokio rx task: `recv` → [`Mixer::push_datagram`];
//! - `voice-ptt` thread polling the PTT key (PushToTalk mode only).
//!
//! Nothing runs while voice is stopped and the mic test is off.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lc_p2p::DatagramTransport;
use lc_runtime::Paths;
use lc_types::events::VoiceSpeakingPayload;
use lc_types::voice::{AudioDevice, VoiceMode, VoiceSettings};
use lc_types::{AppError, AppResult, ErrorCode};
use tokio::task::JoinHandle;

use crate::audio::{list_devices, AudioIo, CaptureQueue, IoConfig};
use crate::contract::{VoiceEngine, VoiceObserver};
use crate::mixer::Mixer;
use crate::ptt::{self, PttError, PttWatcher};
use crate::settings::{sanitize, SettingsFile};
use crate::speaking::{SpeakingTracker, MIN_INTERVAL};
use crate::tx::{Gate, TxControl, TxPipeline};

/// `peer_id` used in `voice://speaking` for the local user.
pub const SELF_PEER_ID: &str = "self";

const TICK: Duration = Duration::from_millis(10);
const REPORT: Duration = Duration::from_millis(50);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Per-frame state read by the tick task.
#[derive(Debug, Clone, Copy)]
struct Live {
    gate: Gate,
    self_muted: bool,
    input_gain: f32,
    noise_suppression: bool,
    mic_test: bool,
}

/// State shared with the background tasks (no back-reference to the engine,
/// so dropping the engine stops everything).
struct Shared {
    observer: Arc<dyn VoiceObserver>,
    mixer: Arc<Mutex<Mixer>>,
    capture: Arc<CaptureQueue>,
    live: Mutex<Live>,
    ptt: Mutex<Option<PttWatcher>>,
    tracker: Mutex<SpeakingTracker>,
    speaker: u8,
    seq_start: u16,
}

impl Shared {
    fn control(&self) -> (TxControl, bool) {
        let live = *lock(&self.live);
        let ptt_pressed = lock(&self.ptt).as_ref().is_some_and(PttWatcher::pressed);
        (
            TxControl {
                gate: live.gate,
                ptt_pressed,
                self_muted: live.self_muted,
                input_gain: live.input_gain,
                noise_suppression: live.noise_suppression,
            },
            live.mic_test,
        )
    }

    fn emit(&self, payloads: Vec<VoiceSpeakingPayload>) {
        for p in payloads {
            self.observer.on_speaking(p);
        }
    }

    fn report(&self, self_speaking: bool, self_level: f32, mic_test: bool, now: Instant) {
        let activity = lock(&self.mixer).activity();
        let mut out = Vec::new();
        {
            let mut t = lock(&self.tracker);
            if mic_test {
                // Level meter for the settings screen, even with a closed gate.
                if let Some(mut p) = t.update(SELF_PEER_ID, true, self_level, now) {
                    p.speaking = self_speaking;
                    out.push(p);
                }
            } else if let Some(p) = t.update(SELF_PEER_ID, self_speaking, self_level, now) {
                out.push(p);
            }
            let mut known = t.known();
            for a in &activity {
                known.retain(|k| k != &a.peer_id);
                out.extend(t.update(&a.peer_id, a.speaking, a.level, now));
            }
            for gone in known.iter().filter(|k| *k != SELF_PEER_ID) {
                out.extend(t.update(gone, false, 0.0, now));
            }
        }
        self.emit(out);
    }
}

struct Run {
    io: Option<AudioIo>,
    tick: Option<JoinHandle<()>>,
    rx: Option<JoinHandle<()>>,
    transport: Option<Arc<dyn DatagramTransport>>,
    mic_test: bool,
}

impl Run {
    fn abort_tasks(&mut self) {
        if let Some(t) = self.tick.take() {
            t.abort();
        }
        if let Some(t) = self.rx.take() {
            t.abort();
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        self.abort_tasks();
    }
}

/// Real voice engine. Construct once; see the module docs for its threads.
pub struct CpalVoiceEngine {
    file: SettingsFile,
    settings: Mutex<VoiceSettings>,
    shared: Arc<Shared>,
    run: tokio::sync::Mutex<Run>,
}

impl CpalVoiceEngine {
    /// Loads `<root>/voice.json` (defaults if missing). Cheap: no device or
    /// thread is touched until `start` / `mic_test`.
    pub fn new(paths: &Paths, observer: Arc<dyn VoiceObserver>) -> Self {
        let file = SettingsFile::new(paths);
        let settings = file.load();
        let mut mixer = Mixer::new();
        mixer.set_output_volume(settings.output_volume);
        mixer.set_peer_volumes(&settings.peer_volumes);
        let rnd = RandomState::new().hash_one(std::process::id());
        let shared = Shared {
            observer,
            mixer: Arc::new(Mutex::new(mixer)),
            capture: Arc::new(CaptureQueue::new()),
            live: Mutex::new(Live {
                gate: Gate::Off,
                self_muted: false,
                input_gain: settings.input_gain,
                noise_suppression: settings.noise_suppression,
                mic_test: false,
            }),
            ptt: Mutex::new(None),
            tracker: Mutex::new(SpeakingTracker::new()),
            speaker: rnd as u8,
            seq_start: (rnd >> 8) as u16,
        };
        Self {
            file,
            settings: Mutex::new(settings),
            shared: Arc::new(shared),
            run: tokio::sync::Mutex::new(Run {
                io: None,
                tick: None,
                rx: None,
                transport: None,
                mic_test: false,
            }),
        }
    }

    fn current(&self) -> VoiceSettings {
        lock(&self.settings).clone()
    }

    /// Starts/stops the PTT poller and sets the effective gate.
    fn apply_gate(&self, s: &VoiceSettings, active: bool) -> Result<(), PttError> {
        let mut result = Ok(());
        let gate = match s.mode {
            VoiceMode::Off => Gate::Off,
            VoiceMode::Vad => Gate::Vad,
            VoiceMode::PushToTalk => {
                let mut ptt = lock(&self.shared.ptt);
                *ptt = None;
                let started = if active {
                    PttWatcher::start(s.ptt_key.as_deref()).map(Some)
                } else {
                    check_ptt(s.ptt_key.as_deref()).map(|()| None)
                };
                match started {
                    Ok(w) => {
                        *ptt = w;
                        Gate::PushToTalk
                    }
                    Err(e) => {
                        result = Err(e);
                        Gate::Vad
                    }
                }
            }
        };
        if s.mode != VoiceMode::PushToTalk {
            *lock(&self.shared.ptt) = None;
        }
        let mut live = lock(&self.shared.live);
        live.gate = gate;
        live.input_gain = s.input_gain;
        live.noise_suppression = s.noise_suppression;
        result
    }

    /// Brings threads/devices in line with the wanted state. Returns a
    /// non-fatal warning (listen-only, PTT fallback) when something degraded.
    async fn reconcile(&self, run: &mut Run) -> AppResult<Option<AppError>> {
        let s = self.current();
        let active = run.transport.is_some() || run.mic_test;
        lock(&self.shared.live).mic_test = run.mic_test;
        if !active {
            run.abort_tasks();
            run.io = None;
            *lock(&self.shared.ptt) = None;
            lock(&self.shared.mixer).clear_streams();
            let resets = lock(&self.shared.tracker).reset();
            self.shared.emit(resets);
            return Ok(None);
        }
        let mut warning = self.apply_gate(&s, true).err().map(ptt_error);

        let want = IoConfig {
            input: (s.mode != VoiceMode::Off || run.mic_test).then(|| s.input_device.clone()),
            output: s.output_device.clone(),
        };
        if run.io.as_ref().map(|io| &io.config) != Some(&want) {
            run.io = None;
            let (capture, mixer) = (self.shared.capture.clone(), self.shared.mixer.clone());
            let io = tokio::task::spawn_blocking(move || AudioIo::open(want, capture, mixer))
                .await
                .map_err(|e| AppError::internal(format!("audio open: {e}")))??;
            run.io = Some(io);
        }
        if let Some(e) = run.io.as_ref().and_then(|io| io.input_error.clone()) {
            warning = Some(e.detail("listenOnly", "true"));
        }
        if run.tick.is_none() {
            run.tick = Some(tokio::spawn(tick_loop(
                self.shared.clone(),
                run.transport.clone(),
            )));
        }
        Ok(warning)
    }

    async fn persist(&self, s: VoiceSettings) -> AppResult<()> {
        self.file.save(&s).await?;
        *lock(&self.settings) = s;
        Ok(())
    }
}

fn check_ptt(key: Option<&str>) -> Result<(), PttError> {
    let key = key.ok_or(PttError::NoKey)?;
    ptt::parse_key(key).ok_or_else(|| PttError::UnknownKey(key.to_owned()))?;
    if ptt::is_wayland() {
        return Err(PttError::Wayland);
    }
    Ok(())
}

fn ptt_error(e: PttError) -> AppError {
    let code = match e {
        PttError::NoKey | PttError::UnknownKey(_) => ErrorCode::InvalidInput,
        PttError::Wayland | PttError::Unavailable => ErrorCode::PermissionDenied,
    };
    let mut err = AppError::new(code, "push-to-talk unavailable, using voice activation")
        .detail("fallback", "vad")
        .detail("reason", e.reason());
    if let PttError::UnknownKey(k) = e {
        err = err.detail("key", k);
    }
    err
}

fn warn_to_result(w: Option<AppError>) -> AppResult<()> {
    match w {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Capture → encode → send, plus speaking reports.
async fn tick_loop(shared: Arc<Shared>, transport: Option<Arc<dyn DatagramTransport>>) {
    let mut iv = tokio::time::interval(TICK);
    iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pipeline: Option<TxPipeline> = None;
    let mut buf = Vec::with_capacity(4096);
    let mut last_report = Instant::now();
    let (mut speaking, mut level) = (false, 0f32);
    let mut published: Option<(bool, bool)> = None;
    let mut last_publish = Instant::now()
        .checked_sub(MIN_INTERVAL)
        .unwrap_or_else(Instant::now);
    loop {
        iv.tick().await;
        buf.clear();
        shared.capture.drain_into(&mut buf);
        let rate = shared.capture.rate();
        let pipe = match pipeline.as_mut() {
            Some(p) => {
                p.set_input_rate(rate);
                p
            }
            None => match TxPipeline::new(rate, shared.speaker, shared.seq_start) {
                Ok(p) => pipeline.insert(p),
                Err(e) => {
                    tracing::warn!(error = %e, "voice pipeline");
                    return;
                }
            },
        };
        pipe.push_samples(&buf);
        let (ctl, mic_test) = shared.control();
        while let Some(frame) = pipe.next_frame(&ctl) {
            if mic_test {
                lock(&shared.mixer).push_loopback(&frame.pcm);
            }
            if let (Some(t), Some(d)) = (&transport, frame.datagram.as_deref()) {
                if d.len() <= t.max_datagram_size() {
                    let _ = t.send(None, d).await;
                }
            }
            speaking |= frame.speaking;
            level = level.max(frame.level);
        }
        let now = Instant::now();
        if now.duration_since(last_report) >= REPORT.min(MIN_INTERVAL) {
            shared.report(speaking, level, mic_test, now);
            if let Some(t) = &transport {
                // Room-wide voice state (`ControlMsg::VoiceState`), throttled.
                let state = (speaking, ctl.self_muted);
                if published != Some(state) && now.duration_since(last_publish) >= MIN_INTERVAL {
                    let _ = t.set_voice_state(state.0, state.1).await;
                    published = Some(state);
                    last_publish = now;
                }
            }
            last_report = now;
            speaking = false;
            level = 0.0;
        }
    }
}

/// Feeds received datagrams into the mixer until the room closes.
pub(crate) async fn rx_loop(transport: Arc<dyn DatagramTransport>, mixer: Arc<Mutex<Mixer>>) {
    while let Some((from, data)) = transport.recv().await {
        lock(&mixer).push_datagram(&from, &data);
    }
}

#[async_trait]
impl VoiceEngine for CpalVoiceEngine {
    async fn devices(&self) -> AppResult<Vec<AudioDevice>> {
        tokio::task::spawn_blocking(list_devices)
            .await
            .map_err(|e| AppError::internal(format!("devices: {e}")))?
    }

    async fn settings(&self) -> AppResult<VoiceSettings> {
        Ok(self.current())
    }

    /// Persists, then applies. A PTT problem (Wayland, bad key) is returned
    /// as an error with `details.fallback = "vad"` after VAD was applied.
    async fn set_settings(&self, settings: VoiceSettings) -> AppResult<()> {
        let s = sanitize(settings)?;
        self.persist(s.clone()).await?;
        {
            let mut m = lock(&self.shared.mixer);
            m.set_output_volume(s.output_volume);
            m.set_peer_volumes(&s.peer_volumes);
        }
        let mut run = self.run.lock().await;
        let active = run.transport.is_some() || run.mic_test;
        if active {
            warn_to_result(self.reconcile(&mut run).await?)
        } else {
            self.apply_gate(&s, false).map_err(ptt_error)
        }
    }

    /// Errors after a successful start carry `details.listenOnly` (no
    /// microphone) or `details.fallback` (PTT → VAD); voice keeps running.
    async fn start(&self, transport: Arc<dyn DatagramTransport>) -> AppResult<()> {
        let mut run = self.run.lock().await;
        run.abort_tasks();
        lock(&self.shared.mixer).clear_streams();
        run.rx = Some(tokio::spawn(rx_loop(
            transport.clone(),
            self.shared.mixer.clone(),
        )));
        run.transport = Some(transport);
        match self.reconcile(&mut run).await {
            Ok(w) => warn_to_result(w),
            Err(e) => {
                run.transport = None;
                run.abort_tasks();
                let _ = self.reconcile(&mut run).await;
                Err(e)
            }
        }
    }

    async fn stop(&self) -> AppResult<()> {
        let mut run = self.run.lock().await;
        if let Some(t) = run.transport.take() {
            let muted = lock(&self.shared.live).self_muted;
            let _ = t.set_voice_state(false, muted).await;
        }
        run.abort_tasks();
        lock(&self.shared.mixer).clear_streams();
        let resets = lock(&self.shared.tracker).reset();
        self.shared.emit(resets);
        self.reconcile(&mut run).await.map(|_| ())
    }

    async fn set_peer_muted(&self, peer_id: &str, muted: bool) -> AppResult<()> {
        lock(&self.shared.mixer).set_peer_muted(peer_id, muted);
        Ok(())
    }

    async fn set_peer_volume(&self, peer_id: &str, volume: f32) -> AppResult<()> {
        if !volume.is_finite() || peer_id.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidInput, "bad peer volume"));
        }
        let volume = volume.clamp(0.0, 2.0);
        let mut s = self.current();
        if (volume - 1.0).abs() < f32::EPSILON {
            s.peer_volumes.remove(peer_id);
        } else {
            s.peer_volumes.insert(peer_id.to_owned(), volume);
        }
        self.persist(s).await?;
        lock(&self.shared.mixer).set_peer_volume(peer_id, volume);
        Ok(())
    }

    async fn set_self_muted(&self, muted: bool) -> AppResult<()> {
        lock(&self.shared.live).self_muted = muted;
        Ok(())
    }

    async fn mic_test(&self, enabled: bool) -> AppResult<()> {
        let mut run = self.run.lock().await;
        run.mic_test = enabled;
        if !enabled {
            lock(&self.shared.mixer).clear_loopback();
        }
        match self.reconcile(&mut run).await {
            Ok(w) => warn_to_result(w),
            Err(e) => {
                run.mic_test = false;
                let _ = self.reconcile(&mut run).await;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NullObserver;
    impl VoiceObserver for NullObserver {
        fn on_speaking(&self, _payload: VoiceSpeakingPayload) {}
    }

    fn engine(tag: &str) -> (CpalVoiceEngine, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("lc-voice-engine-{tag}-{}", std::process::id()));
        (
            CpalVoiceEngine::new(&Paths::new(&dir), Arc::new(NullObserver)),
            dir,
        )
    }

    #[tokio::test]
    async fn settings_and_peer_volume_persist_without_devices() {
        let (e, dir) = engine("persist");
        let mut s = e.settings().await.unwrap();
        s.mode = VoiceMode::Vad;
        s.input_gain = 1.5;
        e.set_settings(s).await.unwrap();
        e.set_peer_volume("uuid-a", 0.4).await.unwrap();
        e.set_peer_muted("uuid-a", true).await.unwrap();
        e.set_self_muted(true).await.unwrap();
        assert!(e.set_peer_volume("uuid-a", f32::NAN).await.is_err());
        let (again, _) = engine("persist");
        let s = again.settings().await.unwrap();
        assert_eq!(s.input_gain, 1.5);
        assert_eq!(s.peer_volumes.get("uuid-a"), Some(&0.4));
        // Stop while idle is a no-op.
        e.stop().await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn bad_ptt_key_falls_back_to_vad() {
        let (e, dir) = engine("ptt");
        let mut s = e.settings().await.unwrap();
        s.mode = VoiceMode::PushToTalk;
        s.ptt_key = Some("NoSuchKey".into());
        let err = e.set_settings(s).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert_eq!(
            err.details.unwrap().get("fallback").map(String::as_str),
            Some("vad")
        );
        assert_eq!(lock(&e.shared.live).gate, Gate::Vad);
        // Settings are still saved as requested.
        assert_eq!(e.settings().await.unwrap().mode, VoiceMode::PushToTalk);
        let _ = std::fs::remove_dir_all(dir);
    }
}
