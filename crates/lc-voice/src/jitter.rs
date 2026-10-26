//! Per-speaker adaptive jitter buffer keyed by the 16-bit wrapping sequence
//! number.
//!
//! - Playout starts once `target` frames are buffered (40–120 ms, adaptive).
//! - Underrun (nothing buffered): the frame is concealed *without* advancing
//!   the sequence, so a late packet still plays and the buffer grows by one
//!   frame; each such late arrival raises `target`. Long stable periods lower
//!   it again.
//! - Real loss (a hole with later packets present): conceal with FEC from
//!   the next packet and advance.
//! - Excess depth (pre-roll bursts, clock drift) is drained by the mixer via
//!   [`JitterBuffer::excess`], which skips quiet frames.
//!
//! Senders only transmit while their gate is open, so after a few concealed
//! frames the stream is considered silent and re-anchors on the next spurt.

use std::collections::VecDeque;

/// Bounds and start value of the adaptive playout target (frames of 20 ms).
pub const MIN_TARGET: usize = 2;
pub const MAX_TARGET: usize = 6;
const START_TARGET: usize = 3;
/// Hard latency cap: oldest frames beyond this are dropped (500 ms; leaves
/// room for a pre-roll burst).
pub const MAX_BUFFERED: usize = 25;
/// Packets further than this ahead of the playout point re-anchor the stream.
const MAX_AHEAD: i16 = 60;
/// Underrun concealments before the stream is considered silent.
const PLC_MAX: u32 = 3;
/// Playing pops without underrun before `target` is lowered (5 s).
const STABLE_POPS: u32 = 250;

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

#[derive(Debug)]
pub struct JitterBuffer {
    /// Slot `i` holds sequence `next + i`.
    slots: VecDeque<Option<Vec<u8>>>,
    next: Option<u16>,
    playing: bool,
    /// Consecutive underrun concealments.
    underruns: u32,
    /// Pops spent prebuffering.
    waits: u32,
    target: usize,
    stable: u32,
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self {
            slots: VecDeque::new(),
            next: None,
            playing: false,
            underruns: 0,
            waits: 0,
            target: START_TARGET,
            stable: 0,
        }
    }
}

