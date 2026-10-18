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

//! Re-queues delayed jobs that missed their dispatch.
//!
//! A delayed job (`register_job_at`, a retry backoff, a parked job) reaches a
//! worker through ONE queue entry, made by a task that sleeps until the job is
//! due. If that entry never happens, or a worker refuses the claim and drops
//! it, the job stays `Scheduled` for good: the watchdog only reaps running
//! jobs, the cleanup only sweeps finished ones, and nothing else looks. This
//! was observed as one-shot invocations armed a few minutes ahead sitting
//! `scheduled` for ever, past their `run_at`, with no error and no log line.
//!
//! The sweeper is the second path. Every few seconds it asks the registry for
//! `Scheduled` jobs that are overdue by more than a grace period and queues
//! them again. That is safe to do at any time because a claim is atomic: if a
//! job is in the queue twice, the second pop finds it already running and
//! drops the duplicate. It is also bounded: a job is re-queued at most once per
//! `REDISPATCH_EVERY`, so a genuine backlog (overdue because every worker is
//! busy) costs one extra queue entry a minute per waiting job, not a flood.

use chrono::{DateTime, Utc};
use raisin_storage::jobs::{JobId, JobRegistry};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use super::dispatcher::JobDispatcher;

/// How often the registry is checked.
const SWEEP_INTERVAL: Duration = Duration::from_secs(5);

/// How far past its due time a job must be before the sweeper steps in. Long
/// enough that a job whose own timer is merely late (a busy runtime) is left
/// to it, short enough that a lost one is found within the minute.
const OVERDUE_GRACE_SECS: i64 = 20;

/// Minimum spacing between two re-queues of the same job.
const REDISPATCH_EVERY: Duration = Duration::from_secs(120);

pub struct OverdueDispatchSweeper {
    registry: Arc<JobRegistry>,
    dispatcher: Arc<JobDispatcher>,
    shutdown: CancellationToken,
    /// When each overdue job was last re-queued.
    requeued: HashMap<JobId, Instant>,
}

