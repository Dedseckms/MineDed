//! Transmit gate state machine (VAD and push-to-talk).
//!
//! VAD input is RNNoise's per-frame speech probability plus the frame level.
//! The gate opens only after `ONSET_FRAMES` confident frames (hysteresis:
//! open above `OPEN_PROB`, stay open above `HOLD_PROB`), stays open for at
//! least `MIN_ON` frames and closes only after `HANGOVER` frames without
//! speech. The onset delay is covered by the sender's pre-roll buffer, so no
//! syllable is lost and the gate never flickers per frame.

/// Probability that counts as confident speech (opening).
const OPEN_PROB: f32 = 0.6;
/// Probability that keeps an open gate alive.
const HOLD_PROB: f32 = 0.3;
/// A single frame this confident opens immediately.
const INSTANT_PROB: f32 = 0.92;
/// Frames below this level are never speech (digital silence, far noise).
const MIN_DB: f32 = -58.0;
/// Opening needs the frame this far above the tracked noise floor, holding
/// a bit less. RNNoise alone rates stationary noise (fans) as speech.
const OPEN_MARGIN_DB: f32 = 6.0;
const HOLD_MARGIN_DB: f32 = 3.0;
/// Floor seeding window (200 ms) and highest initial floor.
const SEED_FRAMES: u32 = 10;
const SEED_MAX_DB: f32 = -45.0;
/// Upward drift of the floor per 20 ms frame: 1 dB/s while open (speech is
/// not learned as noise), 12.5 dB/s while closed (new noise learned fast).
const FLOOR_RISE_OPEN_DB: f32 = 0.02;
const FLOOR_RISE_CLOSED_DB: f32 = 0.25;
/// Consecutive confident frames needed to open (40 ms).
pub const ONSET_FRAMES: u32 = 2;
/// Frames kept open after the last speech frame (400 ms).
pub const HANGOVER: u32 = 20;
/// Minimum open time (300 ms).
pub const MIN_ON: u32 = 15;
/// Frames encoded while closed and sent when the gate opens (240 ms).
pub const PREROLL_FRAMES: usize = 12;
/// Push-to-talk: pre-roll on press (60 ms) and tail after release (160 ms).
pub const PTT_PREROLL_FRAMES: usize = 3;
pub const PTT_TAIL: u32 = 8;

/// Result of one gate update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateStep {
    Closed,
    /// Opened on this frame: flush the pre-roll first.
    Opened,
    Open,
    /// Last open frame: send it faded out, then closed.
    Closing,
}

