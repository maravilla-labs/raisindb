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

//! The scheduler itself: the lock, the two wait primitives, and the counters.
//!
//! The locking contract is stated in the [`super`] module docs and is the one
//! thing to preserve when editing this file: the state `Mutex` is never held
//! across an `await`, and a job is in the queue BEFORE its permit exists.

use super::drr::{DrrState, PushOutcome};
use super::weights::{scheduling_weights, TenantWeights};
use super::{FairSchedulerStats, QueueLimits, TenantQueueStats, DEFAULT_MAX_QUEUED_PER_TENANT};
use raisin_storage::jobs::{JobCategory, JobId, JobPriority};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, Semaphore};

/// Fair-share scheduler for one job category.
pub struct FairScheduler {
    category: JobCategory,
    state: Mutex<DrrState>,
    /// One permit per queued job. See the locking note in the module docs.
    items: Semaphore,
    /// Signalled whenever a job leaves the queue, waking a blocked enqueuer.
    space: Notify,
    weights: Arc<dyn TenantWeights>,
    dispatched: [AtomicU64; 3],
    rejected_tenant_full: AtomicU64,
    rejected_pool_full: AtomicU64,
}

impl FairScheduler {
    /// A scheduler reading the process-wide weight table — what the server
    /// runs. Empty until an operator sets a weight, which is equal share for
    /// everyone; see [`super::weights`] for why the table is a global.
    pub fn new(category: JobCategory) -> Self {
        Self::with_weights(
            category,
            scheduling_weights().clone(),
            QueueLimits::default(),
        )
    }

