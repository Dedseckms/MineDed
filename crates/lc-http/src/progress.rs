//! Aggregated, throttled byte progress for one `fetch_many` call.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use lc_runtime::ProgressSink;
use lc_types::task::TaskEvent;

/// i18n key of the download stage.
pub const STAGE: &str = "stages.download";
const MIN_INTERVAL: Duration = Duration::from_millis(100);

struct Window {
    last_emit: Option<Instant>,
    /// Network bytes at the previous speed sample.
    sample_bytes: u64,
    sample_at: Instant,
    speed: Option<u64>,
}

pub(crate) struct Progress<'a> {
    sink: &'a dyn ProgressSink,
    total: Option<u64>,
    /// Bytes accounted for, including skipped files and resumed `.part`s.
    done: AtomicU64,
    /// Bytes actually received from the network (for speed).
    net: AtomicU64,
    window: Mutex<Window>,
}

impl<'a> Progress<'a> {
    pub fn new(sink: &'a dyn ProgressSink, total: Option<u64>) -> Self {
        Self {
            sink,
            total,
            done: AtomicU64::new(0),
            net: AtomicU64::new(0),
            window: Mutex::new(Window {
                last_emit: None,
                sample_bytes: 0,
                sample_at: Instant::now(),
                speed: None,
            }),
        }
    }

    /// Bytes received from the network.
    pub fn received(&self, n: u64) {
        self.net.fetch_add(n, Ordering::Relaxed);
        self.done.fetch_add(n, Ordering::Relaxed);
        self.emit(false);
    }

    /// Bytes already on disk (skipped file or resumed part).
    pub fn reused(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
        self.emit(false);
    }

    /// Takes back bytes that were counted but then discarded.
    pub fn rollback(&self, n: u64) {
        if n == 0 {
            return;
        }
        let mut cur = self.done.load(Ordering::Relaxed);
        loop {
            let next = cur.saturating_sub(n);
            match self
                .done
                .compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
    }

    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    /// Emits a `Progress` event at most every 100 ms unless `force` is set.
    pub fn emit(&self, force: bool) {
        let now = Instant::now();
        let mut w = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        if !force {
            if let Some(last) = w.last_emit {
                if now.duration_since(last) < MIN_INTERVAL {
                    return;
                }
            }
        }
        let net = self.net.load(Ordering::Relaxed);
        let dt = now.duration_since(w.sample_at);
        if dt >= MIN_INTERVAL {
            let bytes = net.saturating_sub(w.sample_bytes);
            // Truncation to whole bytes per second is intended.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let bps = (bytes as f64 / dt.as_secs_f64()) as u64;
            w.speed = Some(bps);
            w.sample_bytes = net;
            w.sample_at = now;
        }
        w.last_emit = Some(now);
        let speed = w.speed;
        drop(w);
        self.sink.emit(TaskEvent::Progress {
            id: self.sink.task_id(),
            done: self.done(),
            total: self.total,
            stage: STAGE.to_owned(),
            speed_bps: speed,
        });
    }
}
