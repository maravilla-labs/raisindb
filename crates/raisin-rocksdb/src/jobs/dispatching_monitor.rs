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

//! Auto-dispatching monitor for the job system
//!
//! Automatically dispatches newly registered jobs to the correct category pool.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use raisin_storage::jobs::{
    JobCategory, JobEvent, JobId, JobInfo, JobMonitor, JobPriority, JobStatus,
};
use std::sync::Arc;

use super::dispatcher::JobDispatcher;

/// Sleep until `now()` reaches `due`.
///
/// `tokio::time::sleep` measures a monotonic clock, while a job's due time and
/// the claim that checks it (`JobRegistry::try_claim_job`) read the wall
/// clock. The two disagree by whatever the wall clock was slewed or stepped by
/// while the timer ran, so a sleep of exactly `due - now` can wake a hair
/// BEFORE the wall clock says the job is due, and the claim then refuses it —
/// consuming the only queue entry the job had. Re-checking the wall clock after
/// each sleep makes an early wake a short extra sleep instead.
async fn sleep_until_due_with(due: DateTime<Utc>, now: impl Fn() -> DateTime<Utc>) {
    loop {
        let remaining = due - now();
        if remaining <= chrono::Duration::zero() {
            return;
        }
        // At least a millisecond, or a sub-millisecond remainder would spin.
        let wait = remaining
            .to_std()
            .unwrap_or_default()
            .max(std::time::Duration::from_millis(1));
        tokio::time::sleep(wait).await;
    }
}

/// Queue `job_id` for its category pool once the wall clock reaches `due`.
///
/// The one place a delayed job is held back, used for a new job, a requeued
/// one and a job restored at boot, so none of them can reach a worker early.
pub(crate) async fn dispatch_when_due(
    dispatcher: Arc<JobDispatcher>,
    job_id: JobId,
    priority: JobPriority,
    category: JobCategory,
    tenant: String,
    due: DateTime<Utc>,
) {
    sleep_until_due_with(due, Utc::now).await;
    dispatcher
        .dispatch_categorized(job_id, priority, category, &tenant)
        .await;
}

/// Monitor that automatically dispatches newly created jobs to worker queues
///
/// Routes jobs to the correct category pool based on their job type's category.
pub struct DispatchingMonitor {
    dispatcher: Arc<JobDispatcher>,
}

impl DispatchingMonitor {
    pub fn new(dispatcher: Arc<JobDispatcher>) -> Self {
        Self { dispatcher }
    }
}

#[async_trait]
impl JobMonitor for DispatchingMonitor {
    async fn on_job_created(&self, job: &JobInfo) {
        let priority = job.job_type.default_priority();
        let category = job.job_type.category();
        let tenant = job.tenant.clone();

        // Jobs registered with a future schedule (register_job_at) are
        // dispatched only once their time arrives. next_retry_at doubles
        // as the scheduled-dispatch time for new jobs.
        if let Some(scheduled_at) = job.next_retry_at {
            let now = chrono::Utc::now();
            if scheduled_at > now {
                let dispatcher = self.dispatcher.clone();
                let job_id = job.id.clone();
                let tenant = tenant.clone();
                let delay = (scheduled_at - now).to_std().unwrap_or_default();

                tracing::debug!(
                    job_id = %job.id,
                    job_type = %job.job_type,
                    delay_seconds = delay.as_secs(),
                    category = %category,
                    "Scheduled delayed dispatch for new job"
                );

                tokio::spawn(dispatch_when_due(
                    dispatcher,
                    job_id,
                    priority,
                    category,
                    tenant,
                    scheduled_at,
                ));
                return;
            }
        }

        self.dispatcher
            .dispatch_categorized(job.id.clone(), priority, category, &tenant)
            .await;

        tracing::debug!(
            job_id = %job.id,
            job_type = %job.job_type,
            priority = ?priority,
            category = %category,
            "Auto-dispatched new job to worker queue"
        );
    }

    async fn on_job_update(&self, event: JobEvent) {
        // Re-dispatch a job that has gone BACK to Scheduled from an execution
        // state — a retry, or a park.
        //
        // The test used to be `retry_count > 0`, which is the retry path's
        // signature and silently excluded the park path: `park_job`
        // deliberately does NOT increment `retry_count` ("that is the entire
        // point"), so a job parked on its FIRST attempt — the common case when
        // an upstream breaker opens — was left Scheduled with nothing to
        // re-queue it. It then sat non-terminal for the life of the process:
        // the watchdog only reaps Running/Executing, the cleanup sweeps only
        // terminal entries, and `force-fail-stuck` only targets Running. Every
        // one of those jobs held a slot against `max_active_jobs_per_tenant`
        // for ever, so a long enough outage turned into "this tenant may no
        // longer write".
        //
        // Keyed on the OLD status rather than on retry_count so both paths are
        // covered by the thing they actually share. Registration broadcasts
        // `created`, not an update, so this cannot double-dispatch a new job.
        let returned_to_queue = matches!(event.new_status, JobStatus::Scheduled)
            && matches!(
                event.old_status,
                Some(JobStatus::Running | JobStatus::Executing)
            );
        if returned_to_queue {
            let dispatcher = self.dispatcher.clone();
            let job_id = event.job_info.id.clone();
            let priority = event.job_info.job_type.default_priority();
            let category = event.job_info.job_type.category();
            let tenant = event.job_info.tenant.clone();

            // Respect backoff delay if set
            if let Some(next_retry_at) = event.job_info.next_retry_at {
                let now = chrono::Utc::now();
                if next_retry_at > now {
                    let delay = (next_retry_at - now).to_std().unwrap_or_default();
                    tokio::spawn(dispatch_when_due(
                        dispatcher,
                        job_id,
                        priority,
                        category,
                        tenant,
                        next_retry_at,
                    ));

                    tracing::debug!(
                        job_id = %event.job_info.id,
                        retry_count = event.job_info.retry_count,
                        delay_seconds = delay.as_secs(),
                        category = %category,
                        "Scheduled delayed re-dispatch for requeued job"
                    );
                    return;
                }
            }

            // No delay or already past — dispatch immediately
            self.dispatcher
                .dispatch_categorized(job_id, priority, category, &tenant)
                .await;

            tracing::debug!(
                job_id = %event.job_info.id,
                retry_count = event.job_info.retry_count,
                category = %category,
                "Re-dispatched requeued job to worker queue"
            );
        }
    }

