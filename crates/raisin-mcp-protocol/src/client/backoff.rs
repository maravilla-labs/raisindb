// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! How long to wait before trying a dropped stream again.
//!
//! Modelled on `RetryConfig::delay_for_attempt` in `raisin-replication`, the
//! only jittered backoff in the workspace, with its two flaws fixed:
//!
//! - **It panics.** `2u64.pow(attempt - 1)` overflows in debug builds once the
//!   attempt count passes 64. A subscription that has been failing for a day
//!   reaches that; a replication retry capped at 10 attempts never does.
//! - **Its jitter only ever adds.** `capped + rand(0..10%)` shifts every node's
//!   delay in the same direction, so N nodes reconnecting after the same outage
//!   stay clustered. Jitter here is symmetric — full jitter over `[base/2, base]`
//!   — which is what actually spreads a herd.

use std::time::Duration;

/// First delay after a drop.
pub const DEFAULT_BASE_DELAY: Duration = Duration::from_millis(500);
/// Ceiling on the delay, however long the outage lasts.
pub const DEFAULT_MAX_DELAY: Duration = Duration::from_secs(60);

/// Exponential backoff with symmetric jitter.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    attempt: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_DELAY, DEFAULT_MAX_DELAY)
    }
}

impl Backoff {
    /// Build a backoff between `base` and `max`.
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max,
            attempt: 0,
        }
    }

    /// Forget the failures — call after a stream stays up long enough to count.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// How many consecutive failures have been recorded.
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// The next delay, advancing the attempt counter.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.peek();
        // Saturating, so a stream that has been down for weeks keeps returning
        // `max` instead of wrapping to zero and hammering the remote server.
        self.attempt = self.attempt.saturating_add(1);
        delay
    }

    /// The delay this attempt would produce, without advancing.
    fn peek(&self) -> Duration {
        // Shift rather than `pow`, and stop doubling once the ceiling is passed.
        // `1u64 << 63` is the largest safe shift; beyond it the cap applies
        // anyway, so there is nothing to compute.
        let ceiling = self.max.as_millis().min(u64::MAX as u128) as u64;
        let base = self.base.as_millis().min(u64::MAX as u128) as u64;
        let scaled = if self.attempt >= 63 {
            ceiling
        } else {
            base.saturating_mul(1u64 << self.attempt).min(ceiling)
        };
        Duration::from_millis(jitter(scaled))
    }
}

/// Full jitter: a uniform point in `[value / 2, value]`.
///
/// Halving the floor is deliberate. Jitter that only ever *adds* to the delay
/// leaves every caller's wait ordered the same way it started, which is the
/// thundering herd it was supposed to break up.
fn jitter(value_ms: u64) -> u64 {
    if value_ms < 2 {
        return value_ms;
    }
    let half = value_ms / 2;
    half + (entropy() % (value_ms - half + 1))
}

/// A cheap, non-cryptographic entropy source.
///
/// The clock's sub-millisecond component, mixed with a process-local counter so
/// two calls in the same nanosecond still differ. Adding the `rand` crate for
/// backoff jitter would be the only use of it in this crate, and this is not a
/// security decision — an attacker who can predict a reconnect delay learns
/// nothing worth having.
fn entropy() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let tick = COUNTER.fetch_add(1, Ordering::Relaxed);
    // A cheap mix so the low bits are not just the counter's.
    nanos.wrapping_mul(6364136223846793005).wrapping_add(tick)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delay_grows_and_then_stops_at_the_ceiling() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(10));
        let mut previous = Duration::ZERO;
        for _ in 0..6 {
            let delay = backoff.next_delay();
            assert!(delay <= Duration::from_secs(10), "never past the ceiling");
            previous = delay;
        }
        let _ = previous;
        // Far past the point where doubling would exceed the ceiling.
        for _ in 0..20 {
            assert!(backoff.next_delay() <= Duration::from_secs(10));
        }
    }

    /// The flaw inherited from the replication implementation: `2u64.pow(n)`
    /// panics in debug once n passes 64. A stream failing for a day gets there.
    #[test]
    fn a_very_long_outage_does_not_panic_or_wrap() {
        let mut backoff = Backoff::new(Duration::from_millis(500), Duration::from_secs(60));

        // The ramp: the first attempts are legitimately short.
        for _ in 0..8 {
            assert!(backoff.next_delay() <= Duration::from_secs(60));
        }

        // Far past the point where doubling would overflow. The inherited
        // implementation panics here in debug; wrapping would instead collapse
        // the delay to zero and hammer the remote server.
        for _ in 0..5_000 {
            let delay = backoff.next_delay();
            assert!(
                delay >= Duration::from_secs(30),
                "a saturated backoff must stay in the ceiling's jitter band, got {delay:?}"
            );
            assert!(delay <= Duration::from_secs(60));
        }
    }

    /// Jitter must spread both ways. An additive-only implementation would put
    /// every sample in the top decile and never below the nominal value.
    #[test]
    fn jitter_is_symmetric_around_the_nominal_delay() {
        let samples: Vec<u64> = (0..500).map(|_| jitter(1000)).collect();
        assert!(samples.iter().all(|v| (500..=1000).contains(v)));
        assert!(
            samples.iter().any(|v| *v < 750),
            "some samples must fall BELOW the nominal delay"
        );
        assert!(samples.iter().any(|v| *v > 750), "and some above");
    }

    #[test]
    fn reset_returns_to_the_base_delay() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(60));
        for _ in 0..10 {
            backoff.next_delay();
        }
        assert_eq!(backoff.attempt(), 10);
        backoff.reset();
        assert_eq!(backoff.attempt(), 0);
        assert!(backoff.next_delay() <= Duration::from_millis(100));
    }

    #[test]
    fn tiny_values_do_not_divide_by_zero() {
        assert_eq!(jitter(0), 0);
        assert_eq!(jitter(1), 1);
        assert!((1..=2).contains(&jitter(2)));
    }
}
