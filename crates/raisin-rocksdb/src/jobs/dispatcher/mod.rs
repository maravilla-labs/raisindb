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

//! Job dispatcher for fair-share work distribution
//!
//! Jobs are routed to a per-category scheduler, so realtime, background and
//! system jobs stay in separate pools. Within a category, work is admitted by
//! deficit round robin over per-tenant queues rather than a FIFO — see
//! [`crate::jobs::fair`] for what that guarantees and the incident that
//! demanded it.
//!
//! Every dispatch therefore names a TENANT. It is not optional and there is no
//! default: a job dispatched under the wrong tenant is charged to someone
//! else's share, and the fairness is only as good as the attribution. Each
//! call site has a `JobInfo`, a `JobContext` or a trigger match in hand, all of
//! which carry the tenant id.

use crate::jobs::fair::{FairScheduler, TenantQueueStats};
use raisin_storage::jobs::{JobCategory, JobId, JobPriority};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

/// Job dispatcher that routes jobs to category-specific fair schedulers
///
/// Each category (Realtime, Background, System) gets its own scheduler,
/// preventing cross-category starvation.
pub struct JobDispatcher {
    categories: HashMap<JobCategory, Arc<FairScheduler>>,
}

/// Job receiver for workers in a specific category pool
///
/// Workers use this to receive jobs. Priority ordering (High > Normal > Low)
/// is resolved WITHIN a tenant; whose turn it is comes first. See
/// the `fair` module docs for why that way round.
#[derive(Clone)]
pub struct JobReceiver {
    scheduler: Arc<FairScheduler>,
}

/// Statistics about a single category's queue state
#[derive(Debug, Clone, Default)]
pub struct CategoryQueueStats {
    pub high_queue_len: usize,
    pub normal_queue_len: usize,
    pub low_queue_len: usize,
    pub total_high_dispatched: u64,
    pub total_normal_dispatched: u64,
    pub total_low_dispatched: u64,
    /// Tenants with pending work — the length of one scheduling round, and the
    /// answer to "how many tenants are competing for this pool right now".
    pub active_tenants: usize,
}

/// Statistics about dispatcher queue state (aggregate across all categories)
#[derive(Debug, Clone)]
pub struct DispatcherStats {
    /// Number of jobs in high priority queue (aggregate)
    pub high_queue_len: usize,
    /// Number of jobs in normal priority queue (aggregate)
    pub normal_queue_len: usize,
    /// Number of jobs in low priority queue (aggregate)
    pub low_queue_len: usize,
    /// Total jobs dispatched to high priority (aggregate)
    pub total_high_dispatched: u64,
    /// Total jobs dispatched to normal priority (aggregate)
    pub total_normal_dispatched: u64,
    /// Total jobs dispatched to low priority (aggregate)
    pub total_low_dispatched: u64,
    /// Per-category stats
    pub category_stats: HashMap<JobCategory, CategoryQueueStats>,
}

impl JobDispatcher {
    /// Create a new dispatcher with per-category receivers
    ///
    /// Returns the dispatcher and a map of receivers (one per category).
    pub fn new() -> (Self, HashMap<JobCategory, JobReceiver>) {
        let mut categories = HashMap::new();
        let mut receivers = HashMap::new();

        for category in [
            JobCategory::Realtime,
            JobCategory::Background,
            JobCategory::System,
        ] {
            let scheduler = Arc::new(FairScheduler::new(category));
            receivers.insert(
                category,
                JobReceiver {
                    scheduler: scheduler.clone(),
                },
            );
            categories.insert(category, scheduler);
        }

        (Self { categories }, receivers)
    }

    fn scheduler(&self, category: JobCategory) -> &Arc<FairScheduler> {
        match self.categories.get(&category) {
            Some(s) => s,
            None => {
                warn!(
                    category = %category,
                    "No scheduler for category, falling back to Realtime"
                );
                self.categories
                    .get(&JobCategory::Realtime)
                    .expect("Realtime scheduler must exist")
            }
        }
    }

