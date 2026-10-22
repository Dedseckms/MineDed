//! Small DSP helpers: streaming linear resampler, levels, channel mixing.

/// Opus/internal sample rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// 20 ms at 48 kHz mono.
pub const FRAME: usize = 960;
pub const FRAME_MS: u32 = 20;

/// Streaming linear-interpolation resampler (mono). Cheap and good enough for
/// voice; passthrough when the rates match.
#[derive(Debug, Clone)]
pub struct Resampler {
    /// Input samples advanced per output sample.
    step: f64,
    /// Position of the next output, relative to the current chunk start
    /// (`-1` refers to `last`).
    pos: f64,
    last: f32,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self {
            step: f64::from(from.max(1)) / f64::from(to.max(1)),
            pos: 0.0,
            last: 0.0,
        }
    }

    pub fn is_passthrough(&self) -> bool {
        (self.step - 1.0).abs() < f64::EPSILON
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.is_passthrough() {
            out.extend_from_slice(input);
            return;
        }
        let Some(&tail) = input.last() else {
            return;
        };
        let n = input.len() as f64;
        let at = |i: i64| -> f32 {
            if i < 0 {
                self.last
            } else {
                input[i as usize]
            }
        };
        while self.pos < n - 1.0 {
            let i = self.pos.floor();
            let frac = (self.pos - i) as f32;
            let a = at(i as i64);
            let b = at(i as i64 + 1);
            out.push(a + (b - a) * frac);
            self.pos += self.step;
        }
        self.pos -= n;
        self.last = tail;
    }
}

/// RMS of a frame.
pub fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    (sum / frame.len() as f32).sqrt()
}

/// RMS in dBFS, floored at -100.
pub fn dbfs(rms: f32) -> f32 {
    if rms <= 1e-5 {
        -100.0
    } else {
        20.0 * rms.log10()
    }
}

/// Maps RMS to a 0..=1 meter value (-60 dBFS .. 0 dBFS).
pub fn meter(rms: f32) -> f32 {
    ((dbfs(rms) + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// Interleaved multi-channel → mono (average).
pub fn downmix(interleaved: &[f32], channels: usize, out: &mut Vec<f32>) {
    if channels <= 1 {
        out.extend_from_slice(interleaved);
        return;
    }
    let inv = 1.0 / channels as f32;
    for frame in interleaved.chunks_exact(channels) {
        out.push(frame.iter().sum::<f32>() * inv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_rate_ratio() {
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        for _ in 0..100 {
            r.process(&[0.5; 441], &mut out);
        }
        // 44100 in → ~48000 out.
        assert!((out.len() as i64 - 48_000).abs() < 3, "{}", out.len());
        assert!(out.iter().skip(2).all(|s| (s - 0.5).abs() < 1e-6));

        let mut r = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        r.process(&[1.0, 2.0], &mut out);
        assert_eq!(out, vec![1.0, 2.0]);
    }

    #[test]
    fn downsample_ratio() {
        let mut r = Resampler::new(96_000, 48_000);
        let mut out = Vec::new();
        for _ in 0..10 {
            r.process(&[0.1; 960], &mut out);
        }
        assert!((out.len() as i64 - 4800).abs() < 2);
    }

    #[test]
    fn levels() {
        assert_eq!(meter(0.0), 0.0);
        assert!((meter(1.0) - 1.0).abs() < 1e-6);
        let mut m = Vec::new();
        downmix(&[1.0, 0.0, 0.5, 0.5], 2, &mut m);
        assert_eq!(m, vec![0.5, 0.5]);
    }
}
