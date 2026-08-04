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

//! Running something on exactly one node of a cluster.
//!
//! The first renewable lease in this codebase. Every existing lease user takes a
//! short one, does its work and releases — nothing has needed to *hold* one
//! before, which is why `LockManager::renew` had no production caller.
//!
//! The lease IS the heartbeat. There is deliberately no separate liveness
//! record: the holder renews, and any other node's `try_acquire` returns
//! `Some(guard)` — with a higher fencing token — the moment the TTL lapses. That
//! one call is the atomic answer to "is the leader really gone", from the same
//! source of truth as the lease itself. A second heartbeat store could disagree
//! with the lock, which is the mirrored-state failure this codebase keeps paying
//! for.
//!
//! Two things the lock layer does not do for you, both handled here:
//!
//! - **`renew() -> Ok(false)` is the only signal that you have been fenced.**
//!   There is no callback and no watch. A holder that ignores it keeps working
//!   alongside the new leader.
//! - **[`LockGuard`](crate::LockGuard) has no `Drop` impl.** Release is explicit,
//!   on demotion and on cancellation, or the work stays parked until the TTL
//!   expires on its own.

use std::future::Future;
use std::time::Duration;

use crate::{LockManagerHandle, Result};

/// Fraction of the TTL between renewals.
///
/// A third, not a half: one slow round trip to Redis must not be enough to lose
/// a lease that was healthy, and at TTL/2 a single delayed renewal leaves no
/// margin at all.
const RENEW_DIVISOR: u32 = 3;

/// Elects one holder of `key` and runs work while it holds it.
pub struct LeaseElection {
    manager: LockManagerHandle,
    key: String,
    owner: String,
    ttl: Duration,
}

impl LeaseElection {
    /// Build an election for one lease key.
    ///
    /// `owner` is diagnostic only — no backend reads it back, and `release` and
    /// `renew` match on the fencing token. Make it something a log reader can
    /// use to tell two nodes apart.
    pub fn new(
        manager: LockManagerHandle,
        key: impl Into<String>,
        owner: impl Into<String>,
        ttl: Duration,
    ) -> Self {
        Self {
            manager,
            key: key.into(),
            owner: owner.into(),
            ttl,
        }
    }

    /// Stand for election until `cancel` completes.
    ///
    /// While elected, `work` runs. It is **dropped** on demotion or
    /// cancellation, which is how a held-open connection gets closed: there is
    /// no signal to pass down, because dropping the future cancels it at its
    /// next await point.
    ///
    /// `work` returning on its own is treated as "done for this term" — the
    /// lease is released and the loop stands again. A supervisor that wants to
    /// hold a term should not return.
    pub async fn run<C, F, Fut>(&self, cancel: C, work: F) -> Result<()>
    where
        C: Future<Output = ()>,
        F: Fn() -> Fut,
        Fut: Future<Output = ()>,
    {
        tokio::pin!(cancel);

        loop {
            if let Some(guard) = self
                .manager
                .try_acquire(&self.key, &self.owner, self.ttl)
                .await?
            {
                tracing::debug!(key = %self.key, owner = %self.owner, "elected");
                let outcome = self.hold(&mut cancel, guard.token, &work).await;

                // Explicit, because LockGuard has no Drop impl. Best-effort: if
                // we were already fenced this is a no-op against the new holder.
                let _ = self.manager.release(&self.key, guard.token).await;

                match outcome {
                    Term::Cancelled => return Ok(()),
                    Term::Fenced => {
                        tracing::info!(key = %self.key, "lease was taken over; standing down");
                    }
                    Term::Finished => {}
                }
            }

            // Standing by. Poll at half the TTL so a lapsed lease is picked up
            // promptly without every node hammering the backend.
            let wait = jittered(self.ttl / 2);
            tokio::select! {
                _ = cancel.as_mut() => return Ok(()),
                _ = tokio::time::sleep(wait) => {}
            }
        }
    }

    /// Hold the lease, renewing, until the term ends.
    async fn hold<C, F, Fut>(
        &self,
        cancel: &mut std::pin::Pin<&mut C>,
        token: u64,
        work: &F,
    ) -> Term
    where
        C: Future<Output = ()>,
        F: Fn() -> Fut,
        Fut: Future<Output = ()>,
    {
        let running = work();
        tokio::pin!(running);

        let mut ticker = tokio::time::interval(self.ttl / RENEW_DIVISOR);
        // The first tick is immediate; skip it, we only just acquired.
        ticker.tick().await;
        // A renewal missed under load must not queue up a burst of catch-up
        // renewals — one late renewal is the signal, not N.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = cancel.as_mut() => return Term::Cancelled,
                _ = &mut running => return Term::Finished,
                _ = ticker.tick() => {
                    match self.manager.renew(&self.key, token, self.ttl).await {
                        // THE demotion signal. Returning here drops `running`,
                        // which cancels the work at its next await point.
                        Ok(false) => return Term::Fenced,
                        Ok(true) => {}
                        // A backend that is unreachable is not proof we lost the
                        // lease, but we can no longer prove we hold it either.
                        // Standing down is the safe reading: two holders is
                        // worse than none.
                        Err(e) => {
                            tracing::warn!(key = %self.key, error = %e, "lease renewal failed; standing down");
                            return Term::Fenced;
                        }
                    }
                }
            }
        }
    }
}