    /// Dispatch a job to the appropriate category, charged to `tenant`
    pub async fn dispatch_categorized(
        &self,
        job_id: JobId,
        priority: JobPriority,
        category: JobCategory,
        tenant: &str,
    ) {
        self.scheduler(category)
            .push(tenant, job_id, priority)
            .await;
    }

    /// Dispatch a job using default Realtime category (backward compatibility)
    pub async fn dispatch(&self, job_id: JobId, priority: JobPriority, tenant: &str) {
        self.dispatch_categorized(job_id, priority, JobCategory::Realtime, tenant)
            .await;
    }

    /// Try to dispatch a job without blocking
    pub fn try_dispatch(&self, job_id: JobId, priority: JobPriority, tenant: &str) -> bool {
        self.try_dispatch_categorized(job_id, priority, JobCategory::Realtime, tenant)
    }

    /// Try to dispatch a job to a specific category without blocking
    pub fn try_dispatch_categorized(
        &self,
        job_id: JobId,
        priority: JobPriority,
        category: JobCategory,
        tenant: &str,
    ) -> bool {
        self.scheduler(category).try_push(tenant, job_id, priority)
    }

    /// Get current queue statistics (aggregate + per-category)
    pub fn stats(&self) -> DispatcherStats {
        let mut total_high = 0usize;
        let mut total_normal = 0usize;
        let mut total_low = 0usize;
        let mut total_high_dispatched = 0u64;
        let mut total_normal_dispatched = 0u64;
        let mut total_low_dispatched = 0u64;
        let mut category_stats = HashMap::new();

        for (&cat, scheduler) in &self.categories {
            let s = scheduler.stats();
            let cat_stats = CategoryQueueStats {
                high_queue_len: s.depth_high,
                normal_queue_len: s.depth_normal,
                low_queue_len: s.depth_low,
                total_high_dispatched: s.dispatched_high,
                total_normal_dispatched: s.dispatched_normal,
                total_low_dispatched: s.dispatched_low,
                active_tenants: s.active_tenants,
            };
            total_high += cat_stats.high_queue_len;
            total_normal += cat_stats.normal_queue_len;
            total_low += cat_stats.low_queue_len;
            total_high_dispatched += cat_stats.total_high_dispatched;
            total_normal_dispatched += cat_stats.total_normal_dispatched;
            total_low_dispatched += cat_stats.total_low_dispatched;
            category_stats.insert(cat, cat_stats);
        }

        DispatcherStats {
            high_queue_len: total_high,
            normal_queue_len: total_normal,
            low_queue_len: total_low,
            total_high_dispatched,
            total_normal_dispatched,
            total_low_dispatched,
            category_stats,
        }
    }

    /// Per-tenant queue depths for one category — who is using this pool.
    pub fn tenant_stats(&self, category: JobCategory) -> Vec<TenantQueueStats> {
        self.categories
            .get(&category)
            .map(|s| s.tenant_stats())
            .unwrap_or_default()
    }

    /// Close all schedulers, signaling workers to shut down
    pub fn close(&self) {
        for scheduler in self.categories.values() {
            scheduler.close();
        }
    }
}

impl JobReceiver {
    /// Receive the next job.
    ///
    /// Which TENANT is served is decided by the fair scheduler; which of that
    /// tenant's jobs is decided by priority, High > Normal > Low.
    pub async fn recv(&self) -> Option<JobId> {
        self.scheduler.pop().await
    }

    /// Try to receive a job without blocking
    pub fn try_recv(&self) -> Option<JobId> {
        self.scheduler.try_pop()
    }

    /// Check if the scheduler has been closed
    pub fn is_closed(&self) -> bool {
        self.scheduler.is_closed()
    }

    /// Get the total number of pending jobs across all tenant queues
    pub fn pending_count(&self) -> usize {
        self.scheduler.pending_count()
    }
}

#[cfg(test)]
mod tests;
