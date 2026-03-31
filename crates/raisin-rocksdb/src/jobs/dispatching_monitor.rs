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
use raisin_storage::jobs::{JobEvent, JobId, JobInfo, JobMonitor, JobStatus};
use std::sync::Arc;

use super::dispatcher::JobDispatcher;

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
        self.dispatcher
            .dispatch_categorized(job.id.clone(), priority, category)
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
        // Re-dispatch jobs that are scheduled for retry
        if matches!(event.new_status, JobStatus::Scheduled) && event.job_info.retry_count > 0 {
            let dispatcher = self.dispatcher.clone();
            let job_id = event.job_info.id.clone();
            let priority = event.job_info.job_type.default_priority();
            let category = event.job_info.job_type.category();

            // Respect backoff delay if set
            if let Some(next_retry_at) = event.job_info.next_retry_at {
                let now = chrono::Utc::now();
                if next_retry_at > now {
                    let delay = (next_retry_at - now).to_std().unwrap_or_default();
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        dispatcher
                            .dispatch_categorized(job_id, priority, category)
                            .await;
                    });

                    tracing::debug!(
                        job_id = %event.job_info.id,
                        retry_count = event.job_info.retry_count,
                        delay_seconds = delay.as_secs(),
                        category = %category,
                        "Scheduled delayed re-dispatch for retried job"
                    );
                    return;
                }
            }

            // No delay or already past — dispatch immediately
            self.dispatcher
                .dispatch_categorized(job_id, priority, category)
                .await;

            tracing::debug!(
                job_id = %event.job_info.id,
                retry_count = event.job_info.retry_count,
                category = %category,
                "Re-dispatched retried job to worker queue"
            );
        }
    }

    async fn on_job_removed(&self, _job_id: &JobId) {
        // No-op
    }
}
