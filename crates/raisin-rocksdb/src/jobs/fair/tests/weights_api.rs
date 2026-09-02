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

//! The operator-set weight: validated at the edge, durable across a restart,
//! and actually honoured by the scheduler.
//!
//! These tests touch the PROCESS-WIDE table, so they take `GLOBAL` and use
//! tenant ids of their own. Two tests replacing that table concurrently would
//! delete each other's entries and fail for a reason that has nothing to do
//! with what they assert.

use super::*;
use crate::jobs::fair::weight_store;
use crate::jobs::fair::weights::{
    scheduling_weights, validate_weight, WeightError, MAX_TENANT_WEIGHT,
};
use raisin_storage::jobs::JobPriority;
use rocksdb::{Options, DB};
use std::sync::Mutex;
use tempfile::TempDir;

static GLOBAL: Mutex<()> = Mutex::new(());

fn test_db() -> (TempDir, DB) {
    let dir = TempDir::new().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    let db = DB::open_cf(&opts, dir.path(), vec![crate::cf::REGISTRY]).unwrap();
    (dir, db)
}

/// A weight an operator set must still be in effect after a restart.
///
/// The failure this prevents is silent: with the weight held only in memory,
/// every restart flattens the whole deployment back to equal share. Nothing
/// errors, nothing logs, and the operator's configuration is simply gone.
#[test]
fn weight_survives_a_restart() {
    let _guard = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, db) = test_db();

    weight_store::set_weight(&db, "persist-me", 8).unwrap();
    assert_eq!(scheduling_weights().get("persist-me"), Some(8));

    // A restart: the process table is gone, storage is not.
    scheduling_weights().replace_all(HashMap::new());
    assert_eq!(scheduling_weights().get("persist-me"), None);

    weight_store::install_persisted_weights(&db).unwrap();
    assert_eq!(scheduling_weights().get("persist-me"), Some(8));
    assert_eq!(scheduling_weights().weight_for("persist-me"), 8);
}

/// Zero is refused at the edge, not corrected.
///
/// A zero-weight queue is granted no credit per round, so it is served never:
/// its jobs are accepted and then sit there, which is a hang reported nowhere.
/// The scheduler clamps defensively too — asserted here — but a caller that
/// asked for zero made a mistake and must be told.
#[test]
fn zero_weight_is_refused_and_can_never_take_effect() {
    assert_eq!(validate_weight(0), Err(WeightError::TooLow));
    assert_eq!(
        validate_weight(MAX_TENANT_WEIGHT + 1),
        Err(WeightError::TooHigh)
    );
    assert_eq!(validate_weight(1), Ok(1));
    assert_eq!(validate_weight(MAX_TENANT_WEIGHT), Ok(MAX_TENANT_WEIGHT));

    // Belt and braces: even a caller that skipped validation cannot install a
    // zero into the live table.
    let table = SharedWeights::new();
    table.set("sneaky", 0);
    assert_eq!(table.weight_for("sneaky"), 1);
}

/// An unconfigured tenant is weight 1 — absent and "set to 1" are the same
/// thing, so a server nobody has configured is pure round robin.
#[test]
fn absent_weight_reads_as_one() {
    let _guard = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, db) = test_db();

    assert_eq!(weight_store::get_weight(&db, "never-set").unwrap(), None);
    assert_eq!(
        scheduling_weights().weight_for("never-set"),
        DEFAULT_TENANT_WEIGHT
    );
}

/// The number an operator sets is the ratio the scheduler serves.
///
/// Weight 4 against weight 1 means roughly four jobs to one over a round —
/// and, just as importantly, the weight-1 tenant still advances. A weight
/// that persisted but never reached the scheduler is the whole feature
/// missing with a green API response.
#[tokio::test]
async fn scheduler_honours_a_set_weight() {
    let _guard = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, db) = test_db();

    weight_store::set_weight(&db, "heavy", 4).unwrap();
    weight_store::set_weight(&db, "light", 1).unwrap();

    // `FairScheduler::new` reads the process-wide table — the same path the
    // server takes, not a hand-built weights provider.
    let sched = FairScheduler::new(JobCategory::Background);
    let mut owner = HashMap::new();
    for tenant in ["heavy", "light"] {
        for _ in 0..20 {
            let id = JobId::new();
            assert!(sched.try_push(tenant, id.clone(), JobPriority::Normal));
            owner.insert(id, tenant.to_string());
        }
    }

    let mut served = Vec::new();
    for _ in 0..10 {
        let id = sched.pop().await.expect("scheduler ran dry");
        served.push(owner.get(&id).cloned().unwrap());
    }

    let heavy = served.iter().filter(|t| t.as_str() == "heavy").count();
    let light = served.len() - heavy;
    assert!(
        heavy > light,
        "weight 4 must outpace weight 1, got heavy={heavy} light={light}"
    );
    assert!(
        light > 0,
        "the weight-1 tenant must still advance — a ratio, never a precedence"
    );
}