impl JitterBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Current playout target in frames.
    #[cfg(test)]
    pub fn target(&self) -> usize {
        self.target
    }

    pub fn buffered(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Frames buffered beyond the target while playing (latency to drain).
    pub fn excess(&self) -> usize {
        if self.playing {
            self.slots.len().saturating_sub(self.target)
        } else {
            0
        }
    }

    pub fn push(&mut self, seq: u16, payload: Vec<u8>) {
        let Some(next) = self.next else {
            self.anchor(seq, payload);
            return;
        };
        let d = seq.wrapping_sub(next) as i16;
        if d < 0 {
            // While idle, a packet from "the past" is either a reordered start
            // of the spurt (keep both) or a restarted sender (re-anchor; any
            // stale prebuffered packet is discarded).
            if !self.playing && (self.buffered() == 0 || d <= -(MAX_AHEAD)) {
                self.anchor(seq, payload);
            } else if !self.playing {
                for _ in 0..(-d) {
                    self.slots.push_front(None);
                }
                self.next = Some(seq);
                self.slots[0] = Some(payload);
            }
            // Otherwise: late or duplicate, drop.
            return;
        }
        if d >= MAX_AHEAD || (!self.playing && self.buffered() == 0 && d as usize > self.target) {
            self.anchor(seq, payload);
            return;
        }
        if d == 0 && self.playing && self.underruns > 0 {
            // The expected packet came after we had to conceal: too little
            // buffering for this path.
            self.target = (self.target + 1).min(MAX_TARGET);
            self.stable = 0;
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
        self.underruns = 0;
        self.waits = 0;
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
            if self.buffered() == 0 {
                return Playout::Silence;
            }
            self.waits += 1;
            // A short spurt does not wait forever for its successors.
            if self.slots.len() >= self.target || self.waits as usize > self.target {
                self.playing = true;
                self.waits = 0;
            } else {
                return Playout::Silence;
            }
        }
        if self.buffered() == 0 {
            self.underruns += 1;
            if self.underruns > PLC_MAX {
                // End of the talk spurt: an unanswered underrun says nothing
                // about the path, so it does not count against the target.
                self.playing = false;
                self.underruns = 0;
                self.slots.clear();
                return Playout::Silence;
            }
            // Wait for the packet instead of skipping it (time stretch).
            return Playout::Lost { fec: None };
        }
        self.underruns = 0;
        self.stable += 1;
        if self.stable >= STABLE_POPS {
            self.stable = 0;
            self.target = self.target.saturating_sub(1).max(MIN_TARGET);
        }
        match self.advance() {
            Some(p) => Playout::Packet(p),
            None => {
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
        assert_eq!(jb.pop(), Playout::Silence);
        jb.push(12, p(12));
        assert_eq!(jb.pop(), Playout::Packet(p(10)));
        jb.push(13, p(13));
        assert_eq!(jb.pop(), Playout::Packet(p(11)));
        assert_eq!(jb.pop(), Playout::Packet(p(12)));
        assert_eq!(jb.pop(), Playout::Packet(p(13)));
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
        jb.push(7, p(7));
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
    fn late_packet_is_waited_for_and_raises_target() {
        let mut jb = JitterBuffer::new();
        for s in 0..3u16 {
            jb.push(s, p(s as u8));
        }
        drain(&mut jb, 3);
        let t0 = jb.target();
        // Packet 3 is late: one concealed frame, then it still plays.
        assert_eq!(jb.pop(), Playout::Lost { fec: None });
        jb.push(3, p(3));
        assert_eq!(jb.pop(), Playout::Packet(p(3)));
        assert_eq!(jb.target(), t0 + 1);
    }

    #[test]
    fn target_adapts_within_bounds() {
        let mut jb = JitterBuffer::new();
        let mut seq = 0u16;
        // Repeated late arrivals: target climbs to the max, never beyond.
        for _ in 0..10 {
            jb.push(seq, p(0));
            seq = seq.wrapping_add(1);
            while jb.buffered() > 0 {
                jb.pop();
            }
            jb.pop(); // underrun
        }
        assert_eq!(jb.target(), MAX_TARGET);
        // A long stable stream lowers it to the minimum.
        for _ in 0..(STABLE_POPS as usize * (MAX_TARGET + 1)) {
            jb.push(seq, p(0));
            seq = seq.wrapping_add(1);
            jb.pop();
        }
        assert_eq!(jb.target(), MIN_TARGET);
    }

    #[test]
    fn end_of_spurt_conceals_then_silence_and_resumes() {
        let mut jb = JitterBuffer::new();
        for s in 1..=3u16 {
            jb.push(s, p(s as u8));
        }
        drain(&mut jb, 3);
        let t = jb.target();
        let tail = drain(&mut jb, 5);
        assert_eq!(tail[0], Playout::Lost { fec: None });
        assert_eq!(tail[2], Playout::Lost { fec: None });
        assert_eq!(tail[3], Playout::Silence);
        assert!(!jb.is_playing());
        assert_eq!(jb.target(), t);
        // Next spurt continues the sequence much later.
        for s in 4..=6u16 {
            jb.push(s, p(s as u8));
        }
        assert_eq!(jb.pop(), Playout::Packet(p(4)));
    }

    #[test]
    fn preroll_burst_is_kept_and_reported_as_excess() {
        let mut jb = JitterBuffer::new();
        for s in 100..115u16 {
            jb.push(s, p(s as u8));
        }
        assert_eq!(jb.pop(), Playout::Packet(p(100)));
        assert_eq!(jb.excess(), 14 - jb.target());
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
        jb.push(30_002, p(11));
        assert_eq!(jb.pop(), Playout::Packet(p(9)));
    }

    #[test]
    fn stale_packet_then_restarted_sender() {
        // Regression (qa B2): stale seq from the previous stream, then the
        // sender restarts at a lower seq.
        let mut jb = JitterBuffer::new();
        jb.push(1453, p(0));
        let mut played = 0;
        for s in 1000..1010u16 {
            jb.push(s, p(s as u8));
            if matches!(jb.pop(), Playout::Packet(_)) {
                played += 1;
            }
        }
        assert!(played >= 7, "{played}");
    }

    #[test]
    fn lone_packet_does_not_wait_forever() {
        let mut jb = JitterBuffer::new();
        jb.push(7, p(7));
        let out = drain(&mut jb, 6);
        assert!(out.contains(&Playout::Packet(p(7))));
    }

    #[test]
    fn latency_is_capped() {
        let mut jb = JitterBuffer::new();
        for s in 0..40u16 {
            jb.push(s, p(s as u8));
        }
        assert!(jb.buffered() <= MAX_BUFFERED);
        assert_eq!(jb.pop(), Playout::Packet(p(15)));
    }
}
