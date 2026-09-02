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

//! Bounds, shutdown and concurrency of the scheduler itself.

use super::{count, Harness};
use crate::jobs::fair::*;
use raisin_storage::jobs::{JobCategory, JobId, JobPriority};
use std::collections::HashMap;
use std::sync::Arc;

/// The bound is PER TENANT. A single shared bound would let one tenant exhaust
/// it, which is the same failure one level down: the queue is "bounded" and the
/// small tenant's job still has nowhere to go.
#[tokio::test]
async fn a_tenant_at_its_bound_is_refused_and_the_others_are_untouched() {
    let h = Harness::with_limits(
        HashMap::new(),
        QueueLimits {
            per_tenant: 4,
            total: 1_000,
        },
    );
    h.push_n("hog", 4, JobPriority::Normal);

    assert!(
        !h.sched.try_push("hog", JobId::new(), JobPriority::Normal),
        "the 5th job of a 4-job bound must be refused"
    );
    assert!(
        h.sched.try_push("other", JobId::new(), JobPriority::Normal),
        "another tenant must still be admitted"
    );

    let stats = h.sched.stats();
    assert_eq!(stats.rejected_tenant_full, 1);
    assert_eq!(stats.rejected_pool_full, 0);

    // A refusal must not have left an empty queue in the ring: the ring holds
    // hog and other, and every pop returns a job.
    assert_eq!(stats.active_tenants, 2);
    for _ in 0..5 {
        assert!(h.sched.try_pop().is_some());
    }
    assert!(h.sched.try_pop().is_none());
}

/// The whole-pool bound is a memory ceiling, and reaching it is reported
/// separately — "one tenant is misbehaving" and "the machine is full" are
/// different operator actions.
#[tokio::test]
async fn the_pool_bound_is_reported_separately_from_the_tenant_bound() {
    let h = Harness::with_limits(
        HashMap::new(),
        QueueLimits {
            per_tenant: 100,
            total: 3,
        },
    );
    h.push_one("a", JobPriority::Normal);
    h.push_one("b", JobPriority::Normal);
    h.push_one("c", JobPriority::Normal);

    assert!(!h.sched.try_push("d", JobId::new(), JobPriority::Normal));
    let stats = h.sched.stats();
    assert_eq!(stats.rejected_pool_full, 1);
    assert_eq!(stats.rejected_tenant_full, 0);
}

/// A weight of zero would mean a queue that is accepted and never served — a
/// hang, reported nowhere. It is clamped up to one, so the floor of the ratio
/// is always "still makes progress".
#[tokio::test]
async fn a_zero_weight_is_clamped_up_rather_than_starving_a_tenant() {
    let h = Harness::new(HashMap::from([
        ("zero".to_string(), 0),
        ("huge".to_string(), u32::MAX),
    ]));
    h.push_n("zero", 5, JobPriority::Normal);
    h.push_n("huge", 5, JobPriority::Normal);

    // huge is clamped to MAX_TENANT_WEIGHT, so it takes its whole backlog in
    // one turn; zero is clamped to 1 and is served in the same round.
    let owners = h.pop_owners(6).await;
    assert_eq!(count(&owners, "zero"), 1, "got {owners:?}");
    assert!(weights::MAX_TENANT_WEIGHT >= 5);
}

/// A closed scheduler still hands back what it holds. A shutdown that dropped
/// queued jobs on the floor would lose work the registry still believes is
/// Scheduled.
#[tokio::test]
async fn close_drains_what_is_queued_then_reports_end_of_stream() {
    let h = Harness::equal();
    h.push_n("a", 2, JobPriority::Normal);
    h.sched.close();

    assert!(h.sched.pop().await.is_some());
    assert!(h.sched.pop().await.is_some());
    assert!(h.sched.pop().await.is_none());
    assert!(h.sched.is_closed());
    assert!(!h.sched.try_push("a", JobId::new(), JobPriority::Normal));
}

/// Every queued job is served exactly once across concurrent workers — the
/// permit count and the queue depth must stay in step, or a worker either
/// spins on an empty queue or a job is handed out twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_workers_serve_every_job_exactly_once() {
    let sched = Arc::new(FairScheduler::new(JobCategory::Background));
    let total = 400;
    for i in 0..total {
        let tenant = format!("t{}", i % 8);
        assert!(sched.try_push(&tenant, JobId::new(), JobPriority::Normal));
    }
    sched.close(); // so workers stop once drained

    let mut workers = Vec::new();
    for _ in 0..4 {
        let sched = sched.clone();
        workers.push(tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(id) = sched.pop().await {
                seen.push(id);
            }
            seen
        }));
    }

    let mut all = Vec::new();
    for w in workers {
        all.extend(w.await.unwrap());
    }
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(all.len(), total, "every job served");
    assert_eq!(unique.len(), total, "and none of them twice");
    assert_eq!(sched.pending_count(), 0);
}

/// The default the server ships with is equal weight for everyone, because
/// there is no tier model to read. If a tier source is ever wired in, this test
/// is the one that should change deliberately.
#[test]
fn the_shipped_weight_is_equal_for_every_tenant() {
    assert_eq!(tenant_weight("acme"), DEFAULT_TENANT_WEIGHT);
    assert_eq!(tenant_weight("other"), DEFAULT_TENANT_WEIGHT);
    assert_eq!(EqualWeights.weight_for("acme"), DEFAULT_TENANT_WEIGHT);
}
