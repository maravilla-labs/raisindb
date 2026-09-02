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

//! One tenant's queue: three priority levels and a deficit counter.

use raisin_storage::jobs::{JobId, JobPriority};
use std::collections::VecDeque;

/// A single tenant's pending work within one category pool.
///
/// Priority is resolved WITHIN a tenant, not across tenants: the scheduler
/// picks whose turn it is, and this picks which of that tenant's jobs goes.
/// The alternative — priority as the outer dimension, fairness inside it —
/// would let a tenant with a backlog of High jobs delay every other tenant's
/// Normal work indefinitely, which is the starvation this whole module exists
/// to remove. Cross-category isolation (Realtime vs Background) is what keeps
/// latency-sensitive work away from bulk work, and that is unchanged.
pub(super) struct TenantQueue {
    high: VecDeque<JobId>,
    normal: VecDeque<JobId>,
    low: VecDeque<JobId>,
    /// Credit remaining in this tenant's current turn. Zero means "the next
    /// visit starts a fresh turn", which is when the quantum is re-granted.
    pub(super) deficit: u32,
    /// Credit granted at the start of each turn — the tenant's weight.
    pub(super) quantum: u32,
}

impl TenantQueue {
    pub(super) fn new(quantum: u32) -> Self {
        Self {
            high: VecDeque::new(),
            normal: VecDeque::new(),
            low: VecDeque::new(),
            deficit: 0,
            quantum,
        }
    }

    pub(super) fn push(&mut self, job_id: JobId, priority: JobPriority) {
        match priority {
            JobPriority::High => self.high.push_back(job_id),
            JobPriority::Normal => self.normal.push_back(job_id),
            JobPriority::Low => self.low.push_back(job_id),
        }
    }

    /// Highest-priority pending job for this tenant, with the level it came
    /// from — the caller needs it to keep its per-priority depth counters in
    /// step without walking every queue on every pop.
    pub(super) fn pop(&mut self) -> Option<(JobId, JobPriority)> {
        if let Some(id) = self.high.pop_front() {
            return Some((id, JobPriority::High));
        }
        if let Some(id) = self.normal.pop_front() {
            return Some((id, JobPriority::Normal));
        }
        self.low.pop_front().map(|id| (id, JobPriority::Low))
    }

    pub(super) fn len(&self) -> usize {
        self.high.len() + self.normal.len() + self.low.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.high.is_empty() && self.normal.is_empty() && self.low.is_empty()
    }

    /// Depth per priority level, in `(high, normal, low)` order.
    pub(super) fn depths(&self) -> (usize, usize, usize) {
        (self.high.len(), self.normal.len(), self.low.len())
    }
}
