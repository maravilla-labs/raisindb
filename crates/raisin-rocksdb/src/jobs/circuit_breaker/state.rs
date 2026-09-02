//! One breaker's state machine, and the policy that drives it.
//!
//! Split from the registry so the transitions can be read (and tested) without
//! the map and the locking around them.

use std::time::{Duration, Instant};

/// When a breaker opens, how long it stays open, and how long a probe may be
/// outstanding before it is presumed lost.
#[derive(Debug, Clone, Copy)]
pub struct BreakerPolicy {
    /// Consecutive UPSTREAM faults that open the breaker.
    pub failure_threshold: u32,
    /// How long `Open` lasts before one probe is allowed through.
    pub cooldown: Duration,
    /// How long a half-open probe may be outstanding before another caller may
    /// take its place.
    ///
    /// Without this, a probe holder that never reports — it panicked, its job
    /// was cancelled, it returned early for an unrelated reason — would wedge
    /// the breaker half-open forever, which is a stall with no outage behind
    /// it. The deadline is generous: a re-probe costs one request, while a
    /// premature one costs the property that a probe is single.
    pub probe_timeout: Duration,
}

impl Default for BreakerPolicy {
    /// Defaults chosen against the incident, not tuned in the abstract.
    ///
    /// * `5` consecutive faults: high enough that a single flaky request does
    ///   not park a healthy fleet, low enough that 476 identical 503s become
    ///   five.
    /// * `30s` cooldown: the outage window was minutes, so a parked job costs
    ///   at most one extra half-minute once the upstream returns, while a
    ///   shorter cooldown re-probes often enough to keep a struggling upstream
    ///   loaded.
    /// * `120s` probe timeout: comfortably longer than any single embedding
    ///   request, so a slow probe is never mistaken for a lost one.
    ///
    /// They are constants rather than config because an operator has no way to
    /// pick better values than these until the admin surface (§D of the plan)
    /// shows them what the breakers are actually doing.
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            cooldown: Duration::from_secs(30),
            probe_timeout: Duration::from_secs(120),
        }
    }
}

/// What a breaker is currently doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    /// Attempts pass through.
    Closed,
    /// Attempts are parked; a probe is allowed once the cooldown elapses.
    Open,
    /// Exactly one attempt is out, deciding whether to close or re-open.
    HalfOpen,
}

/// A breaker's state as an outside reader sees it.
///
/// Deliberately owned and `Clone`: it crosses out of the registry's locks, and
/// an admin endpoint reading through a borrow would hold a `Mutex` while
/// serialising JSON.
#[derive(Debug, Clone)]
pub struct BreakerStatus {
    pub key: String,
    pub state: BreakerState,
    /// Consecutive upstream faults. Reset by any success.
    pub consecutive_failures: u32,
    /// The message of the most recent failure counted against this breaker.
    pub last_error: Option<String>,
    /// How long until a probe is allowed. `None` when the breaker is not open.
    pub next_probe_in: Option<Duration>,
}

/// The mutable half. Kept to `Copy` fields plus one `String` so the critical
/// section around it stays trivial.
#[derive(Debug)]
pub(super) struct Cell {
    state: BreakerState,
    consecutive_failures: u32,
    last_error: Option<String>,
    /// When the current `Open` period began. The cooldown is measured from
    /// here, so re-opening after a failed probe restarts it.
    opened_at: Option<Instant>,
    /// When the outstanding half-open probe was admitted.
    probe_started_at: Option<Instant>,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            state: BreakerState::Closed,
            consecutive_failures: 0,
            last_error: None,
            opened_at: None,
            probe_started_at: None,
        }
    }
}

impl Cell {
    /// `None` to admit; `Some(remaining)` to park.
    ///
    /// This is the only method that MUTATES on the admission path, because
    /// claiming the single probe is a state change: the caller that flips
    /// `Open` → `HalfOpen` is the probe, and every other caller keeps parking
    /// until it reports.
    pub(super) fn admit(&mut self, policy: &BreakerPolicy) -> Option<Duration> {
        match self.state {
            BreakerState::Closed => None,
            BreakerState::Open => {
                let elapsed = self.opened_at.map(|t| t.elapsed()).unwrap_or_default();
                if elapsed >= policy.cooldown {
                    self.state = BreakerState::HalfOpen;
                    self.probe_started_at = Some(Instant::now());
                    None
                } else {
                    Some(policy.cooldown - elapsed)
                }
            }
            BreakerState::HalfOpen => {
                // A probe is out. Admit a replacement only once the outstanding
                // one is past its deadline and therefore presumed lost.
                let elapsed = self
                    .probe_started_at
                    .map(|t| t.elapsed())
                    .unwrap_or(policy.probe_timeout);
                if elapsed >= policy.probe_timeout {
                    self.probe_started_at = Some(Instant::now());
                    None
                } else {
                    Some(policy.probe_timeout - elapsed)
                }
            }
        }
    }

    /// The read-only question: is this breaker OPEN right now?
    ///
    /// Never claims a probe and never mutates, so a job that is already
    /// admitted can ask it between units of work without stealing the probe
    /// slot from itself.
    pub(super) fn remaining_cooldown(&self, policy: &BreakerPolicy) -> Option<Duration> {
        if self.state != BreakerState::Open {
            return None;
        }
        let elapsed = self.opened_at.map(|t| t.elapsed()).unwrap_or_default();
        Some(policy.cooldown.saturating_sub(elapsed)).filter(|d| !d.is_zero())
    }

    pub(super) fn record_success(&mut self) {
        self.state = BreakerState::Closed;
        self.consecutive_failures = 0;
        self.last_error = None;
        self.opened_at = None;
        self.probe_started_at = None;
    }

    /// `upstream_fault` is [`raisin_error::Error::is_upstream_fault`], asked by
    /// the caller so the classification has exactly one implementation.
    pub(super) fn record_failure(
        &mut self,
        policy: &BreakerPolicy,
        upstream_fault: bool,
        message: String,
    ) {
        if !upstream_fault {
            // OUR fault (a 4xx: bad request, wrong model, an expired key that
            // belongs to ONE tenant). It must not open the breaker, and it must
            // not be counted towards opening it either — otherwise a tenant
            // with a broken config slowly parks everyone else's jobs, which is
            // the same harm by a slower route.
            //
            // A half-open probe that gets a 4xx CLOSES the breaker: the
            // upstream answered, which is precisely what the probe asked. The
            // request itself still fails and the job retries normally.
            if self.state == BreakerState::HalfOpen {
                self.record_success();
            }
            return;
        }

        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.last_error = Some(message);
        match self.state {
            // A failed probe re-opens immediately and restarts the cooldown —
            // one request per cooldown is the whole budget an unhealthy
            // upstream gets.
            BreakerState::HalfOpen => self.open(),
            BreakerState::Closed if self.consecutive_failures >= policy.failure_threshold => {
                self.open()
            }
            _ => {}
        }
    }

    fn open(&mut self) {
        self.state = BreakerState::Open;
        self.opened_at = Some(Instant::now());
        self.probe_started_at = None;
    }

    pub(super) fn status(&self, key: String, policy: &BreakerPolicy) -> BreakerStatus {
        BreakerStatus {
            key,
            state: self.state,
            consecutive_failures: self.consecutive_failures,
            last_error: self.last_error.clone(),
            next_probe_in: self.remaining_cooldown(policy),
        }
    }
}
