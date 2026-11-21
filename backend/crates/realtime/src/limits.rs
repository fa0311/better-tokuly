//! Per-connection rate limiting.
use std::time::Instant;

/// A token bucket: `burst` frames at once, refilled at `per_second`.
/// Kept in memory per connection, so checking it costs nothing.
#[derive(Debug)]
pub(crate) struct TokenBucket {
    capacity: f64,
    per_second: f64,
    tokens: f64,
    refilled_at: Instant,
}

impl TokenBucket {
    pub(crate) fn new(burst: u32, per_second: f64) -> Self {
        let capacity = f64::from(burst.max(1));
        Self { capacity, per_second, tokens: capacity, refilled_at: Instant::now() }
    }

    /// Takes one token if there is one.
    pub(crate) fn allow(&mut self) -> bool {
        self.allow_at(Instant::now())
    }

    fn allow_at(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.refilled_at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_second).min(self.capacity);
        self.refilled_at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn burst_then_steady_rate() {
        let mut bucket = TokenBucket::new(3, 2.0);
        let start = bucket.refilled_at;
        assert!((0..3).all(|_| bucket.allow_at(start)));
        assert!(!bucket.allow_at(start));
        // Half a second later one token (2/s) is back, not more.
        assert!(bucket.allow_at(start + Duration::from_millis(500)));
        assert!(!bucket.allow_at(start + Duration::from_millis(500)));
    }

    #[test]
    fn idle_time_never_exceeds_the_burst() {
        let mut bucket = TokenBucket::new(2, 100.0);
        let later = bucket.refilled_at + Duration::from_secs(3600);
        assert!(bucket.allow_at(later));
        assert!(bucket.allow_at(later));
        assert!(!bucket.allow_at(later));
    }
}
