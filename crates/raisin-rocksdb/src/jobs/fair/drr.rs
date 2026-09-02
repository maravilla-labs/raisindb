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

//! The deficit round-robin state machine.
//!
//! Purely synchronous and lock-free by construction: it is the thing the
//! scheduler's `Mutex` guards, so it must contain no `await` and no I/O. Every
//! method here is a handful of map and deque operations.

use super::queue::TenantQueue;
use super::weights::clamp_weight;
use raisin_storage::jobs::{JobId, JobPriority};
use std::collections::{HashMap, VecDeque};

/// What happened to an enqueue attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PushOutcome {
    Accepted,
    /// This tenant is at its own bound. Every OTHER tenant still has room —
    /// that is the whole point of bounding per tenant rather than globally.
    TenantFull,
    /// The category pool as a whole is at its bound.
    SchedulerFull,
    Closed,
}

/// Per-tenant queue depths, for the operator surface.
#[derive(Debug, Clone)]
pub struct TenantQueueStats {
    pub tenant: String,
    pub weight: u32,
    pub depth_high: usize,
    pub depth_normal: usize,
    pub depth_low: usize,
}

/// Deficit round robin over per-tenant queues.
///
/// # The invariant everything rests on
///
/// **Every tenant named in `order` has a non-empty queue.** A queue that
/// empties is removed from both `order` and `queues` in the same step that
/// emptied it, and a tenant is (re-)added to `order` exactly when its queue
/// goes from empty to non-empty. So [`DrrState::pop`] never has to skip past
/// empty queues, which is what makes it O(1) rather than O(tenants) — and with
/// 100k jobs from one tenant, an O(tenants) scan per job is a real cost.
///
/// Dropping the entry when a queue empties also drops its unspent deficit,
/// which is deliberate: carrying credit forward would let a tenant that goes
/// quiet for an hour return with an hour's worth of credit and monopolise a
/// round. In DRR terms, an idle queue's deficit resets to zero.
pub(super) struct DrrState {
    queues: HashMap<String, TenantQueue>,
    /// Ring of tenants with pending work. Front is whose turn it is.
    order: VecDeque<String>,
    /// Total queued, and the per-priority split, maintained incrementally so
    /// the stats surface never walks every tenant.
    total: usize,
    depth_high: usize,
    depth_normal: usize,
    depth_low: usize,
    closed: bool,
    /// Per-tenant bound. See [`super::QueueLimits`].
    max_per_tenant: usize,
    /// Whole-pool bound.
    max_total: usize,
}

impl DrrState {
    pub(super) fn new(max_per_tenant: usize, max_total: usize) -> Self {
        Self {
            queues: HashMap::new(),
            order: VecDeque::new(),
            total: 0,
            depth_high: 0,
            depth_normal: 0,
            depth_low: 0,
            closed: false,
            max_per_tenant,
            max_total,
        }
    }

    pub(super) fn push(
        &mut self,
        tenant: &str,
        job_id: JobId,
        priority: JobPriority,
        weight: u32,
    ) -> PushOutcome {
        if self.closed {
            return PushOutcome::Closed;
        }
        if self.total >= self.max_total {
            return PushOutcome::SchedulerFull;
        }

        // Refuse BEFORE touching the ring. Admitting the tenant and then
        // rejecting its job would leave an empty queue in `order` and break the
        // invariant that every tenant in the ring has work — `pop` would then
        // hand back `None` while other tenants still had jobs queued.
        let already_queued = self.queues.get(tenant).map(|q| q.len()).unwrap_or(0);
        if already_queued >= self.max_per_tenant {
            return PushOutcome::TenantFull;
        }

        let quantum = clamp_weight(weight);
        let queue = self.queues.entry(tenant.to_string()).or_insert_with(|| {
            // A tenant appearing for the first time in this round joins at the
            // BACK of the ring, so it cannot cut in front of a tenant whose
            // turn is already pending. Its first job still runs within one
            // round — that is the progress guarantee.
            self.order.push_back(tenant.to_string());
            TenantQueue::new(quantum)
        });

        // Re-read the weight on every push so a tier change takes effect at the
        // start of the tenant's next turn rather than at process restart.
        queue.quantum = quantum;
        queue.push(job_id, priority);

        self.total += 1;
        match priority {
            JobPriority::High => self.depth_high += 1,
            JobPriority::Normal => self.depth_normal += 1,
            JobPriority::Low => self.depth_low += 1,
        }
        PushOutcome::Accepted
    }

    /// Take the next job: the highest-priority job of whichever tenant's turn
    /// it is.
    ///
    /// Serving one job costs one credit. When a tenant's credit runs out it
    /// rotates to the back of the ring; when its queue empties it leaves the
    /// ring entirely. Either way the next `pop` belongs to a different tenant,
    /// which is what bounds another tenant's wait to one round.
    pub(super) fn pop(&mut self) -> Option<JobId> {
        let tenant = self.order.front()?.clone();
        let queue = match self.queues.get_mut(&tenant) {
            Some(q) => q,
            None => {
                // Cannot happen while the invariant holds; repair rather than
                // panic, because a panic here takes down a worker thread and
                // the pool silently loses a dispatcher.
                debug_assert!(false, "tenant in DRR ring with no queue: {tenant}");
                self.order.pop_front();
                return self.pop();
            }
        };

        if queue.deficit == 0 {
            queue.deficit = queue.quantum;
        }

        let (job_id, level) = queue.pop()?;
        queue.deficit -= 1;
        let exhausted = queue.deficit == 0;
        let emptied = queue.is_empty();

        if emptied {
            self.queues.remove(&tenant);
            self.order.pop_front();
        } else if exhausted {
            self.order.rotate_left(1);
        }

        self.total -= 1;
        match level {
            JobPriority::High => self.depth_high -= 1,
            JobPriority::Normal => self.depth_normal -= 1,
            JobPriority::Low => self.depth_low -= 1,
        }
        Some(job_id)
    }

    pub(super) fn close(&mut self) {
        self.closed = true;
    }

    pub(super) fn is_closed(&self) -> bool {
        self.closed
    }

    pub(super) fn len(&self) -> usize {
        self.total
    }

    pub(super) fn depths(&self) -> (usize, usize, usize) {
        (self.depth_high, self.depth_normal, self.depth_low)
    }

    /// Number of tenants with pending work — the number of queues a round
    /// visits, and therefore the worst-case wait (in turns) for a newcomer.
    pub(super) fn active_tenants(&self) -> usize {
        self.order.len()
    }

    pub(super) fn tenant_stats(&self) -> Vec<TenantQueueStats> {
        let mut out: Vec<TenantQueueStats> = self
            .queues
            .iter()
            .map(|(tenant, q)| {
                let (h, n, l) = q.depths();
                TenantQueueStats {
                    tenant: tenant.clone(),
                    weight: q.quantum,
                    depth_high: h,
                    depth_normal: n,
                    depth_low: l,
                }
            })
            .collect();
        // Deterministic ordering, so a console row does not jump between polls.
        out.sort_by(|a, b| a.tenant.cmp(&b.tenant));
        out
    }
}
