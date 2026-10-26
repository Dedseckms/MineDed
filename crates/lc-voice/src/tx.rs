//! Send side: device-rate mono samples → 48 kHz 20 ms frames → gain,
//! high-pass, RNNoise (+ speech probability), soft limiter, gate (PTT/VAD
//! with pre-roll and hangover), Opus, `VoiceHeader` datagrams.
//!
//! While capturing, every frame is encoded so the Opus encoder state stays
//! continuous; closed-gate packets go into a pre-roll ring that is flushed
//! (with consecutive sequence numbers) when the gate opens, so the start of
//! a phrase is never swallowed.

use std::collections::VecDeque;

use lc_p2p::proto::VoiceHeader;
use lc_types::AppResult;
use nnnoiseless::DenoiseState;

use crate::codec::OpusEncoder;
use crate::dsp::{
    dbfs, meter, ramp, rms, soft_limit, HighPass, Resampler, FRAME, FRAME_MS, SAMPLE_RATE,
};
use crate::vad::{GateStep, PttGate, Vad, PREROLL_FRAMES, PTT_PREROLL_FRAMES};

/// High-pass corner: below the voice fundamental, above rumble/fan hum.
const HIGH_PASS_HZ: f64 = 90.0;
/// Dry signal mixed back after RNNoise: masks "musical noise" artefacts.
const DRY_MIX: f32 = 0.06;

/// Effective transmit mode (after the PTT → VAD fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Off,
    PushToTalk,
    Vad,
}

/// Per-frame controls, snapshotted from the engine state.
#[derive(Debug, Clone, Copy)]
pub struct TxControl {
    pub gate: Gate,
    pub ptt_pressed: bool,
    pub self_muted: bool,
    pub input_gain: f32,
    pub noise_suppression: bool,
}

#[derive(Debug, Clone)]
pub struct TxFrame {
    /// Processed PCM (for the mic test loopback).
    pub pcm: Vec<f32>,
    /// Datagrams to broadcast in order (pre-roll flush + current frame);
    /// empty while the gate is closed.
    pub datagrams: Vec<Vec<u8>>,
    /// Gate open and not muted.
    pub speaking: bool,
    /// 0..=1 meter of the processed frame.
    pub level: f32,
}

pub struct TxPipeline {
    resampler: Resampler,
    input_rate: u32,
    acc: Vec<f32>,
    high_pass: HighPass,
    denoise: Box<DenoiseState<'static>>,
    ns_out: Vec<f32>,
    ns_in: Vec<f32>,
    vad: Vad,
    ptt: PttGate,
    last_gate: Gate,
    preroll: VecDeque<(u32, Vec<u8>)>,
    encoder: OpusEncoder,
    speaker: u8,
    seq: u16,
    frames: u32,
}

impl TxPipeline {
    pub fn new(input_rate: u32, speaker: u8, seq_start: u16) -> AppResult<Self> {
        Ok(Self {
            resampler: Resampler::new(input_rate, SAMPLE_RATE),
            input_rate,
            acc: Vec::with_capacity(FRAME * 4),
            high_pass: HighPass::new(HIGH_PASS_HZ, SAMPLE_RATE),
            denoise: DenoiseState::new(),
            ns_out: vec![0.0; DenoiseState::FRAME_SIZE],
            ns_in: vec![0.0; DenoiseState::FRAME_SIZE],
            vad: Vad::new(),
            ptt: PttGate::default(),
            last_gate: Gate::Off,
            preroll: VecDeque::with_capacity(PREROLL_FRAMES + 1),
            encoder: OpusEncoder::new()?,
            speaker,
            seq: seq_start,
            frames: 0,
        })
    }

    /// Re-targets the resampler when the capture device changes.
    pub fn set_input_rate(&mut self, rate: u32) {
        if rate != self.input_rate {
            self.input_rate = rate;
            self.resampler = Resampler::new(rate, SAMPLE_RATE);
            self.acc.clear();
        }
    }

