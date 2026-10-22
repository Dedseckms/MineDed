//! Receive side: per-speaker jitter buffer + decoder, mixing with per-peer
//! volume/mute, output volume and the local mic-test loopback.

use std::collections::{HashMap, VecDeque};

use lc_p2p::proto::VoiceHeader;

use crate::codec::OpusDecoder;
use crate::dsp::{meter, rms, FRAME};
use crate::jitter::{JitterBuffer, Playout};

/// Streams without packets for this many frames are dropped (10 s).
const IDLE_FRAMES: u32 = 500;
/// A stream counts as speaking while it played a real packet within 200 ms.
const SPEAKING_FRAMES: u32 = 10;
/// Mic-test loopback is capped at 200 ms so it never drifts behind.
const LOOPBACK_MAX: usize = FRAME * 10;

/// A remote stream: the transport-level sender plus the header speaker byte
/// (the host forwards other guests' voice over its own connection).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StreamKey {
    pub peer_id: String,
    pub speaker: u8,
}

struct Stream {
    jitter: JitterBuffer,
    decoder: Option<OpusDecoder>,
    /// Frames since the last real packet was played.
    since_voice: u32,
    /// Frames since the last packet arrived.
    since_rx: u32,
    /// Peak meter value since the last activity snapshot.
    level: f32,
}

/// Activity of one peer for the speaking indicator.
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    pub peer_id: String,
    pub speaking: bool,
    pub level: f32,
}

#[derive(Default)]
pub struct Mixer {
    streams: HashMap<StreamKey, Stream>,
    muted: HashMap<String, bool>,
    volumes: HashMap<String, f32>,
    output_volume: f32,
    loopback: VecDeque<f32>,
    scratch: Vec<f32>,
}

impl Mixer {
    pub fn new() -> Self {
        Self {
            output_volume: 1.0,
            ..Self::default()
        }
    }

    pub fn set_output_volume(&mut self, v: f32) {
        self.output_volume = v.clamp(0.0, 2.0);
    }

    pub fn set_peer_volume(&mut self, peer_id: &str, v: f32) {
        self.volumes.insert(peer_id.to_owned(), v.clamp(0.0, 2.0));
    }

