// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Store conformance for child runs: lineage invariants on create, the
//! hand-back-owed worklist, and keyed checkpoint reads.

use serde_json::json;

use crate::child::{ChildObjective, Delegation};
use crate::child_apply::apply_handback_delivered;
use crate::conformance::{commit, create_request, first_event, scope};
use crate::events::CheckpointReason;
use crate::ids::RunId;
use crate::lifecycle::{self, Fence};
use crate::service::new_record;
use crate::state::{RunOutcome, TerminalStatus};
use crate::store::AgentRunStore;

const T0: u64 = 1_000_000;

pub(crate) async fn lineage_tables(store: &dyn AgentRunStore) {
    let s = scope("t1");
    let req = create_request(&s, "/child");
    let mut rec = new_record(&req, RunId::new_v4(), T0);
    rec.depth = 1;
    assert!(
        store
            .create(rec.clone(), first_event(&req), None)
            .await
            .is_err(),
        "lineage: depth without a parent is refused"
    );
    let parent = RunId::new_v4();
    rec.parent_run_id = Some(parent.clone());
    rec.root_run_id = Some(parent);
    let objective: ChildObjective = serde_json::from_value(json!({ "title": "t" })).unwrap();
    rec.delegation = Some(Delegation {
        objective,
        child_no: 1,
        spawn_key: None,
        handback_delivered: false,
    });
    let run = rec.run_id.clone();
    store
        .create(rec, first_event(&req), None)
        .await
        .expect("child create");
    assert!(
        store.scan_handback_owed(&s, 10).await.unwrap().is_empty(),
        "live child owes nothing"
    );

    let rec = store.load(&s, &run).await.unwrap().unwrap();
    let t = lifecycle::apply_acquire(&rec, "w1", T0, 90_000).unwrap();
    commit(store, &t, Fence::None).await.unwrap();
    let c1 = lifecycle::apply_checkpoint(&t.record, CheckpointReason::Compaction, None, T0);
    commit(store, &c1, Fence::None).await.unwrap();
    let c2 = lifecycle::apply_checkpoint(&c1.record, CheckpointReason::Periodic, None, T0);
    commit(store, &c2, Fence::None).await.unwrap();
    let first = store
        .read_checkpoint(&s, &run, 1)
        .await
        .unwrap()
        .expect("checkpoint 1");
    assert_eq!(
        (first.checkpoint_no, first.reason),
        (1, CheckpointReason::Compaction)
    );
    assert_eq!(
        store
            .read_checkpoint(&s, &run, 2)
            .await
            .unwrap()
            .unwrap()
            .reason,
        CheckpointReason::Periodic
    );

    let end = lifecycle::apply_terminal(
        &c2.record,
        TerminalStatus::Completed,
        RunOutcome::new("succeeded", None),
        None,
        T0,
    )
    .unwrap();
    commit(store, &end, Fence::None).await.unwrap();
    assert_eq!(
        store.scan_handback_owed(&s, 10).await.unwrap(),
        vec![run.clone()],
        "terminal child owes its hand-back"
    );
    let delivered = apply_handback_delivered(&end.record, T0).expect("owed");
    commit(store, &delivered, Fence::None).await.unwrap();
    assert!(
        store.scan_handback_owed(&s, 10).await.unwrap().is_empty(),
        "delivered leaves the worklist"
    );
}