    /// Mono samples at the input rate.
    pub fn push_samples(&mut self, samples: &[f32]) {
        self.resampler.process(samples, &mut self.acc);
        // Never queue more than 200 ms (e.g. after a stall).
        let max = FRAME * 10;
        if self.acc.len() > max {
            let drop = self.acc.len() - max;
            self.acc.drain(..drop);
        }
    }

    /// Processes the next complete frame, if any.
    pub fn next_frame(&mut self, ctl: &TxControl) -> Option<TxFrame> {
        if self.acc.len() < FRAME {
            return None;
        }
        let mut pcm: Vec<f32> = self.acc.drain(..FRAME).collect();
        self.frames = self.frames.wrapping_add(1);
        let gain = ctl.input_gain.clamp(0.0, 2.0);
        if (gain - 1.0).abs() > f32::EPSILON {
            for s in &mut pcm {
                *s *= gain;
            }
        }
        self.high_pass.process(&mut pcm);
        let raw_db = dbfs(rms(&pcm));
        let prob = self.denoise(&mut pcm, ctl.noise_suppression);
        soft_limit(&mut pcm);
        let level = meter(rms(&pcm));

        if ctl.gate != self.last_gate {
            self.vad.reset();
            self.ptt.reset();
            self.preroll.clear();
            self.last_gate = ctl.gate;
        }
        let (step, preroll_len) = match ctl.gate {
            Gate::Off => (GateStep::Closed, 0),
            Gate::PushToTalk => (self.ptt.update(ctl.ptt_pressed), PTT_PREROLL_FRAMES),
            Gate::Vad => (self.vad.update(prob, raw_db), PREROLL_FRAMES),
        };
        if step == GateStep::Closing {
            ramp(&mut pcm, 1.0, 0.0);
        }
        let mut datagrams = Vec::new();
        let speaking = step.is_open() && !ctl.self_muted;
        if ctl.gate != Gate::Off && !ctl.self_muted {
            if let Ok(opus) = self.encoder.encode(&pcm) {
                let opus = opus.to_vec();
                if step == GateStep::Opened {
                    while let Some((ts, p)) = self.preroll.pop_front() {
                        datagrams.push(self.packetize(ts, &p));
                    }
                }
                if step.is_open() {
                    datagrams.push(self.packetize(self.frames, &opus));
                } else {
                    self.preroll.push_back((self.frames, opus));
                    while self.preroll.len() > preroll_len {
                        self.preroll.pop_front();
                    }
                }
            }
        } else {
            self.preroll.clear();
        }
        Some(TxFrame {
            pcm,
            datagrams,
            speaking,
            level,
        })
    }

    fn packetize(&mut self, frame: u32, opus: &[u8]) -> Vec<u8> {
        let header = VoiceHeader {
            speaker: self.speaker,
            seq: self.seq,
            timestamp_ms: frame.wrapping_mul(FRAME_MS),
        };
        self.seq = self.seq.wrapping_add(1);
        header.encode(opus)
    }

    /// Runs RNNoise on both 10 ms halves; returns the max speech
    /// probability. The denoised signal (with a little dry signal mixed back)
    /// replaces `pcm` only when noise suppression is enabled.
    fn denoise(&mut self, pcm: &mut [f32], apply: bool) -> f32 {
        let mut prob = 0f32;
        let (chunks, _) = pcm.as_chunks_mut::<{ DenoiseState::FRAME_SIZE }>();
        for chunk in chunks {
            for (d, s) in self.ns_in.iter_mut().zip(chunk.iter()) {
                *d = s * 32_767.0;
            }
            prob = prob.max(self.denoise.process_frame(&mut self.ns_out, &self.ns_in));
            if apply {
                for (s, d) in chunk.iter_mut().zip(self.ns_out.iter()) {
                    *s = d / 32_767.0 * (1.0 - DRY_MIX) + *s * DRY_MIX;
                }
            }
        }
        prob
    }