    pub fn with_weights(
        category: JobCategory,
        weights: Arc<dyn TenantWeights>,
        limits: QueueLimits,
    ) -> Self {
        Self {
            category,
            state: Mutex::new(DrrState::new(limits.per_tenant, limits.total)),
            items: Semaphore::new(0),
            space: Notify::new(),
            weights,
            dispatched: [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)],
            rejected_tenant_full: AtomicU64::new(0),
            rejected_pool_full: AtomicU64::new(0),
        }
    }

    pub fn category(&self) -> JobCategory {
        self.category
    }

    /// Enqueue without blocking. `false` means the job was not queued.
    pub fn try_push(&self, tenant: &str, job_id: JobId, priority: JobPriority) -> bool {
        let weight = self.weights.weight_for(tenant);
        let outcome = {
            // Scope the guard: `add_permits` below must happen with the lock
            // released, and a worker woken by it takes this same lock.
            let mut state = self.state.lock().expect("DRR state lock poisoned");
            state.push(tenant, job_id.clone(), priority, weight)
        };

        match outcome {
            PushOutcome::Accepted => {
                self.items.add_permits(1);
                self.count_dispatched(priority);
                tracing::debug!(
                    job_id = %job_id,
                    tenant = %tenant,
                    priority = %priority,
                    category = %self.category,
                    "Job queued for fair-share dispatch"
                );
                true
            }
            PushOutcome::TenantFull => {
                self.rejected_tenant_full.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    job_id = %job_id,
                    tenant = %tenant,
                    category = %self.category,
                    limit = DEFAULT_MAX_QUEUED_PER_TENANT,
                    "Tenant queue full - job not dispatched. Other tenants are unaffected"
                );
                false
            }
            PushOutcome::SchedulerFull => {
                self.rejected_pool_full.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    job_id = %job_id,
                    tenant = %tenant,
                    category = %self.category,
                    "Job queue full - job not dispatched"
                );
                false
            }
            PushOutcome::Closed => {
                tracing::warn!(job_id = %job_id, "Scheduler closed - job not dispatched");
                false
            }
        }
    }

    /// Enqueue, waiting for room if the POOL is full.
    ///
    /// A tenant that fills its OWN queue is refused rather than waited on. The
    /// enqueue path is shared — one monitor task dispatches for every tenant —
    /// so blocking it on one tenant's backlog would stall every other tenant's
    /// dispatch, which inverts the fairness this module is for.
    pub async fn push(&self, tenant: &str, job_id: JobId, priority: JobPriority) -> bool {
        loop {
            // Register interest BEFORE the attempt, or a pop landing between
            // the failed push and the wait is a missed wakeup and this hangs
            // until the next unrelated pop.
            let waiting = self.space.notified();
            tokio::pin!(waiting);
            waiting.as_mut().enable();

            let weight = self.weights.weight_for(tenant);
            let outcome = {
                let mut state = self.state.lock().expect("DRR state lock poisoned");
                state.push(tenant, job_id.clone(), priority, weight)
            };

            match outcome {
                PushOutcome::Accepted => {
                    self.items.add_permits(1);
                    self.count_dispatched(priority);
                    return true;
                }
                PushOutcome::SchedulerFull => {
                    self.rejected_pool_full.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        job_id = %job_id,
                        tenant = %tenant,
                        category = %self.category,
                        "Job queue full - waiting for room"
                    );
                    waiting.await;
                }
                PushOutcome::TenantFull => {
                    self.rejected_tenant_full.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        job_id = %job_id,
                        tenant = %tenant,
                        category = %self.category,
                        limit = DEFAULT_MAX_QUEUED_PER_TENANT,
                        "Tenant queue full - job not dispatched. Other tenants are unaffected"
                    );
                    return false;
                }
                PushOutcome::Closed => return false,
            }
        }
    }

    /// Take the next job, waiting for one to arrive.
    ///
    /// Cancel-safe, which the worker loop depends on: it selects on this
    /// against a shutdown token, so the future is dropped mid-flight on every
    /// shutdown. Dropping while parked in `acquire` returns the permit to the
    /// next waiter, and once the permit is in hand there is no further await
    /// before the job is taken — so no path drops a permit while leaving its
    /// job queued, which would strand that job until an unrelated push.
    ///
    /// `None` means the scheduler is closed AND drained — a closed scheduler
    /// still hands back what it holds, so a shutdown does not silently discard
    /// queued work that a worker could still have run.
    pub async fn pop(&self) -> Option<JobId> {
        loop {
            match self.items.acquire().await {
                Ok(permit) => {
                    // The permit stands for the job about to be taken; forget
                    // it rather than let Drop return it, or the count drifts
                    // above the number of queued jobs and a later `pop` spins.
                    permit.forget();
                    if let Some(job_id) = self.take() {
                        return Some(job_id);
                    }
                    // Unreachable while permits and jobs stay in step. Loop
                    // rather than return None: returning would tell a worker
                    // the pool had shut down when it had not.
                    debug_assert!(false, "permit acquired but no job queued");
                }
                Err(_) => return self.take(),
            }
        }
    }

    /// Take the next job if one is ready.
    pub fn try_pop(&self) -> Option<JobId> {
        // Through the semaphore, not straight to the lock, so the permit count
        // stays equal to the queue depth.
        match self.items.try_acquire() {
            Ok(permit) => {
                permit.forget();
                self.take()
            }
            Err(_) => None,
        }
    }

    fn take(&self) -> Option<JobId> {
        let job_id = {
            let mut state = self.state.lock().expect("DRR state lock poisoned");
            state.pop()
        };
        if job_id.is_some() {
            // Outside the lock: a woken enqueuer takes it immediately.
            self.space.notify_waiters();
        }
        job_id
    }

    /// Stop accepting work. Queued jobs remain drainable (see [`Self::pop`]).
    pub fn close(&self) {
        {
            let mut state = self.state.lock().expect("DRR state lock poisoned");
            state.close();
        }
        // Wakes every worker parked in `pop`; each then drains what is left.
        self.items.close();
        self.space.notify_waiters();
    }

    pub fn is_closed(&self) -> bool {
        self.state
            .lock()
            .expect("DRR state lock poisoned")
            .is_closed()
    }

    pub fn pending_count(&self) -> usize {
        self.state.lock().expect("DRR state lock poisoned").len()
    }

    pub fn stats(&self) -> FairSchedulerStats {
        let (depth_high, depth_normal, depth_low, active_tenants) = {
            let state = self.state.lock().expect("DRR state lock poisoned");
            let (h, n, l) = state.depths();
            (h, n, l, state.active_tenants())
        };
        FairSchedulerStats {
            depth_high,
            depth_normal,
            depth_low,
            dispatched_high: self.dispatched[0].load(Ordering::Relaxed),
            dispatched_normal: self.dispatched[1].load(Ordering::Relaxed),
            dispatched_low: self.dispatched[2].load(Ordering::Relaxed),
            active_tenants,
            rejected_tenant_full: self.rejected_tenant_full.load(Ordering::Relaxed),
            rejected_pool_full: self.rejected_pool_full.load(Ordering::Relaxed),
        }
    }

    /// Per-tenant depths — "who is using the machine right now", for the
    /// operator surface.
    pub fn tenant_stats(&self) -> Vec<TenantQueueStats> {
        self.state
            .lock()
            .expect("DRR state lock poisoned")
            .tenant_stats()
    }

    fn count_dispatched(&self, priority: JobPriority) {
        let idx = match priority {
            JobPriority::High => 0,
            JobPriority::Normal => 1,
            JobPriority::Low => 2,
        };
        self.dispatched[idx].fetch_add(1, Ordering::Relaxed);
    }
}