/// How a term ended.
enum Term {
    /// The caller cancelled; stop entirely.
    Cancelled,
    /// Someone else holds the lease now.
    Fenced,
    /// The work returned by itself.
    Finished,
}

/// Spread a delay over `[d/2, d]` so N nodes do not retry in lockstep.
fn jittered(base: Duration) -> Duration {
    let millis = base.as_millis().min(u64::MAX as u128) as u64;
    if millis < 2 {
        return base;
    }
    let half = millis / 2;
    Duration::from_millis(half + (entropy() % (millis - half + 1)))
}

/// Cheap non-cryptographic entropy; backoff spread is not a security decision.
fn entropy() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos
        .wrapping_mul(6364136223846793005)
        .wrapping_add(COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InProcessLockManager;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn manager() -> LockManagerHandle {
        Arc::new(InProcessLockManager::new())
    }

    #[tokio::test]
    async fn the_elected_node_runs_the_work() {
        let ran = Arc::new(AtomicUsize::new(0));
        let counter = ran.clone();
        let election = LeaseElection::new(manager(), "k", "node-a", Duration::from_secs(30));

        // Cancellation must be able to fire WHILE run() is holding a term —
        // a signal that can only arrive after run() returns would deadlock.
        election
            .run(
                async { tokio::time::sleep(Duration::from_millis(80)).await },
                move || {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        // Hold the term open; cancellation drops this future.
                        std::future::pending::<()>().await;
                    }
                },
            )
            .await
            .expect("election must not fail");

        assert_eq!(ran.load(Ordering::SeqCst), 1, "work ran exactly once");
    }

    /// Cancelling must RELEASE the lease, not leave it parked until the TTL —
    /// LockGuard has no Drop impl, so nothing else would.
    #[tokio::test]
    async fn cancelling_releases_the_lease_for_the_next_node() {
        let manager = manager();
        let election = LeaseElection::new(manager.clone(), "k", "node-a", Duration::from_secs(300));

        election
            .run(
                async { tokio::time::sleep(Duration::from_millis(50)).await },
                || async { std::future::pending::<()>().await },
            )
            .await
            .expect("election must not fail");

        // With a 300s TTL, only an explicit release can make this succeed.
        assert!(
            manager
                .try_acquire("k", "node-b", Duration::from_secs(30))
                .await
                .unwrap()
                .is_some(),
            "the lease must be free immediately after cancellation"
        );
    }

    /// A second node must not run the work while the first holds a live lease.
    #[tokio::test]
    async fn a_second_node_does_not_run_while_the_lease_is_held() {
        let manager = manager();
        let held = manager
            .try_acquire("k", "node-a", Duration::from_secs(30))
            .await
            .unwrap()
            .expect("node-a wins");

        let ran = Arc::new(AtomicUsize::new(0));
        let counter = ran.clone();
        let election = LeaseElection::new(manager.clone(), "k", "node-b", Duration::from_secs(30));

        // Give node-b a moment to stand and be refused, then cancel it.
        let result = tokio::time::timeout(
            Duration::from_millis(200),
            election.run(std::future::pending::<()>(), move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            }),
        )
        .await;

        assert!(result.is_err(), "node-b should still be standing by");
        assert_eq!(ran.load(Ordering::SeqCst), 0, "node-b must not have run");
        let _ = held;
    }

    /// Takeover after a lapse must mint a HIGHER fencing token — that is what
    /// makes the old holder's renew fail.
    #[tokio::test]
    async fn a_lapsed_lease_is_taken_over_with_a_higher_token() {
        let manager = manager();
        let first = manager
            .try_acquire("k", "node-a", Duration::from_millis(30))
            .await
            .unwrap()
            .expect("node-a wins");

        tokio::time::sleep(Duration::from_millis(60)).await;

        let second = manager
            .try_acquire("k", "node-b", Duration::from_secs(30))
            .await
            .unwrap()
            .expect("node-b takes over after the lapse");
        assert!(second.token > first.token, "fencing tokens must increase");

        // And the fenced holder learns about it exactly one way.
        let renewed = manager
            .renew("k", first.token, Duration::from_secs(30))
            .await
            .unwrap();
        assert!(!renewed, "a fenced holder's renew must return Ok(false)");
    }

    /// The fenced holder must not be able to release the new holder's lease.
    #[tokio::test]
    async fn a_fenced_holder_cannot_release_the_new_lease() {
        let manager = manager();
        let first = manager
            .try_acquire("k", "node-a", Duration::from_millis(30))
            .await
            .unwrap()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        let second = manager
            .try_acquire("k", "node-b", Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();

        assert!(!manager.release("k", first.token).await.unwrap());
        assert!(manager.release("k", second.token).await.unwrap());
    }

    #[test]
    fn jitter_stays_within_half_the_base() {
        for _ in 0..200 {
            let d = jittered(Duration::from_millis(1000));
            assert!(d >= Duration::from_millis(500) && d <= Duration::from_millis(1000));
        }
    }
}
