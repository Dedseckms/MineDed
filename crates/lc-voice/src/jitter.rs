//! Per-speaker jitter buffer keyed by the 16-bit wrapping sequence number.
//!
//! Senders only transmit while the gate is open (PTT/VAD), so gaps in arrival
//! are normal: after a few concealed frames the buffer goes back to
//! prebuffering and re-anchors on the next talk spurt.

use std::collections::VecDeque;

/// Frames buffered before playout starts (2 Г— 20 ms).
pub const TARGET: usize = 2;
/// Above this many buffered frames the oldest are dropped (latency cap).
pub const MAX_BUFFERED: usize = 8;
/// Packets further than this ahead of the playout point re-anchor the stream.
const MAX_AHEAD: i16 = 50;
/// Concealed frames before the stream is considered silent.
const PLC_MAX: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Playout {
    Packet(Vec<u8>),
    /// Frame missing: conceal; `fec` is the next packet if it already arrived.
    Lost {
        fec: Option<Vec<u8>>,
    },
    /// Nothing to play.
    Silence,
}

#[derive(Debug, Default)]
pub struct JitterBuffer {
    /// Slot `i` holds sequence `next + i`.
    slots: VecDeque<Option<Vec<u8>>>,
    next: Option<u16>,
    playing: bool,
    missing: u32,
}

impl JitterBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn buffered(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    pub fn push(&mut self, seq: u16, payload: Vec<u8>) {
        let Some(next) = self.next else {
            self.anchor(seq, payload);
            return;
        };
        let d = seq.wrapping_sub(next) as i16;
        if d < 0 {
            // While idle a packet from "the past" means the sender restarted
            // (or is a reordered start of a spurt): re-anchor.
            if !self.playing && self.buffered() == 0 {
                self.anchor(seq, payload);
            } else if !self.playing && d > -(MAX_AHEAD) {
                for _ in 0..(-d) {
                    self.slots.push_front(None);
                }
                self.next = Some(seq);
                self.slots[0] = Some(payload);
            }
            // Otherwise: late or duplicate, drop.
            return;
        }
        if d >= MAX_AHEAD || (!self.playing && self.buffered() == 0 && d as usize > TARGET) {
            self.anchor(seq, payload);
            return;
        }
        let idx = d as usize;
        while self.slots.len() <= idx {
            self.slots.push_back(None);
        }
        if self.slots[idx].is_none() {
            self.slots[idx] = Some(payload);
        }
        while self.slots.len() > MAX_BUFFERED {
            self.advance();
        }
    }

    fn anchor(&mut self, seq: u16, payload: Vec<u8>) {
        self.slots.clear();
        self.slots.push_back(Some(payload));
        self.next = Some(seq);
        self.playing = false;
        self.missing = 0;
    }

    fn advance(&mut self) -> Option<Vec<u8>> {
        let front = self.slots.pop_front().flatten();
        if let Some(n) = self.next.as_mut() {
            *n = n.wrapping_add(1);
        }
        front
    }

    /// Next 20 ms frame to play.
    pub fn pop(&mut self) -> Playout {
        if !self.playing {
            if self.slots.len() >= TARGET && self.buffered() > 0 {
                self.playing = true;
            } else {
                return Playout::Silence;
            }
        }
        if self.buffered() == 0 {
            self.missing += 1;
            if self.missing > PLC_MAX {
                self.playing = false;
                self.slots.clear();
                return Playout::Silence;
            }
            // Keep the expected sequence moving so late packets are dropped.
            self.advance();
            return Playout::Lost { fec: None };
        }
        match self.advance() {
            Some(p) => {
                self.missing = 0;
                Playout::Packet(p)
            }
            None => {
                self.missing += 1;
                let fec = self.slots.front().cloned().flatten();
                Playout::Lost { fec }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(n: u8) -> Vec<u8> {
        vec![n]
    }

    fn drain(jb: &mut JitterBuffer, n: usize) -> Vec<Playout> {
        (0..n).map(|_| jb.pop()).collect()
    }

    #[test]
    fn in_order_with_prebuffer() {
        let mut jb = JitterBuffer::new();
        jb.push(10, p(10));
        assert_eq!(jb.pop(), Playout::Silence);
        jb.push(11, p(11));
        assert_eq!(jb.pop(), Playout::Packet(p(10)));
        jb.push(12, p(12));
        assert_eq!(jb.pop(), Playout::Packet(p(11)));
        assert_eq!(jb.pop(), Playout::Packet(p(12)));
    }

    #[test]
    fn reorder_and_duplicates() {
        let mut jb = JitterBuffer::new();
        for s in [1u16, 3, 2, 2, 4, 3, 5] {
            jb.push(s, p(s as u8));
        }
        let got = drain(&mut jb, 5);
        let want: Vec<_> = (1..=5).map(|s| Playout::Packet(p(s))).collect();
        assert_eq!(got, want);
        // Late duplicate is dropped.
        jb.push(3, p(3));
        assert_eq!(jb.buffered(), 0);
    }

    #[test]
    fn reordered_start_of_spurt() {
        let mut jb = JitterBuffer::new();
        jb.push(6, p(6));
        jb.push(5, p(5));
        assert_eq!(jb.pop(), Playout::Packet(p(5)));
        assert_eq!(jb.pop(), Playout::Packet(p(6)));
    }

    #[test]
    fn loss_uses_fec_from_next_packet() {
        let mut jb = JitterBuffer::new();
        jb.push(1, p(1));
        jb.push(3, p(3));
        jb.push(4, p(4));
        assert_eq!(jb.pop(), Playout::Packet(p(1)));
        assert_eq!(jb.pop(), Playout::Lost { fec: Some(p(3)) });
        assert_eq!(jb.pop(), Playout::Packet(p(3)));
        assert_eq!(jb.pop(), Playout::Packet(p(4)));
    }

    #[test]
    fn end_of_spurt_conceals_then_silence_and_resumes() {
        let mut jb = JitterBuffer::new();
        jb.push(1, p(1));
        jb.push(2, p(2));
        drain(&mut jb, 2);
        let tail = drain(&mut jb, 5);
        assert_eq!(tail[0], Playout::Lost { fec: None });
        assert_eq!(tail[2], Playout::Lost { fec: None });
        assert_eq!(tail[3], Playout::Silence);
        assert!(!jb.is_playing());
        // Next spurt continues the sequence much later.
        jb.push(3, p(3));
        jb.push(4, p(4));
        assert_eq!(jb.pop(), Playout::Packet(p(3)));
    }

    #[test]
    fn wraparound_and_sender_restart() {
        let mut jb = JitterBuffer::new();
        jb.push(65_535, p(1));
        jb.push(0, p(2));
        jb.push(1, p(3));
        assert_eq!(
            drain(&mut jb, 3),
            vec![
                Playout::Packet(p(1)),
                Playout::Packet(p(2)),
                Playout::Packet(p(3))
            ]
        );
        // Big jump = new stream.
        jb.push(30_000, p(9));
        jb.push(30_001, p(10));
        assert_eq!(jb.pop(), Playout::Packet(p(9)));
    }

    #[test]
    fn latency_is_capped() {
        let mut jb = JitterBuffer::new();
        for s in 0..20u16 {
            jb.push(s, p(s as u8));
        }
        assert!(jb.buffered() <= MAX_BUFFERED);
        assert_eq!(jb.pop(), Playout::Packet(p(12)));
    }
}