impl GateStep {
    pub fn is_open(self) -> bool {
        !matches!(self, GateStep::Closed)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Vad {
    open: bool,
    floor_db: f32,
    seeded: u32,
    onset: u32,
    hang: u32,
    on: u32,
}

impl Vad {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feeds one frame (`prob` from RNNoise, `db` = frame level in dBFS).
    pub fn update(&mut self, prob: f32, db: f32) -> GateStep {
        self.track_floor(db);
        let audible = db > MIN_DB;
        if !self.open {
            if audible && db > self.floor_db + OPEN_MARGIN_DB && prob >= OPEN_PROB {
                self.onset += 1;
            } else {
                self.onset = 0;
            }
            if self.onset >= ONSET_FRAMES
                || (audible && db > self.floor_db + OPEN_MARGIN_DB && prob >= INSTANT_PROB)
            {
                self.open = true;
                self.onset = 0;
                self.hang = HANGOVER;
                self.on = 1;
                return GateStep::Opened;
            }
            return GateStep::Closed;
        }
        self.on += 1;
        if audible && db > self.floor_db + HOLD_MARGIN_DB && prob >= HOLD_PROB {
            self.hang = HANGOVER;
        } else {
            self.hang = self.hang.saturating_sub(1);
        }
        if self.hang == 0 && self.on >= MIN_ON {
            self.open = false;
            return GateStep::Closing;
        }
        GateStep::Open
    }

    /// Noise floor: seeded from the quietest early frames, follows quieter
    /// frames at once and rises only slowly.
    fn track_floor(&mut self, db: f32) {
        if self.seeded < SEED_FRAMES {
            let seed = db.min(SEED_MAX_DB);
            self.floor_db = if self.seeded == 0 {
                seed
            } else {
                self.floor_db.min(seed)
            };
            self.seeded += 1;
        } else if db < self.floor_db {
            self.floor_db = db;
        } else {
            self.floor_db += if self.open {
                FLOOR_RISE_OPEN_DB
            } else {
                FLOOR_RISE_CLOSED_DB
            };
        }
    }
}

/// Push-to-talk gate with a short release tail.
#[derive(Debug, Clone, Default)]
pub struct PttGate {
    open: bool,
    tail: u32,
}

impl PttGate {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn update(&mut self, pressed: bool) -> GateStep {
        if pressed {
            self.tail = PTT_TAIL;
            if !self.open {
                self.open = true;
                return GateStep::Opened;
            }
            return GateStep::Open;
        }
        if !self.open {
            return GateStep::Closed;
        }
        self.tail = self.tail.saturating_sub(1);
        if self.tail == 0 {
            self.open = false;
            return GateStep::Closing;
        }
        GateStep::Open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEECH: (f32, f32) = (0.95, -25.0);
    const QUIET: (f32, f32) = (0.02, -70.0);

    fn run(vad: &mut Vad, f: (f32, f32), n: usize) -> Vec<GateStep> {
        (0..n).map(|_| vad.update(f.0, f.1)).collect()
    }

    #[test]
    fn hangover_timing() {
        let mut vad = Vad::new();
        assert!(run(&mut vad, QUIET, 50)
            .iter()
            .all(|s| *s == GateStep::Closed));
        assert_eq!(vad.update(SPEECH.0, SPEECH.1), GateStep::Opened);
        run(&mut vad, SPEECH, 30);
        let tail = run(&mut vad, QUIET, HANGOVER as usize + 2);
        // Open for exactly the hangover, closing on its last frame.
        let open = tail.iter().filter(|s| **s == GateStep::Open).count();
        assert_eq!(open, HANGOVER as usize - 1);
        assert_eq!(tail[HANGOVER as usize - 1], GateStep::Closing);
        assert_eq!(tail[HANGOVER as usize], GateStep::Closed);
    }

    #[test]
    fn onset_needs_consecutive_confident_frames() {
        let mut vad = Vad::new();
        assert_eq!(vad.update(0.7, -30.0), GateStep::Closed);
        assert_eq!(vad.update(0.7, -30.0), GateStep::Opened);
        // Confident but inaudible frames never open.
        let mut vad = Vad::new();
        assert!(run(&mut vad, (0.99, -80.0), 20)
            .iter()
            .all(|s| *s == GateStep::Closed));
    }

    #[test]
    fn short_blip_respects_min_on_time() {
        let mut vad = Vad::new();
        vad.update(SPEECH.0, SPEECH.1);
        let steps = run(&mut vad, QUIET, 40);
        let open = steps.iter().take_while(|s| s.is_open()).count();
        assert!(open + 1 >= MIN_ON as usize, "{open}");
    }

    #[test]
    fn flickering_probability_does_not_toggle() {
        // Noise-like probabilities alternating around the thresholds: either
        // never opens, or once opened stays open (no per-frame toggling).
        let mut vad = Vad::new();
        let mut transitions = 0;
        let mut prev = false;
        for i in 0..500 {
            let p = if i % 2 == 0 { 0.65 } else { 0.35 };
            let open = vad.update(p, -40.0).is_open();
            transitions += usize::from(open != prev);
            prev = open;
        }
        assert_eq!(transitions, 0);
    }

    #[test]
    fn ptt_tail() {
        let mut g = PttGate::default();
        assert_eq!(g.update(false), GateStep::Closed);
        assert_eq!(g.update(true), GateStep::Opened);
        assert_eq!(g.update(true), GateStep::Open);
        let tail: Vec<_> = (0..PTT_TAIL + 1).map(|_| g.update(false)).collect();
        assert_eq!(tail[PTT_TAIL as usize - 1], GateStep::Closing);
        assert_eq!(tail[PTT_TAIL as usize], GateStep::Closed);
    }
}