    pub fn set_peer_volumes<'a>(&mut self, all: impl IntoIterator<Item = (&'a String, &'a f32)>) {
        self.volumes = all
            .into_iter()
            .map(|(k, v)| (k.clone(), v.clamp(0.0, 2.0)))
            .collect();
    }

    pub fn set_peer_muted(&mut self, peer_id: &str, muted: bool) {
        if muted {
            self.muted.insert(peer_id.to_owned(), true);
        } else {
            self.muted.remove(peer_id);
        }
    }

    /// Drops all remote streams (voice stopped).
    pub fn clear_streams(&mut self) {
        self.streams.clear();
    }

    /// Feeds a raw datagram from the transport. Malformed ones are ignored.
    pub fn push_datagram(&mut self, peer_id: &str, data: &[u8]) {
        let Ok((header, opus)) = VoiceHeader::decode(data) else {
            return;
        };
        if opus.is_empty() || opus.len() > crate::codec::MAX_PACKET {
            return;
        }
        let key = StreamKey {
            peer_id: peer_id.to_owned(),
            speaker: header.speaker,
        };
        let stream = self.streams.entry(key).or_insert_with(|| Stream {
            jitter: JitterBuffer::new(),
            decoder: None,
            since_voice: u32::MAX / 2,
            since_rx: 0,
            level: 0.0,
        });
        stream.since_rx = 0;
        stream.jitter.push(header.seq, opus.to_vec());
    }

    /// Mic-test: plays local processed microphone audio (48 kHz mono).
    pub fn push_loopback(&mut self, pcm: &[f32]) {
        self.loopback.extend(pcm.iter().copied());
        while self.loopback.len() > LOOPBACK_MAX {
            self.loopback.pop_front();
        }
    }

    pub fn clear_loopback(&mut self) {
        self.loopback.clear();
    }

    /// Produces the next 20 ms of mixed output.
    pub fn mix(&mut self, out: &mut [f32; FRAME]) {
        out.fill(0.0);
        let mut frame = [0f32; FRAME];
        self.streams.retain(|_, s| s.since_rx < IDLE_FRAMES);
        for (key, s) in &mut self.streams {
            s.since_rx = s.since_rx.saturating_add(1);
            let playout = s.jitter.pop();
            let muted = self.muted.contains_key(&key.peer_id);
            if matches!(playout, Playout::Packet(_)) {
                s.since_voice = 0;
            } else {
                s.since_voice = s.since_voice.saturating_add(1);
            }
            if matches!(playout, Playout::Silence) {
                continue;
            }
            // Muted peers are not decoded at all (CPU), only tracked.
            if muted {
                continue;
            }
            let decoder = match s.decoder.as_mut() {
                Some(d) => d,
                None => match OpusDecoder::new() {
                    Ok(d) => s.decoder.insert(d),
                    Err(_) => continue,
                },
            };
            match &playout {
                Playout::Packet(p) => decoder.decode(p, &mut frame),
                Playout::Lost { fec: Some(next) } => decoder.decode_fec(next, &mut frame),
                Playout::Lost { fec: None } => decoder.conceal(&mut frame),
                Playout::Silence => continue,
            }
            let vol = self.volumes.get(&key.peer_id).copied().unwrap_or(1.0);
            if matches!(playout, Playout::Packet(_)) {
                s.level = s.level.max(meter(rms(&frame)));
            }
            for (o, x) in out.iter_mut().zip(frame.iter()) {
                *o += x * vol;
            }
        }
        if !self.loopback.is_empty() {
            for o in out.iter_mut() {
                match self.loopback.pop_front() {
                    Some(x) => *o += x,
                    None => break,
                }
            }
        }
        let gain = self.output_volume;
        for o in out.iter_mut() {
            *o = (*o * gain).clamp(-1.0, 1.0);
        }
    }

    /// Activity per peer since the previous call (levels are peak-held).
    pub fn activity(&mut self) -> Vec<Activity> {
        let mut by_peer: HashMap<&str, Activity> = HashMap::new();
        for (key, s) in &mut self.streams {
            let speaking =
                s.since_voice < SPEAKING_FRAMES && !self.muted.contains_key(&key.peer_id);
            let level = if speaking { s.level } else { 0.0 };
            s.level = 0.0;
            let e = by_peer.entry(&key.peer_id).or_insert_with(|| Activity {
                peer_id: key.peer_id.clone(),
                speaking: false,
                level: 0.0,
            });
            e.speaking |= speaking;
            e.level = e.level.max(level);
        }
        let mut v: Vec<_> = by_peer.into_values().collect();
        v.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
        v
    }

    /// Pulls `n` 48 kHz samples, mixing new frames as needed (output callback).
    pub fn pull(&mut self, n: usize, out: &mut Vec<f32>) {
        let mut buf = [0f32; FRAME];
        while self.scratch.len() < n {
            self.mix(&mut buf);
            self.scratch.extend_from_slice(&buf);
        }
        out.extend(self.scratch.drain(..n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::OpusEncoder;

    fn tone(k: usize) -> Vec<f32> {
        (0..FRAME)
            .map(|i| 0.25 * (((k * FRAME + i) as f32) * 0.06).sin())
            .collect()
    }

    fn feed(mixer: &mut Mixer, enc: &mut OpusEncoder, peer: &str, speaker: u8, seq: u16) {
        let pkt = enc.encode(&tone(seq as usize)).unwrap().to_vec();
        let h = VoiceHeader {
            speaker,
            seq,
            timestamp_ms: u32::from(seq) * 20,
        };
        mixer.push_datagram(peer, &h.encode(&pkt));
    }

    fn energy_run(volume_a: f32, muted_b: bool, out_vol: f32) -> f32 {
        let mut mixer = Mixer::new();
        mixer.set_peer_volume("a", volume_a);
        mixer.set_peer_muted("b", muted_b);
        mixer.set_output_volume(out_vol);
        let mut ea = OpusEncoder::new().unwrap();
        let mut eb = OpusEncoder::new().unwrap();
        let mut out = [0f32; FRAME];
        let mut e = 0.0;
        for seq in 0..30u16 {
            feed(&mut mixer, &mut ea, "a", 1, seq);
            feed(&mut mixer, &mut eb, "b", 2, seq);
            mixer.mix(&mut out);
            if seq > 5 {
                e += rms(&out);
            }
        }
        e
    }

    #[test]
    fn volumes_and_mute() {
        let base = energy_run(1.0, true, 1.0);
        assert!(base > 0.1);
        let half = energy_run(0.5, true, 1.0);
        assert!((half / base - 0.5).abs() < 0.05, "{}", half / base);
        let silent = energy_run(0.0, true, 1.0);
        assert!(silent < 1e-3);
        let both = energy_run(1.0, false, 1.0);
        assert!(both > base * 1.2);
        let out_half = energy_run(1.0, true, 0.5);
        assert!((out_half / base - 0.5).abs() < 0.05);
    }

    #[test]
    fn activity_and_garbage() {
        let mut mixer = Mixer::new();
        mixer.push_datagram("x", &[1, 2]);
        mixer.push_datagram("x", &[9, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert!(mixer.activity().is_empty());
        let mut enc = OpusEncoder::new().unwrap();
        let mut out = [0f32; FRAME];
        for seq in 0..5 {
            feed(&mut mixer, &mut enc, "a", 0, seq);
            mixer.mix(&mut out);
        }
        let act = mixer.activity();
        assert_eq!(act.len(), 1);
        assert!(act[0].speaking && act[0].level > 0.3);
        for _ in 0..20 {
            mixer.mix(&mut out);
        }
        assert!(!mixer.activity()[0].speaking);
    }

    #[test]
    fn loopback_and_pull() {
        let mut mixer = Mixer::new();
        mixer.push_loopback(&[0.5; 100]);
        let mut out = Vec::new();
        mixer.pull(150, &mut out);
        assert_eq!(out.len(), 150);
        assert_eq!(out[99], 0.5);
        assert_eq!(out[100], 0.0);
    }
}
