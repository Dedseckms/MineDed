//! Small DSP helpers: streaming sinc resampler, high-pass filter, soft
//! limiter, levels, channel mixing.

use rubato::{
    Resampler as _, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

/// Opus/internal sample rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// 20 ms at 48 kHz mono.
pub const FRAME: usize = 960;
pub const FRAME_MS: u32 = 20;

/// Streaming band-limited (windowed sinc) resampler for mono audio.
/// Passthrough when the rates match. Input is buffered into fixed 10 ms
/// chunks, so it accepts arbitrary block sizes.
pub struct Resampler {
    inner: Option<SincFixedIn<f32>>,
    chunk: usize,
    pending: Vec<f32>,
    out: Vec<f32>,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let (from, to) = (from.max(1), to.max(1));
        let chunk = (from / 100).clamp(32, 4096) as usize;
        let inner = if from == to {
            None
        } else {
            let params = SincInterpolationParameters {
                sinc_len: 128,
                f_cutoff: 0.92,
                oversampling_factor: 256,
                interpolation: SincInterpolationType::Linear,
                window: WindowFunction::BlackmanHarris2,
            };
            match SincFixedIn::new(f64::from(to) / f64::from(from), 1.0, params, chunk, 1) {
                Ok(r) => Some(r),
                Err(e) => {
                    tracing::warn!(error = %e, from, to, "resampler init failed, passthrough");
                    None
                }
            }
        };
        let out_len = inner.as_ref().map_or(0, |r| r.output_frames_max());
        Self {
            inner,
            chunk,
            pending: Vec::with_capacity(chunk * 2),
            out: vec![0.0; out_len],
        }
    }

    #[cfg(test)]
    pub fn is_passthrough(&self) -> bool {
        self.inner.is_none()
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let Some(inner) = self.inner.as_mut() else {
            out.extend_from_slice(input);
            return;
        };
        self.pending.extend_from_slice(input);
        while self.pending.len() >= self.chunk {
            let res = inner.process_into_buffer(
                &[&self.pending[..self.chunk]],
                &mut [&mut self.out[..]],
                None,
            );
            match res {
                Ok((used, produced)) => {
                    out.extend_from_slice(&self.out[..produced]);
                    self.pending.drain(..used.max(1));
                }
                Err(_) => {
                    self.pending.drain(..self.chunk);
                }
            }
        }
    }
}

/// 2nd-order Butterworth high-pass (RBJ biquad); removes rumble, fan hum and
/// DC before noise suppression.
#[derive(Debug, Clone)]
pub struct HighPass {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl HighPass {
    pub fn new(cutoff_hz: f64, rate: u32) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * cutoff_hz / f64::from(rate.max(1));
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let b0 = (1.0 + cos) / 2.0 / a0;
        Self {
            b: [b0, -(1.0 + cos) / a0, b0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            let x0 = f64::from(*s);
            let y0 = self.b[0] * x0 + self.b[1] * self.x[0] + self.b[2] * self.x[1]
                - self.a[0] * self.y[0]
                - self.a[1] * self.y[1];
            self.x = [x0, self.x[0]];
            self.y = [y0, self.y[0]];
            *s = y0 as f32;
        }
    }
}

/// Knee of the soft limiter.
const LIMIT_KNEE: f32 = 0.8;

/// Memoryless soft limiter: linear below the knee, smooth `tanh` saturation
/// above it, never exceeds 1.0. No gain riding, so no pumping.
pub fn soft_limit(buf: &mut [f32]) {
    let room = 1.0 - LIMIT_KNEE;
    for s in buf {
        let a = s.abs();
        if a > LIMIT_KNEE {
            *s = s.signum() * (LIMIT_KNEE + room * ((a - LIMIT_KNEE) / room).tanh());
        }
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

/// Linear gain ramp `from → to` across the buffer (click-free fades).
pub fn ramp(buf: &mut [f32], from: f32, to: f32) {
    if (from - 1.0).abs() < f32::EPSILON && (to - 1.0).abs() < f32::EPSILON {
        return;
    }
    let n = buf.len().max(1) as f32;
    for (i, s) in buf.iter_mut().enumerate() {
        *s *= from + (to - from) * (i as f32 + 1.0) / n;
    }
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

    fn sine(freq: f32, rate: u32, start: usize, len: usize) -> Vec<f32> {
        (start..start + len)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn run(from: u32, to: u32, freq: f32, block: usize, blocks: usize) -> Vec<f32> {
        let mut r = Resampler::new(from, to);
        let mut out = Vec::new();
        for k in 0..blocks {
            r.process(&sine(freq, from, k * block, block), &mut out);
        }
        out
    }

    #[test]
    fn resampler_rate_ratio_and_tone() {
        // 1 s of 1 kHz at 44.1 kHz in odd-sized blocks → ~48000 samples.
        let out = run(44_100, 48_000, 1000.0, 441, 100);
        assert!((out.len() as i64 - 48_000).abs() < 600, "{}", out.len());
        // Steady state keeps the amplitude (0.5 peak → 0.354 RMS).
        let r = rms(&out[4800..]);
        assert!((r - 0.3536).abs() < 0.01, "{r}");
        // Frequency preserved: ~2 zero crossings per period.
        let tail = &out[4800..4800 + 48_00];
        let zc = tail
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        assert!((99..=101).contains(&zc), "{zc}");

        let mut r = Resampler::new(48_000, 48_000);
        assert!(r.is_passthrough());
        let mut out = Vec::new();
        r.process(&[1.0, 2.0], &mut out);
        assert_eq!(out, vec![1.0, 2.0]);
    }

    #[test]
    fn downsampling_is_band_limited() {
        let out = run(96_000, 48_000, 1000.0, 960, 10);
        assert!((out.len() as i64 - 4800).abs() < 600, "{}", out.len());
        // 30 kHz cannot be represented at 48 kHz: a linear resampler would
        // alias it to 18 kHz, the sinc filter removes it.
        let out = run(96_000, 48_000, 30_000.0, 960, 20);
        assert!(rms(&out[2000..]) < 0.01, "{}", rms(&out[2000..]));
    }

    #[test]
    fn high_pass_removes_rumble_keeps_voice() {
        let mut hp = HighPass::new(90.0, SAMPLE_RATE);
        let mut low = sine(30.0, SAMPLE_RATE, 0, 48_000);
        hp.process(&mut low);
        assert!(rms(&low[24_000..]) < 0.05, "{}", rms(&low[24_000..]));
        let mut hp = HighPass::new(90.0, SAMPLE_RATE);
        let mut voice = sine(300.0, SAMPLE_RATE, 0, 48_000);
        hp.process(&mut voice);
        assert!(rms(&voice[24_000..]) > 0.33);
    }

    #[test]
    fn limiter_and_ramp() {
        let mut v = vec![0.5, 1.5, -3.0, 0.9];
        soft_limit(&mut v);
        assert_eq!(v[0], 0.5);
        assert!(v[1] < 1.0 && v[1] > 0.9 && v[2] >= -1.0 && v[3] < 0.9);
        let mut f = vec![1.0; 4];
        ramp(&mut f, 1.0, 0.0);
        assert_eq!(f[3], 0.0);
        assert!(f[0] > f[1]);
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
