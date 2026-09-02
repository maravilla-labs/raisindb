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

//! The guarantees, asserted. Each test names the failure it prevents.
//!
//! Fairness and ordering live here; the bounds, shutdown and concurrency of
//! the scheduler itself are in [`bounds`].

mod bounds;
mod weights_api;

use super::*;
use raisin_storage::jobs::{JobCategory, JobId, JobPriority};
use std::collections::HashMap;
use std::sync::Arc;

/// A scheduler plus a job-id → tenant map, because the guarantees are all
/// statements about WHOSE job came out and a `JobId` carries no owner.
pub(super) struct Harness {
    pub(super) sched: FairScheduler,
    owner: std::sync::Mutex<HashMap<JobId, String>>,
}

impl Harness {
    pub(super) fn new(weights: HashMap<String, u32>) -> Self {
        Self::with_limits(weights, QueueLimits::default())
    }

    pub(super) fn with_limits(weights: HashMap<String, u32>, limits: QueueLimits) -> Self {
        Self {
            sched: FairScheduler::with_weights(
                JobCategory::Background,
                Arc::new(StaticWeights::new(weights)),
                limits,
            ),
            owner: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Equal weights for everyone — what the server actually runs today.
    pub(super) fn equal() -> Self {
        Self::new(HashMap::new())
    }

    pub(super) fn push_n(&self, tenant: &str, n: usize, priority: JobPriority) -> Vec<JobId> {
        (0..n)
            .map(|_| self.push_one(tenant, priority))
            .collect::<Vec<_>>()
    }

    pub(super) fn push_one(&self, tenant: &str, priority: JobPriority) -> JobId {
        let id = JobId::new();
        assert!(
            self.sched.try_push(tenant, id.clone(), priority),
            "push for {tenant} was refused"
        );
        self.owner
            .lock()
            .unwrap()
            .insert(id.clone(), tenant.to_string());
        id
    }

    /// Pop `n` jobs and return the tenant each belonged to, in order.
    pub(super) async fn pop_owners(&self, n: usize) -> Vec<String> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let id = self.sched.pop().await.expect("scheduler ran dry");
            out.push(self.owner.lock().unwrap().get(&id).cloned().unwrap());
        }
        out
    }
}

pub(super) fn count(owners: &[String], tenant: &str) -> usize {
    owners.iter().filter(|t| t.as_str() == tenant).count()
}

/// THE headline guarantee: a backlog must not delay a newcomer.
///
/// Before this module the category queue was one FIFO, so the small tenant's
/// job sat behind all 10,000 of the big tenant's — that is the 2026-09-02
/// incident in miniature. Here it must come out within one round, i.e. after
/// at most one job per competing tenant.
#[tokio::test]
async fn a_10k_backlog_does_not_delay_another_tenant_beyond_one_round() {
    let h = Harness::equal();
    h.push_n("noisy", 10_000, JobPriority::Normal);

    // The quiet tenant arrives with a single job while the backlog is queued.
    h.push_one("quiet", JobPriority::Normal);

    // One round over two tenants at weight 1 is two jobs.
    let owners = h.pop_owners(2).await;
    assert_eq!(
        owners,
        vec!["noisy".to_string(), "quiet".to_string()],
        "quiet must be served after ONE noisy job, not 10,000"
    );

    // Fairness is not a stall: the backlog keeps draining afterwards, and with
    // quiet now empty it gets the machine to itself.
    assert_eq!(h.sched.pending_count(), 9_999);
    assert_eq!(h.pop_owners(50).await, vec!["noisy".to_string(); 50]);
}

/// Weight is a RATIO, not a precedence. Four paid jobs per free job — and the
/// free tenant advances on EVERY round, never zero, which is exactly the
/// starvation that strict priority would cause.
#[tokio::test]
async fn weights_set_the_ratio_and_the_lower_weight_never_stalls() {
    let h = Harness::new(HashMap::from([
        ("paid".to_string(), 4),
        ("free".to_string(), 1),
    ]));
    h.push_n("paid", 500, JobPriority::Normal);
    h.push_n("free", 500, JobPriority::Normal);

    // A round is 4 paid + 1 free = 5 jobs. Check every round individually:
    // an aggregate 4:1 could hide 20 paid followed by 5 free, which is the
    // starvation shape.
    for round in 0..20 {
        let owners = h.pop_owners(5).await;
        assert_eq!(
            count(&owners, "paid"),
            4,
            "round {round} served {owners:?}, expected 4 paid"
        );
        assert_eq!(
            count(&owners, "free"),
            1,
            "round {round} served {owners:?}, expected 1 free — the lower \
             weight must advance every round"
        );
    }
}

/// An idle tenant costs nothing: its empty queue leaves the ring, so a single
/// active tenant reaches the full global budget rather than being throttled to
/// its "share" while the machine sits idle. That is why a hard per-tenant
/// concurrency cap is the wrong instrument.
#[tokio::test]
async fn an_idle_tenants_queue_is_skipped_so_one_tenant_gets_the_whole_budget() {
    let h = Harness::equal();

    // Three tenants, two of which go quiet after one job each.
    h.push_one("quiet_a", JobPriority::Normal);
    h.push_one("quiet_b", JobPriority::Normal);
    h.push_n("busy", 200, JobPriority::Normal);

    let first_round = h.pop_owners(3).await;
    assert_eq!(count(&first_round, "busy"), 1);
    assert_eq!(h.sched.stats().active_tenants, 1, "the quiet two are gone");

    // From here every pop is the busy tenant's: no empty queue is visited, so
    // 100 consecutive handler permits all go to work that exists.
    let rest = h.pop_owners(100).await;
    assert_eq!(count(&rest, "busy"), 100);
}

/// A tenant that goes quiet must not bank credit while it is away and spend it
/// in one burst on return — that would be a backlog jumping the queue by having
/// been idle, which defeats the round bound.
#[tokio::test]
async fn an_idle_tenant_does_not_hoard_unspent_credit() {
    let h = Harness::new(HashMap::from([("paid".to_string(), 4)]));

    // paid has weight 4 but only one job: it spends 1 credit and leaves.
    h.push_one("paid", JobPriority::Normal);
    assert_eq!(h.pop_owners(1).await, vec!["paid".to_string()]);

    // It returns alongside a competitor. If the 3 unspent credits had been
    // banked it would serve 7 in a row here.
    h.push_n("paid", 10, JobPriority::Normal);
    h.push_n("other", 10, JobPriority::Normal);
    let owners = h.pop_owners(5).await;
    assert_eq!(count(&owners, "paid"), 4, "got {owners:?}");
    assert_eq!(count(&owners, "other"), 1, "got {owners:?}");
}

/// Priority is resolved WITHIN a tenant. Across tenants the round robin wins —
/// otherwise a backlog of High jobs from one tenant would delay every other
/// tenant's Normal work indefinitely, which is the starvation this module
/// exists to remove.
#[tokio::test]
async fn priority_orders_a_tenants_own_jobs_but_does_not_cross_tenants() {
    let h = Harness::equal();
    let low = h.push_one("a", JobPriority::Low);
    let high = h.push_one("a", JobPriority::High);
    let normal = h.push_one("a", JobPriority::Normal);
    h.push_one("b", JobPriority::Low);

    // b's Low job is served in the same round as a's High one...
    assert_eq!(
        h.pop_owners(2).await,
        vec!["a".to_string(), "b".to_string()]
    );
    // ...and a's own three came out High, Normal, Low.
    assert_eq!(h.sched.pop().await.unwrap(), normal);
    assert_eq!(h.sched.pop().await.unwrap(), low);
    let _ = high;
}
