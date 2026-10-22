//! Per-key token-bucket rate limiter (in memory).

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Upper bound of tracked keys; on overflow the table is reset (fail-open for
/// memory safety on a small host, the limiter refills from scratch).
const MAX_KEYS: usize = 50_000;

#[derive(Debug)]
pub struct RateLimiter<K> {
    per_sec: f64,
    burst: f64,
    buckets: Mutex<HashMap<K, (f64, Instant)>>,
}

impl<K: Eq + Hash + Clone> RateLimiter<K> {
    /// `per_min` steady requests per minute, `burst` bucket size.
    pub fn per_minute(per_min: u32, burst: u32) -> Self {
        Self {
            per_sec: f64::from(per_min.max(1)) / 60.0,
            burst: f64::from(burst.max(1)),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// `per_hour` steady requests per hour, `burst` bucket size.
    pub fn per_hour(per_hour: u32, burst: u32) -> Self {
        Self {
            per_sec: f64::from(per_hour.max(1)) / 3600.0,
            burst: f64::from(burst.max(1)),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes one token for `key`. `Err(secs)` = retry after `secs`.
    pub fn check(&self, key: &K) -> Result<(), u64> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: &K, now: Instant) -> Result<(), u64> {
        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= MAX_KEYS && !map.contains_key(key) {
            map.clear();
        }
        let entry = map.entry(key.clone()).or_insert((self.burst, now));
        let elapsed = now.saturating_duration_since(entry.1).as_secs_f64();
        entry.0 = (entry.0 + elapsed * self.per_sec).min(self.burst);
        entry.1 = now;
        if entry.0 >= 1.0 {
            entry.0 -= 1.0;
            Ok(())
        } else {
            let wait = ((1.0 - entry.0) / self.per_sec).ceil();
            Err(wait.max(1.0) as u64)
        }
    }

    /// Drops buckets that are full again (idle keys).
    pub fn sweep(&self) {
        let now = Instant::now();
        let full_after = Duration::from_secs_f64(self.burst / self.per_sec);
        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, (_, last)| now.saturating_duration_since(*last) < full_after);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_refill() {
        let rl = RateLimiter::per_minute(60, 3);
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(rl.check_at(&1u8, t0).is_ok());
        }
        assert_eq!(rl.check_at(&1u8, t0), Err(1));
        // Other keys are independent.
        assert!(rl.check_at(&2u8, t0).is_ok());
        // One token per second.
        assert!(rl.check_at(&1u8, t0 + Duration::from_millis(1_100)).is_ok());
        assert!(rl
            .check_at(&1u8, t0 + Duration::from_millis(1_200))
            .is_err());
    }
}
