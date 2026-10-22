//! Energy VAD: frame RMS against a tracked noise floor, plus hangover so word
//! endings are not clipped.
//!
//! The floor is seeded from the quietest of the first frames, follows quieter
//! frames immediately and rises only slowly (1 dB/s), so a steady voice or
//! sound keeps the gate open for a long time while constant background noise
//! is still learned eventually.

use crate::dsp::{dbfs, rms};

/// Speech must exceed the noise floor by this much.
const MARGIN_DB: f32 = 12.0;
/// Absolute minimum threshold.
const MIN_THRESHOLD_DB: f32 = -50.0;
/// Frames kept open after speech stops (300 ms).
pub const HANGOVER: u32 = 15;
/// Floor seeding window (500 ms).
const SEED_FRAMES: u32 = 25;
/// Highest initial floor.
const SEED_MAX_DB: f32 = -40.0;
/// Upward drift of the floor per 20 ms frame (1 dB/s).
const FLOOR_RISE_DB: f32 = 0.02;

#[derive(Debug, Clone)]
pub struct Vad {
    floor_db: f32,
    seeded: u32,
    hang: u32,
}

impl Default for Vad {
    fn default() -> Self {
        Self {
            floor_db: 0.0,
            seeded: 0,
            hang: 0,
        }
    }
}

impl Vad {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one frame, returns whether the gate is open.
    pub fn update(&mut self, frame: &[f32]) -> bool {
        let db = dbfs(rms(frame));
        if self.seeded < SEED_FRAMES {
            // Capped so talking right from the start is not taken for noise.
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
            self.floor_db += FLOOR_RISE_DB;
        }
        let threshold = (self.floor_db + MARGIN_DB).max(MIN_THRESHOLD_DB);
        if db > threshold {
            self.hang = HANGOVER;
            true
        } else if self.hang > 0 {
            self.hang -= 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::FRAME;

    fn tone(amp: f32) -> Vec<f32> {
        (0..FRAME).map(|i| amp * (i as f32 * 0.07).sin()).collect()
    }

    #[test]
    fn silence_closed_speech_open_with_hangover() {
        let mut vad = Vad::new();
        let quiet = tone(0.0005);
        for _ in 0..50 {
            assert!(!vad.update(&quiet));
        }
        assert!(vad.update(&tone(0.2)));
        for _ in 0..HANGOVER {
            assert!(vad.update(&quiet));
        }
        assert!(!vad.update(&quiet));
    }

    #[test]
    fn steady_voice_keeps_gate_open() {
        let mut vad = Vad::new();
        for _ in 0..50 {
            vad.update(&tone(0.001));
        }
        // 20 s of steady loud sound (humming, singing) stays open.
        let open = (0..1000).filter(|_| vad.update(&tone(0.3))).count();
        assert_eq!(open, 1000);
    }

    #[test]
    fn talking_from_the_first_frame_opens() {
        let mut vad = Vad::new();
        let open = (0..100).filter(|_| vad.update(&tone(0.3))).count();
        assert_eq!(open, 100);
    }

    #[test]
    fn background_noise_from_start_is_learned() {
        let mut vad = Vad::new();
        let noise = tone(0.02); // ~-37 dBFS fan noise
        let open = (0..100).filter(|_| vad.update(&noise)).count();
        assert_eq!(open, 0);
        // Speech over the noise opens.
        assert!(vad.update(&tone(0.3)));
    }
}
