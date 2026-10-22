//! Send side: device-rate mono samples в†’ 48 kHz 20 ms frames в†’ gain, noise
//! suppression, gate (PTT/VAD), Opus, `VoiceHeader` datagram.

use lc_p2p::proto::VoiceHeader;
use lc_types::AppResult;
use nnnoiseless::DenoiseState;

use crate::codec::OpusEncoder;
use crate::dsp::{meter, rms, Resampler, FRAME, FRAME_MS, SAMPLE_RATE};
use crate::vad::Vad;

/// Effective transmit mode (after the PTT в†’ VAD fallback).
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
    /// Datagram to broadcast; `None` while the gate is closed.
    pub datagram: Option<Vec<u8>>,
    /// Gate open and not muted.
    pub speaking: bool,
    /// 0..=1 meter of the processed frame.
    pub level: f32,
}

pub struct TxPipeline {
    resampler: Resampler,
    input_rate: u32,
    acc: Vec<f32>,
    denoise: Option<Box<DenoiseState<'static>>>,
    ns_buf: Vec<f32>,
    vad: Vad,
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
            denoise: None,
            ns_buf: vec![0.0; DenoiseState::FRAME_SIZE],
            vad: Vad::new(),
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
        if ctl.noise_suppression {
            self.denoise(&mut pcm);
        } else {
            self.denoise = None;
        }
        for s in &mut pcm {
            *s = s.clamp(-1.0, 1.0);
        }
        let level = meter(rms(&pcm));
        let vad_open = self.vad.update(&pcm);
        let open = match ctl.gate {
            Gate::Off => false,
            Gate::PushToTalk => ctl.ptt_pressed,
            Gate::Vad => vad_open,
        };
        let speaking = open && !ctl.self_muted;
        let datagram = if speaking { self.encode(&pcm) } else { None };
        Some(TxFrame {
            pcm,
            datagram,
            speaking,
            level,
        })
    }

    fn encode(&mut self, pcm: &[f32]) -> Option<Vec<u8>> {
        let opus = self.encoder.encode(pcm).ok()?;
        let header = VoiceHeader {
            speaker: self.speaker,
            seq: self.seq,
            timestamp_ms: self.frames.wrapping_mul(FRAME_MS),
        };
        self.seq = self.seq.wrapping_add(1);
        Some(header.encode(opus))
    }

    /// RNNoise works on 480-sample frames in i16 range.
    fn denoise(&mut self, pcm: &mut [f32]) {
        let state = self.denoise.get_or_insert_with(DenoiseState::new);
        let (chunks, _) = pcm.as_chunks_mut::<{ DenoiseState::FRAME_SIZE }>();
        for chunk in chunks {
            for s in chunk.iter_mut() {
                *s *= 32_767.0;
            }
            state.process_frame(&mut self.ns_buf, chunk);
            for (s, d) in chunk.iter_mut().zip(self.ns_buf.iter()) {
                *s = d / 32_767.0;
            }
        }
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

    fn ctl(gate: Gate) -> TxControl {
        TxControl {
            gate,
            ptt_pressed: false,
            self_muted: false,
            input_gain: 1.0,
            noise_suppression: false,
        }
    }

    fn tone(len: usize, amp: f32) -> Vec<f32> {
        (0..len).map(|i| amp * (i as f32 * 0.06).sin()).collect()
    }

    #[test]
    fn gating_modes() {
        let mut tx = TxPipeline::new(48_000, 7, 100).unwrap();
        tx.push_samples(&tone(FRAME * 6, 0.3));
        assert!(tx.next_frame(&ctl(Gate::Off)).unwrap().datagram.is_none());
        assert!(tx
            .next_frame(&ctl(Gate::PushToTalk))
            .unwrap()
            .datagram
            .is_none());
        let mut c = ctl(Gate::PushToTalk);
        c.ptt_pressed = true;
        let f = tx.next_frame(&c).unwrap();
        let (h, _) = VoiceHeader::decode(f.datagram.as_ref().unwrap()).unwrap();
        assert_eq!((h.speaker, h.seq), (7, 100));
        c.self_muted = true;
        assert!(!tx.next_frame(&c).unwrap().speaking);
        let f = tx.next_frame(&ctl(Gate::Vad)).unwrap();
        let (h, _) = VoiceHeader::decode(f.datagram.as_ref().unwrap()).unwrap();
        assert_eq!(h.seq, 101);
        assert!(f.level > 0.5);
        assert!(tx.next_frame(&ctl(Gate::Vad)).is_some());
        assert!(tx.next_frame(&ctl(Gate::Vad)).is_none());
    }

    #[test]
    fn resamples_device_rate_and_denoises() {
        let mut tx = TxPipeline::new(44_100, 0, 0).unwrap();
        tx.push_samples(&tone(4410 + 20, 0.3));
        let mut c = ctl(Gate::Vad);
        c.noise_suppression = true;
        let mut n = 0;
        while let Some(f) = tx.next_frame(&c) {
            assert_eq!(f.pcm.len(), FRAME);
            n += 1;
        }
        assert_eq!(n, 5); // 100 ms
    }
}
