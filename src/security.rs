use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use subtle::ConstantTimeEq;

/// Compares two secrets in constant time with respect to their contents so a
/// network attacker cannot use response-timing differences to recover the
/// pairing token one byte at a time. Only the (non-secret) length is allowed
/// to short-circuit the comparison.
pub fn tokens_match(provided: &str, expected: &str) -> bool {
    if expected.is_empty() {
        // An unset/empty configured token must never be satisfied by any
        // request, including another empty header value.
        return false;
    }

    let provided = provided.as_bytes();
    let expected = expected.as_bytes();

    if provided.len() != expected.len() {
        return false;
    }

    provided.ct_eq(expected).into()
}

const MAX_FAILURES_PER_WINDOW: u32 = 8;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);
const LOCKOUT_DURATION: Duration = Duration::from_secs(60);

#[derive(Default)]
struct AttemptRecord {
    failures: u32,
    window_started_at: Option<Instant>,
    locked_until: Option<Instant>,
}

/// A simple per-IP fixed-window lockout for authenticated endpoints. This is
/// intentionally basic (in-memory, process-lifetime only) rather than a
/// general-purpose crate dependency, since the only goal is to make blind
/// token-guessing and pairing-notification spam noticeably expensive.
pub struct RateLimiter {
    attempts: Mutex<HashMap<IpAddr, AttemptRecord>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `Some(remaining)` if the given address is currently locked out.
    pub fn locked_out(&self, addr: IpAddr) -> Option<Duration> {
        let attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let record = attempts.get(&addr)?;
        let locked_until = record.locked_until?;
        let now = Instant::now();
        if locked_until > now {
            Some(locked_until - now)
        } else {
            None
        }
    }

    pub fn record_failure(&self, addr: IpAddr) {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let now = Instant::now();
        let record = attempts.entry(addr).or_default();

        let window_expired = record
            .window_started_at
            .map(|started| now.duration_since(started) > FAILURE_WINDOW)
            .unwrap_or(true);

        if window_expired {
            record.failures = 0;
            record.window_started_at = Some(now);
        }

        record.failures += 1;

        if record.failures >= MAX_FAILURES_PER_WINDOW {
            record.locked_until = Some(now + LOCKOUT_DURATION);
        }
    }

    pub fn record_success(&self, addr: IpAddr) {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        attempts.remove(&addr);
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_tokens_compare_equal() {
        assert!(tokens_match("same-token-value", "same-token-value"));
    }

    #[test]
    fn different_tokens_do_not_match() {
        assert!(!tokens_match("token-a", "token-b"));
    }

    #[test]
    fn different_length_tokens_do_not_match() {
        assert!(!tokens_match("short", "much-longer-token"));
    }

    #[test]
    fn empty_expected_token_never_matches() {
        assert!(!tokens_match("", ""));
        assert!(!tokens_match("anything", ""));
    }

    #[test]
    fn lockout_triggers_after_max_failures() {
        let limiter = RateLimiter::new();
        let addr: IpAddr = "127.0.0.1".parse().unwrap();

        for _ in 0..MAX_FAILURES_PER_WINDOW {
            assert!(limiter.locked_out(addr).is_none());
            limiter.record_failure(addr);
        }

        assert!(limiter.locked_out(addr).is_some());
    }

    #[test]
    fn success_clears_recorded_failures() {
        let limiter = RateLimiter::new();
        let addr: IpAddr = "127.0.0.1".parse().unwrap();

        limiter.record_failure(addr);
        limiter.record_failure(addr);
        limiter.record_success(addr);

        for _ in 0..MAX_FAILURES_PER_WINDOW {
            limiter.record_failure(addr);
        }
        assert!(limiter.locked_out(addr).is_some());
    }

    #[test]
    fn different_addresses_are_tracked_independently() {
        let limiter = RateLimiter::new();
        let a: IpAddr = "127.0.0.1".parse().unwrap();
        let b: IpAddr = "192.168.1.50".parse().unwrap();

        for _ in 0..MAX_FAILURES_PER_WINDOW {
            limiter.record_failure(a);
        }

        assert!(limiter.locked_out(a).is_some());
        assert!(limiter.locked_out(b).is_none());
    }
}
