//! Opus 48 kHz mono, 20 ms frames (ropus: pure-Rust port of libopus).

use lc_types::{AppError, AppResult};
use ropus::{
    Application, Bitrate, Channels, DecodeMode, Decoder, Encoder, FrameDuration, InbandFec, Signal,
};

use crate::dsp::{FRAME, SAMPLE_RATE};

/// Max encoded frame we accept/produce.
pub const MAX_PACKET: usize = 400;
const BITRATE: u32 = 24_000;
/// Low complexity keeps CPU usage small on weak PCs.
const COMPLEXITY: u8 = 5;

pub struct OpusEncoder {
    inner: Encoder,
    buf: Vec<u8>,
}

impl OpusEncoder {
    pub fn new() -> AppResult<Self> {
        let inner = Encoder::builder(SAMPLE_RATE, Channels::Mono, Application::Voip)
            .bitrate(Bitrate::Bits(BITRATE))
            .complexity(COMPLEXITY)
            .signal(Signal::Voice)
            .vbr(true)
            .frame_duration(FrameDuration::Ms20)
            .inband_fec(InbandFec::Enabled)
            .packet_loss_perc(10)
            .build()
            .map_err(|e| AppError::internal(format!("opus encoder: {e}")))?;
        Ok(Self {
            inner,
            buf: vec![0; MAX_PACKET],
        })
    }

    /// Encodes one 20 ms frame (`FRAME` samples in -1..1).
    pub fn encode(&mut self, pcm: &[f32]) -> AppResult<&[u8]> {
        let n = self
            .inner
            .encode_float(pcm, &mut self.buf)
            .map_err(|e| AppError::internal(format!("opus encode: {e}")))?;
        Ok(&self.buf[..n])
    }

    /// Algorithmic delay in samples at 48 kHz.
    #[cfg(test)]
    pub fn lookahead(&self) -> u32 {
        self.inner.lookahead()
    }
}

pub struct OpusDecoder {
    inner: Decoder,
}

impl OpusDecoder {
    pub fn new() -> AppResult<Self> {
        let inner = Decoder::new(SAMPLE_RATE, Channels::Mono)
            .map_err(|e| AppError::internal(format!("opus decoder: {e}")))?;
        Ok(Self { inner })
    }

    /// Decodes a packet into `out` (`FRAME` samples). Malformed packets are
    /// concealed instead of failing.
    pub fn decode(&mut self, packet: &[u8], out: &mut [f32; FRAME]) {
        if self
            .inner
            .decode_float(packet, out, DecodeMode::Normal)
            .is_err()
        {
            self.conceal(out);
        }
    }

    /// Recovers a lost frame from the in-band FEC of the following packet.
    pub fn decode_fec(&mut self, next_packet: &[u8], out: &mut [f32; FRAME]) {
        if self
            .inner
            .decode_float(next_packet, out, DecodeMode::Fec)
            .is_err()
        {
            self.conceal(out);
        }
    }

    /// Packet loss concealment.
    pub fn conceal(&mut self, out: &mut [f32; FRAME]) {
        if self
            .inner
            .decode_float(&[], out, DecodeMode::Normal)
            .is_err()
        {
            out.fill(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sine(freq: f32, amp: f32, start: usize, len: usize) -> Vec<f32> {
        (start..start + len)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn opus_roundtrip_preserves_energy() {
        let mut enc = OpusEncoder::new().unwrap();
        let mut dec = OpusDecoder::new().unwrap();
        let mut out = [0f32; FRAME];
        let mut in_energy = 0.0;
        let mut out_energy = 0.0;
        for k in 0..25 {
            let frame = sine(440.0, 0.3, k * FRAME, FRAME);
            let packet = enc.encode(&frame).unwrap().to_vec();
            assert!(!packet.is_empty() && packet.len() < 200, "{}", packet.len());
            dec.decode(&packet, &mut out);
            if k >= 5 {
                in_energy += crate::dsp::rms(&frame);
                out_energy += crate::dsp::rms(&out);
            }
        }
        let ratio = out_energy / in_energy;
        assert!((0.7..1.3).contains(&ratio), "energy ratio {ratio}");
    }

    #[test]
    fn codec_fuzz_never_panics_in_debug() {
        let mut seed = 0x1234_5678u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut panics = 0;
        let mut enc = OpusEncoder::new().unwrap();
        let mut dec = OpusDecoder::new().unwrap();
        let mut out = [0f32; FRAME];
        for k in 0..600 {
            let amp = (rnd() % 1000) as f32 / 1000.0;
            let frame: Vec<f32> = (0..FRAME)
                .map(|i| {
                    let noise = (rnd() % 2001) as f32 / 1000.0 - 1.0;
                    if k % 3 == 0 {
                        noise * amp
                    } else {
                        amp * ((i * (k % 50 + 1)) as f32 * 0.01).sin()
                    }
                })
                .collect();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let p = enc.encode(&frame).unwrap().to_vec();
                let mut p2 = p.clone();
                if k % 5 == 0 && p2.len() > 2 {
                    let idx = (rnd() as usize) % (p2.len() - 1) + 1;
                    p2[idx] ^= rnd() as u8;
                }
                dec.decode(&p2, &mut out);
                if k % 7 == 0 {
                    dec.conceal(&mut out);
                    dec.decode_fec(&p, &mut out);
                }
            }));
            if r.is_err() {
                panics += 1;
                enc = OpusEncoder::new().unwrap();
                dec = OpusDecoder::new().unwrap();
            }
        }
        assert_eq!(panics, 0);
    }

    #[test]
    fn plc_and_garbage_do_not_panic() {
        let mut dec = OpusDecoder::new().unwrap();
        let mut out = [1f32; FRAME];
        dec.conceal(&mut out);
        dec.decode(&[0xff, 0xff, 0xff], &mut out);
        dec.decode_fec(&[], &mut out);
    }
}
