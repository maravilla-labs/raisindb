// SPDX-License-Identifier: BSL-1.1

//! Rate limits and turn leases for anonymous visitor chats.
//!
//! In memory, per server process: a visitor's WebSocket lives on one server,
//! and these limits exist to make abuse expensive, not to be exact across a
//! cluster. (A cluster multiplies a per-IP allowance by its size; the
//! per-conversation token budget and message cap are persisted and exact.)

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

/// Sliding-window counters and turn leases, shared by every connection.
#[derive(Default)]
pub struct VisitorLimits {
    windows: DashMap<String, VecDeque<Instant>>,
    leases: DashMap<String, Vec<(String, Instant)>>,
    locks: DashMap<String, Arc<tokio::sync::Mutex<()>>>,
}

/// Above this many tracked keys, stale ones are swept on the next hit.
const SWEEP_ABOVE: usize = 50_000;

impl VisitorLimits {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one event for `key` if fewer than `limit` happened within
    /// `window` before `now`. Returns whether it was allowed.
    pub fn hit(&self, key: &str, limit: u32, window: Duration, now: Instant) -> bool {
        if limit == 0 {
            return false;
        }
        if self.windows.len() > SWEEP_ABOVE {
            self.windows.retain(|_, q| {
                q.back()
                    .is_some_and(|t| now.saturating_duration_since(*t) < window)
            });
        }
        let mut q = self.windows.entry(key.to_string()).or_default();
        while q
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= window)
        {
            q.pop_front();
        }
        if q.len() >= limit as usize {
            return false;
        }
        q.push_back(now);
        true
    }

    /// Take a turn slot for `conversation` in `session`. Refused when that
    /// conversation already has a turn in flight, or the session holds
    /// `max` live slots. A slot expires after `lease`, so a hung turn frees
    /// it on its own.
    pub fn acquire_turn(
        &self,
        session: &str,
        conversation: &str,
        max: u32,
        lease: Duration,
        now: Instant,
    ) -> bool {
        let mut slots = self.leases.entry(session.to_string()).or_default();
        slots.retain(|(_, until)| *until > now);
        if slots.iter().any(|(c, _)| c == conversation) || slots.len() >= max as usize {
            return false;
        }
        slots.push((conversation.to_string(), now + lease));
        true
    }

    /// Free the slot `conversation` holds in `session` (its turn finished).
    pub fn release_turn(&self, session: &str, conversation: &str) {
        if let Some(mut slots) = self.leases.get_mut(session) {
            slots.retain(|(c, _)| c != conversation);
        }
        self.leases.remove_if(session, |_, slots| slots.is_empty());
    }

    /// The lock that serialises one session's writes (its counters live on
    /// its home node, read-modify-write).
    pub fn session_lock(&self, session: &str) -> Arc<tokio::sync::Mutex<()>> {
        if self.locks.len() > SWEEP_ABOVE {
            self.locks.retain(|_, l| Arc::strong_count(l) > 1);
        }
        self.locks
            .entry(session.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn a_window_allows_exactly_its_limit() {
        let l = VisitorLimits::new();
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(l.hit("s", 3, MIN, t0));
        }
        assert!(!l.hit("s", 3, MIN, t0), "the fourth in a minute is refused");
        assert!(l.hit("other", 3, MIN, t0), "keys are independent");
        assert!(
            l.hit("s", 3, MIN, t0 + MIN),
            "a minute later the window has moved on"
        );
    }

    #[test]
    fn a_zero_limit_allows_nothing() {
        assert!(!VisitorLimits::new().hit("s", 0, MIN, Instant::now()));
    }

    #[test]
    fn one_turn_per_conversation_and_a_cap_per_session() {
        let l = VisitorLimits::new();
        let t0 = Instant::now();
        let lease = Duration::from_secs(120);
        assert!(l.acquire_turn("s", "c1", 2, lease, t0));
        assert!(
            !l.acquire_turn("s", "c1", 2, lease, t0),
            "c1 is still answering"
        );
        assert!(l.acquire_turn("s", "c2", 2, lease, t0));
        assert!(!l.acquire_turn("s", "c3", 2, lease, t0), "the session cap");
        l.release_turn("s", "c1");
        assert!(l.acquire_turn("s", "c1", 2, lease, t0), "released on done");
    }

    #[test]
    fn a_hung_turn_frees_its_slot_when_the_lease_runs_out() {
        let l = VisitorLimits::new();
        let t0 = Instant::now();
        let lease = Duration::from_secs(120);
        assert!(l.acquire_turn("s", "c1", 1, lease, t0));
        assert!(!l.acquire_turn("s", "c1", 1, lease, t0 + Duration::from_secs(119)));
        assert!(l.acquire_turn("s", "c1", 1, lease, t0 + lease));
    }
}
