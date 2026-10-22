//! Throttles speaking indicators to ~5 Hz per peer.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use lc_types::events::VoiceSpeakingPayload;

pub const MIN_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Default)]
pub struct SpeakingTracker {
    last: HashMap<String, (Instant, bool)>,
}

impl SpeakingTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a payload to emit: on state change or periodic level updates
    /// while speaking, never more often than [`MIN_INTERVAL`] per peer.
    pub fn update(
        &mut self,
        peer_id: &str,
        speaking: bool,
        level: f32,
        now: Instant,
    ) -> Option<VoiceSpeakingPayload> {
        if let Some((at, was)) = self.last.get(peer_id) {
            if now.duration_since(*at) < MIN_INTERVAL || (!speaking && !*was) {
                return None;
            }
        } else if !speaking {
            return None;
        }
        self.last.insert(peer_id.to_owned(), (now, speaking));
        Some(VoiceSpeakingPayload {
            peer_id: peer_id.to_owned(),
            speaking,
            level: if speaking { level.clamp(0.0, 1.0) } else { 0.0 },
        })
    }

    /// `speaking: false` for everyone still marked as speaking (voice stopped).
    pub fn reset(&mut self) -> Vec<VoiceSpeakingPayload> {
        let out = self
            .last
            .drain()
            .filter(|(_, (_, s))| *s)
            .map(|(peer_id, _)| VoiceSpeakingPayload {
                peer_id,
                speaking: false,
                level: 0.0,
            })
            .collect();
        out
    }

    /// Peers we have state for (to report those that disappeared).
    pub fn known(&self) -> Vec<String> {
        self.last.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttles_and_reports_changes() {
        let mut t = SpeakingTracker::new();
        let t0 = Instant::now();
        assert!(t.update("a", false, 0.0, t0).is_none());
        assert!(t.update("a", true, 0.5, t0).is_some());
        assert!(t
            .update("a", true, 0.6, t0 + Duration::from_millis(50))
            .is_none());
        // Stop within the window is deferred, then delivered.
        assert!(t
            .update("a", false, 0.0, t0 + Duration::from_millis(100))
            .is_none());
        let p = t
            .update("a", false, 0.0, t0 + Duration::from_millis(210))
            .unwrap();
        assert!(!p.speaking);
        assert!(t
            .update("a", false, 0.0, t0 + Duration::from_millis(500))
            .is_none());
        // 10 s of speaking → at most ~5 Hz.
        let n = (0..1000)
            .filter_map(|i| t.update("a", true, 0.5, t0 + Duration::from_millis(1000 + i * 10)))
            .count();
        assert!(n <= 51, "{n}");
        assert_eq!(t.reset().len(), 1);
    }
}
