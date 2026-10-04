//! Per-engine backoff, after SearXNG's engine suspension: an engine that fails stops being asked
//! for a while, so a blocked one costs nothing instead of eating the timeout on every search.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// Captcha / HTTP 403 / 429: usually lasts a while.
    Blocked,
    /// Timeout, network error, unexpected response.
    Broken,
}

impl Failure {
    fn base(self) -> Duration {
        match self {
            Failure::Blocked => Duration::from_secs(10 * 60),
            Failure::Broken => Duration::from_secs(30),
        }
    }
    fn cap(self) -> Duration {
        match self {
            Failure::Blocked => Duration::from_secs(60 * 60),
            Failure::Broken => Duration::from_secs(15 * 60),
        }
    }
}

#[derive(Default)]
struct State {
    fails: u32,
    until: Option<Instant>,
}

#[derive(Default)]
pub struct Suspend {
    inner: Mutex<HashMap<String, State>>,
}

impl Suspend {
    /// How much longer this engine is suspended, if at all.
    pub fn remaining(&self, engine: &str, now: Instant) -> Option<Duration> {
        let map = self.inner.lock().unwrap();
        let until = map.get(engine)?.until?;
        until.checked_duration_since(now).filter(|d| !d.is_zero())
    }

    pub fn record_ok(&self, engine: &str) {
        self.inner.lock().unwrap().remove(engine);
    }

    pub fn record_failure(&self, engine: &str, kind: Failure, now: Instant) -> Duration {
        let mut map = self.inner.lock().unwrap();
        let state = map.entry(engine.to_string()).or_default();
        state.fails += 1;
        let factor = 1u32 << (state.fails - 1).min(10);
        let wait = (kind.base() * factor).min(kind.cap());
        state.until = Some(now + wait);
        wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_back_off_exponentially_up_to_a_cap() {
        let s = Suspend::default();
        let now = Instant::now();
        assert_eq!(
            s.record_failure("e", Failure::Broken, now),
            Duration::from_secs(30)
        );
        assert_eq!(
            s.record_failure("e", Failure::Broken, now),
            Duration::from_secs(60)
        );
        assert_eq!(
            s.record_failure("e", Failure::Broken, now),
            Duration::from_secs(120)
        );
        for _ in 0..20 {
            s.record_failure("e", Failure::Broken, now);
        }
        assert_eq!(
            s.record_failure("e", Failure::Broken, now),
            Duration::from_secs(15 * 60)
        );
    }

    #[test]
    fn blocked_is_suspended_longer_than_broken() {
        let s = Suspend::default();
        let now = Instant::now();
        assert_eq!(
            s.record_failure("blocked", Failure::Blocked, now),
            Duration::from_secs(600)
        );
        assert_eq!(
            s.record_failure("blocked", Failure::Blocked, now),
            Duration::from_secs(1200)
        );
    }

    #[test]
    fn suspension_expires_and_success_forgives() {
        let s = Suspend::default();
        let now = Instant::now();
        s.record_failure("e", Failure::Broken, now);
        assert!(s.remaining("e", now + Duration::from_secs(10)).is_some());
        assert!(s.remaining("e", now + Duration::from_secs(31)).is_none());
        s.record_failure("e", Failure::Broken, now);
        s.record_ok("e");
        assert!(s.remaining("e", now).is_none());
        assert_eq!(
            s.record_failure("e", Failure::Broken, now),
            Duration::from_secs(30),
            "streak was reset"
        );
    }

    #[test]
    fn engines_are_independent() {
        let s = Suspend::default();
        let now = Instant::now();
        s.record_failure("a", Failure::Blocked, now);
        assert!(s.remaining("b", now).is_none());
    }
}
