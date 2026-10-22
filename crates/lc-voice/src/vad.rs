//! Energy VAD: frame RMS against a minimum-statistics noise floor, plus
//! hangover so word endings are not clipped.

use crate::dsp::{dbfs, rms};

/// Speech must exceed the noise floor by this much.
const MARGIN_DB: f32 = 12.0;
/// Absolute minimum threshold.
const MIN_THRESHOLD_DB: f32 = -50.0;
/// Frames kept open after speech stops (300 ms).
pub const HANGOVER: u32 = 15;
/// Noise floor = minimum frame level over the last `BLOCKS × BLOCK` frames (3 s).
const BLOCK: u32 = 50;
const BLOCKS: usize = 3;

#[derive(Debug, Clone)]
pub struct Vad {
    block_mins: [f32; BLOCKS],
    block_idx: usize,
    cur_min: f32,
    cur_len: u32,
    hang: u32,
}

impl Default for Vad {
    fn default() -> Self {
        Self {
            block_mins: [-100.0; BLOCKS],
            block_idx: 0,
            cur_min: f32::MAX,
            cur_len: 0,
            hang: 0,
        }
    }
}

impl Vad {
    pub fn new() -> Self {
        Self::default()
    }

    fn floor(&self) -> f32 {
        self.block_mins.iter().copied().fold(self.cur_min, f32::min)
    }

    /// Feeds one frame, returns whether the gate is open.
    pub fn update(&mut self, frame: &[f32]) -> bool {
        let db = dbfs(rms(frame));
        let threshold = (self.floor() + MARGIN_DB).max(MIN_THRESHOLD_DB);
        let speech = db > threshold;

        self.cur_min = self.cur_min.min(db);
        self.cur_len += 1;
        if self.cur_len >= BLOCK {
            self.block_mins[self.block_idx] = self.cur_min;
            self.block_idx = (self.block_idx + 1) % BLOCKS;
            self.cur_min = f32::MAX;
            self.cur_len = 0;
        }

        if speech {
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
    fn steady_noise_is_learned() {
        let mut vad = Vad::new();
        let noise = tone(0.02); // ~-37 dBFS fan noise
        assert!(vad.update(&noise));
        for _ in 0..(BLOCK as usize * (BLOCKS + 1)) {
            vad.update(&noise);
        }
        // After adaptation the noise no longer opens the gate…
        let open = (0..50).filter(|_| vad.update(&noise)).count();
        assert_eq!(open, 0);
        // …but louder speech still does.
        assert!(vad.update(&tone(0.3)));
    }
}
