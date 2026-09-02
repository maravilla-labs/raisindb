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

//! Fair-share admission: deficit round robin over per-tenant queues.
//!
//! # The incident this exists for
//!
//! 2026-09-02: one tenant's re-processing pass filled the Background pool and
//! took a 16-core host to 96% CPU. The concurrency cap held perfectly — the
//! pool's `Semaphore(max_concurrent_handlers)` was never exceeded. The gap is
//! that a cap on HOW MANY says nothing about WHOSE: jobs arrived on one FIFO
//! channel per category, so a tenant enqueueing 3,000 jobs put all 3,000 ahead
//! of every other tenant's next job.
//!
//! This module replaces that FIFO. It decides WHOSE job takes the next handler
//! permit; it does not decide how many run. The global semaphore stays exactly
//! as it was and remains the machine-safety ceiling.
//!
//! # The two guarantees
//!
//! **Progress.** A tenant with 100,000 queued jobs cannot delay another
//! tenant's single job by more than one scheduling round. The ring is visited
//! in order and a tenant rotates out of the front as soon as its credit is
//! spent, so a newcomer waits behind at most `weight` jobs per competing
//! tenant — never behind a backlog. Under the equal weights this process ships
//! with, that is one job per competing tenant.
//!
//! **Priority without starvation.** Weight sets the credit granted per round,
//! so it is a RATIO: with weights 4 and 1, a round serves roughly four of the
//! first tenant's jobs per one of the second's, and the second advances on
//! every round. Strict priority — drain all of the higher tier before any of
//! the lower — is deliberately NOT the design: it starves the lowest tier for
//! as long as a higher one has a backlog, which that customer experiences as
//! "the product is broken".
//!
//! **An idle tenant costs nothing.** A queue that empties leaves the ring, so
//! one active tenant can use the entire global budget. A hard per-tenant
//! concurrency cap is the wrong instrument for the same reason: it leaves the
//! machine idle while work waits.
//!
//! # Locking
//!
//! One `std::sync::Mutex` over [`drr::DrrState`], held for a handful of map and
//! deque operations and NEVER across an `await` — this codebase has been burned
//! by that before, and a scheduler lock held over anything suspendable would
//! stall every enqueue and every worker in the process at once. Waiting is
//! expressed with a `Semaphore` and a `Notify` OUTSIDE the lock:
//!
//! * `items` holds exactly one permit per queued job. A worker acquires a
//!   permit and only then takes the lock, so it never waits while holding it
//!   and a successful acquire guarantees `pop` finds work. The permit is added
//!   AFTER the job is in the queue, never before, or a worker could win the
//!   race and find nothing.
//! * `space` wakes an enqueuer blocked on a full pool.
//!
//! [`raisin_storage::jobs::JobCategory`] isolation is unchanged: there is one
//! scheduler per category, exactly where there used to be one set of channels.

mod drr;
mod queue;
mod scheduler;
pub mod weight_store;
pub mod weights;

#[cfg(test)]
mod tests;

pub use drr::TenantQueueStats;
pub use scheduler::FairScheduler;
pub use weight_store::{get_weight, install_persisted_weights, set_weight};
pub use weights::{
    scheduling_weights, tenant_weight, validate_weight, EqualWeights, SharedWeights, StaticWeights,
    TenantWeights, WeightError, DEFAULT_TENANT_WEIGHT, MAX_TENANT_WEIGHT, MIN_TENANT_WEIGHT,
};

/// Bounds on how much may be queued, per tenant and in total.
///
/// The per-tenant bound is the point. A single shared bound lets one tenant
/// exhaust it, which is the same failure this module exists to fix, one level
/// down — the queue would be "bounded" and the small tenant's job would still
/// have nowhere to go.
///
/// The total bound is a memory ceiling, not a fairness device: with N tenants
/// each entitled to `per_tenant`, the worst case is N × per_tenant `JobId`s
/// resident, and N is not something this process controls.
#[derive(Debug, Clone, Copy)]
pub struct QueueLimits {
    pub per_tenant: usize,
    pub total: usize,
}

/// Jobs one tenant may have queued in one category pool.
///
/// Matched to the job registry's per-tenant valve (`max_active_jobs_per_tenant`),
/// which refuses REGISTRATION and is the primary defence; a job that is never
/// registered is never dispatched. This bound is the second line: it caps what
/// the scheduler itself can hold when that valve is disabled, which is the
/// default.
pub const DEFAULT_MAX_QUEUED_PER_TENANT: usize = 20_000;

/// Jobs one category pool may hold across all tenants.
///
/// The sum of the three priority channel capacities this replaced
/// (10k high + 50k normal + 100k low), so a single-tenant deployment sees the
/// same ceiling it saw before.
pub const DEFAULT_MAX_QUEUED_TOTAL: usize = 160_000;

impl Default for QueueLimits {
    fn default() -> Self {
        Self {
            per_tenant: DEFAULT_MAX_QUEUED_PER_TENANT,
            total: DEFAULT_MAX_QUEUED_TOTAL,
        }
    }
}

/// Queue state for one category pool, for the operator surface.
#[derive(Debug, Clone, Default)]
pub struct FairSchedulerStats {
    pub depth_high: usize,
    pub depth_normal: usize,
    pub depth_low: usize,
    pub dispatched_high: u64,
    pub dispatched_normal: u64,
    pub dispatched_low: u64,
    /// Tenants with pending work — the length of one scheduling round.
    pub active_tenants: usize,
    /// Rejections because ONE tenant filled its own queue. A non-zero value
    /// names a tenant to look at, not a machine that is too small.
    pub rejected_tenant_full: u64,
    /// Rejections because the whole pool was full.
    pub rejected_pool_full: u64,
}