impl OverdueDispatchSweeper {
    pub fn new(
        registry: Arc<JobRegistry>,
        dispatcher: Arc<JobDispatcher>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            registry,
            dispatcher,
            shutdown,
            requeued: HashMap::new(),
        }
    }

    pub async fn run(mut self) {
        tracing::info!(
            interval_secs = SWEEP_INTERVAL.as_secs(),
            grace_secs = OVERDUE_GRACE_SECS,
            "Overdue job sweeper started"
        );
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        // A tick missed while the runtime was busy is not worth catching up.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                _ = ticker.tick() => {
                    self.sweep_once(Utc::now(), Instant::now()).await;
                }
            }
        }
        tracing::info!("Overdue job sweeper stopped");
    }

    /// One pass. Returns how many jobs were queued again.
    pub async fn sweep_once(&mut self, now: DateTime<Utc>, at: Instant) -> usize {
        let overdue = self
            .registry
            .overdue_scheduled(now, chrono::Duration::seconds(OVERDUE_GRACE_SECS))
            .await;

        // Forget jobs that left the overdue set (claimed, cancelled, removed),
        // so the map cannot outgrow the set it describes.
        let still_overdue: std::collections::HashSet<&JobId> =
            overdue.iter().map(|job| &job.id).collect();
        self.requeued.retain(|id, _| still_overdue.contains(id));

        let mut requeued = 0;
        for job in &overdue {
            if let Some(last) = self.requeued.get(&job.id) {
                if at.saturating_duration_since(*last) < REDISPATCH_EVERY {
                    continue;
                }
            }
            let overdue_by_secs = job
                .next_retry_at
                .map(|due| (now - due).num_seconds())
                .unwrap_or_default();
            tracing::warn!(
                job_id = %job.id,
                job_type = %job.job_type,
                tenant = %job.tenant,
                overdue_by_secs,
                "Scheduled job is past its due time and was never claimed; queueing it again"
            );
            self.dispatcher
                .dispatch_categorized(
                    job.id.clone(),
                    job.job_type.default_priority(),
                    job.job_type.category(),
                    &job.tenant,
                )
                .await;
            self.requeued.insert(job.id.clone(), at);
            requeued += 1;
        }
        requeued
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_storage::jobs::JobType;

    fn setup() -> (
        Arc<JobRegistry>,
        OverdueDispatchSweeper,
        std::collections::HashMap<
            raisin_storage::jobs::JobCategory,
            super::super::dispatcher::JobReceiver,
        >,
    ) {
        let registry = Arc::new(JobRegistry::new());
        let (dispatcher, receivers) = JobDispatcher::new();
        let sweeper = OverdueDispatchSweeper::new(
            registry.clone(),
            Arc::new(dispatcher),
            CancellationToken::new(),
        );
        (registry, sweeper, receivers)
    }

    async fn delayed(registry: &JobRegistry, due: DateTime<Utc>) -> JobId {
        registry
            .register_job_at(JobType::IntegrityScan, "t".to_string(), due, Some(0))
            .await
            .unwrap()
    }

    fn queued(
        receivers: &std::collections::HashMap<
            raisin_storage::jobs::JobCategory,
            super::super::dispatcher::JobReceiver,
        >,
    ) -> Option<JobId> {
        receivers
            .get(&JobType::IntegrityScan.category())
            .expect("category receiver")
            .try_recv()
    }

    /// The reported failure: a job registered for a future moment whose only
    /// queue entry never happened is still `Scheduled` minutes after its due
    /// time. The sweeper must put it on the queue, and a worker must then be
    /// able to claim it.
    #[tokio::test]
    async fn a_delayed_job_that_was_never_queued_is_queued_once_overdue() {
        let (registry, mut sweeper, receivers) = setup();
        let job_id = delayed(&registry, Utc::now() - chrono::Duration::seconds(300)).await;

        assert_eq!(sweeper.sweep_once(Utc::now(), Instant::now()).await, 1);

        assert_eq!(queued(&receivers), Some(job_id.clone()));
        assert!(registry.try_claim_job(&job_id).await.unwrap());
    }

    /// A job that is not yet due, or is only just due, belongs to its own timer.
    #[tokio::test]
    async fn a_job_inside_the_grace_period_is_left_alone() {
        let (registry, mut sweeper, receivers) = setup();
        delayed(&registry, Utc::now() + chrono::Duration::seconds(300)).await;
        delayed(&registry, Utc::now() - chrono::Duration::seconds(2)).await;

        assert_eq!(sweeper.sweep_once(Utc::now(), Instant::now()).await, 0);
        assert_eq!(queued(&receivers), None);
    }

    /// An overdue job that is already running (or finished) is not queued.
    #[tokio::test]
    async fn a_job_that_was_claimed_is_not_queued_again() {
        let (registry, mut sweeper, receivers) = setup();
        let job_id = delayed(&registry, Utc::now() - chrono::Duration::seconds(300)).await;
        assert!(registry.try_claim_job(&job_id).await.unwrap());

        assert_eq!(sweeper.sweep_once(Utc::now(), Instant::now()).await, 0);
        assert_eq!(queued(&receivers), None);
    }

    /// A backlog (overdue because workers are busy, not because anything was
    /// lost) must not grow a queue entry per sweep.
    #[tokio::test]
    async fn a_job_is_queued_again_at_most_once_per_interval() {
        let (registry, mut sweeper, receivers) = setup();
        let job_id = delayed(&registry, Utc::now() - chrono::Duration::seconds(300)).await;
        let t0 = Instant::now();

        assert_eq!(sweeper.sweep_once(Utc::now(), t0).await, 1);
        assert_eq!(queued(&receivers), Some(job_id.clone()));

        // Still unclaimed one sweep later: left alone.
        assert_eq!(sweeper.sweep_once(Utc::now(), t0 + SWEEP_INTERVAL).await, 0);
        assert_eq!(queued(&receivers), None);

        // Still unclaimed after the interval: once more.
        assert_eq!(
            sweeper
                .sweep_once(Utc::now(), t0 + REDISPATCH_EVERY + SWEEP_INTERVAL)
                .await,
            1
        );
        assert_eq!(queued(&receivers), Some(job_id));
    }
}