    async fn on_job_removed(&self, _job_id: &JobId) {
        // No-op
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_storage::jobs::{JobCategory, JobId, JobType};

    fn job_info(retry_count: u32) -> JobInfo {
        JobInfo {
            id: JobId::new(),
            job_type: JobType::IntegrityScan,
            status: JobStatus::Scheduled,
            tenant: "t".to_string(),
            started_at: chrono::Utc::now(),
            completed_at: None,
            progress: None,
            error: None,
            result: None,
            retry_count,
            max_retries: 3,
            last_heartbeat: None,
            timeout_seconds: 300,
            next_retry_at: None,
            executing_since: None,
        }
    }

    /// A parked job carries `retry_count == 0` by design, so the old
    /// `retry_count > 0` gate never re-queued it and it stayed non-terminal —
    /// holding a tenant job-cap slot — until the process restarted.
    #[tokio::test]
    async fn parked_job_is_re_dispatched() {
        let (dispatcher, receivers) = JobDispatcher::new();
        let monitor = DispatchingMonitor::new(Arc::new(dispatcher));

        let info = job_info(0);
        let job_id = info.id.clone();
        monitor
            .on_job_update(JobEvent {
                job_id: job_id.clone(),
                job_info: info,
                old_status: Some(JobStatus::Executing),
                new_status: JobStatus::Scheduled,
                timestamp: chrono::Utc::now(),
            })
            .await;

        let receiver = receivers
            .get(&JobType::IntegrityScan.category())
            .expect("category receiver");
        assert_eq!(receiver.try_recv(), Some(job_id));
    }

    /// The retry path must keep working unchanged.
    #[tokio::test]
    async fn retried_job_is_re_dispatched() {
        let (dispatcher, receivers) = JobDispatcher::new();
        let monitor = DispatchingMonitor::new(Arc::new(dispatcher));

        let info = job_info(1);
        let job_id = info.id.clone();
        monitor
            .on_job_update(JobEvent {
                job_id: job_id.clone(),
                job_info: info,
                old_status: Some(JobStatus::Running),
                new_status: JobStatus::Scheduled,
                timestamp: chrono::Utc::now(),
            })
            .await;

        let receiver = receivers
            .get(&JobType::IntegrityScan.category())
            .expect("category receiver");
        assert_eq!(receiver.try_recv(), Some(job_id));
    }

    /// The wall clock can disagree with the timer that is waiting on it. A
    /// clock running 30 ms behind the one `sleep` measures must not release the
    /// job 30 ms before it is due: the claim reads the wall clock and would
    /// refuse it, dropping the job's only queue entry.
    #[tokio::test]
    async fn waiting_for_a_due_time_outlasts_a_lagging_clock() {
        let lag = chrono::Duration::milliseconds(30);
        let lagging_wall_clock = move || Utc::now() - lag;
        let due = Utc::now() + chrono::Duration::milliseconds(10);

        sleep_until_due_with(due, lagging_wall_clock).await;

        assert!(
            Utc::now() >= due + lag,
            "released at {:?}, before the lagging clock reached {:?}",
            Utc::now(),
            due
        );
    }

    /// A delayed job reaches the queue once its time arrives, not before.
    #[tokio::test]
    async fn delayed_job_is_queued_only_when_due() {
        let (dispatcher, receivers) = JobDispatcher::new();
        let monitor = DispatchingMonitor::new(Arc::new(dispatcher));

        let mut info = job_info(0);
        let job_id = info.id.clone();
        info.next_retry_at = Some(Utc::now() + chrono::Duration::milliseconds(300));
        monitor.on_job_created(&info).await;

        let receiver = receivers
            .get(&JobType::IntegrityScan.category())
            .expect("category receiver");
        assert_eq!(receiver.try_recv(), None, "queued before it was due");

        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        assert_eq!(receiver.try_recv(), Some(job_id));
    }

    /// A terminal transition must not put anything back on a queue.
    #[tokio::test]
    async fn terminal_transition_is_not_re_dispatched() {
        let (dispatcher, receivers) = JobDispatcher::new();
        let monitor = DispatchingMonitor::new(Arc::new(dispatcher));

        let info = job_info(0);
        monitor
            .on_job_update(JobEvent {
                job_id: info.id.clone(),
                job_info: info,
                old_status: Some(JobStatus::Executing),
                new_status: JobStatus::Completed,
                timestamp: chrono::Utc::now(),
            })
            .await;

        for category in [
            JobCategory::Realtime,
            JobCategory::Background,
            JobCategory::System,
        ] {
            assert_eq!(receivers.get(&category).unwrap().try_recv(), None);
        }
    }
}