    /// Opus algorithmic delay in samples.
    #[cfg(test)]
    pub fn codec_delay(&self) -> u32 {
        self.encoder.lookahead()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vad::PTT_TAIL;

    fn ctl(gate: Gate) -> TxControl {
        TxControl {
            gate,
            ptt_pressed: false,
            self_muted: false,
            input_gain: 1.0,
            noise_suppression: true,
        }
    }

    fn tone(len: usize, amp: f32) -> Vec<f32> {
        (0..len).map(|i| amp * (i as f32 * 0.06).sin()).collect()
    }

    fn noise(len: usize, amp: f32, seed: &mut u32) -> Vec<f32> {
        (0..len)
            .map(|_| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 17;
                *seed ^= *seed << 5;
                amp * ((*seed % 2001) as f32 / 1000.0 - 1.0)
            })
            .collect()
    }

    fn seqs(f: &TxFrame) -> Vec<u16> {
        f.datagrams
            .iter()
            .map(|d| VoiceHeader::decode(d).unwrap().0.seq)
            .collect()
    }

    #[test]
    fn ptt_preroll_and_tail() {
        let mut tx = TxPipeline::new(48_000, 7, 100).unwrap();
        let step = |tx: &mut TxPipeline, c: &TxControl| {
            tx.push_samples(&tone(FRAME, 0.3));
            tx.next_frame(c).unwrap()
        };
        let mut c = ctl(Gate::PushToTalk);
        assert!(step(&mut tx, &ctl(Gate::Off)).datagrams.is_empty());
        for _ in 0..5 {
            assert!(step(&mut tx, &c).datagrams.is_empty());
        }
        c.ptt_pressed = true;
        let f = step(&mut tx, &c);
        // Pre-roll + current frame, consecutive sequence numbers.
        assert_eq!(seqs(&f), vec![100, 101, 102, 103]);
        assert_eq!(VoiceHeader::decode(&f.datagrams[0]).unwrap().0.speaker, 7);
        assert!(f.speaking);
        c.ptt_pressed = false;
        let sent: usize = (0..PTT_TAIL + 3)
            .map(|_| step(&mut tx, &c).datagrams.len())
            .sum();
        assert_eq!(sent, PTT_TAIL as usize);
        c.ptt_pressed = true;
        c.self_muted = true;
        let f = step(&mut tx, &c);
        assert!(!f.speaking && f.datagrams.is_empty());
    }

    #[test]
    fn steady_noise_never_opens_vad() {
        // Fan-like broadband noise plus low hum: no transmissions at all.
        let mut tx = TxPipeline::new(48_000, 0, 0).unwrap();
        let mut seed = 0x9e37_79b9u32;
        let mut sent = 0;
        let mut transitions = 0;
        let mut prev = false;
        for k in 0..250 {
            let mut buf = noise(FRAME, 0.03, &mut seed);
            for (i, s) in buf.iter_mut().enumerate() {
                *s += 0.05
                    * ((k * FRAME + i) as f32 * 2.0 * std::f32::consts::PI * 50.0 / 48_000.0).sin();
            }
            tx.push_samples(&buf);
            let f = tx.next_frame(&ctl(Gate::Vad)).unwrap();
            sent += f.datagrams.len();
            transitions += usize::from(f.speaking != prev);
            prev = f.speaking;
        }
        assert!(transitions <= 2, "{transitions}");
        assert!(sent < 40, "{sent}");
    }

    #[test]
    fn vad_closed_frames_feed_preroll_only() {
        let mut tx = TxPipeline::new(48_000, 0, 0).unwrap();
        let mut n = 0;
        for _ in 0..30 {
            tx.push_samples(&[0.0; FRAME]);
            let f = tx.next_frame(&ctl(Gate::Vad)).unwrap();
            assert!(f.datagrams.is_empty() && !f.speaking);
            n += 1;
        }
        assert_eq!(n, 30);
        assert_eq!(tx.preroll.len(), PREROLL_FRAMES);
    }

    #[test]
    fn resamples_device_rate() {
        let mut tx = TxPipeline::new(44_100, 0, 0).unwrap();
        let input = tone(44_100, 0.3);
        let mut n = 0;
        for block in input.chunks(441) {
            tx.push_samples(block);
            while let Some(f) = tx.next_frame(&ctl(Gate::Vad)) {
                assert_eq!(f.pcm.len(), FRAME);
                n += 1;
            }
        }
        assert!((49..=50).contains(&n), "{n}");
    }
}
