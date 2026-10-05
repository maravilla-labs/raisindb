//! The translation history floor is per BRANCH, capped at that branch's HEAD,
//! and never bounces back to the node whose history GC produced it.
//!
//! It used to be one value per repository — the max of every branch's GC
//! cutoff, raised for every planned branch whenever any translation version
//! was deleted anywhere. A retention cutoff (`now - keep_days`) sits above the
//! HEAD of a branch that has been quiet longer than that, so after a resync
//! every replica refused every locale-scoped read of that branch AT ITS HEAD,
//! translated node or not; and since a replica folded the floor it received
//! into the floor it sent, the resync fan-out carried it back to the origin,
//! which then refused its own reads too.

use crate::translation_replication_test::{highest_seq, node, receive, translation_ops, Node};
use crate::translation_substrate_test::{get, rev, store, title, B, R, T};
use raisin_replication::OpType;
use raisin_rocksdb::management::async_indexing::repair::{run_repair, RepairKind, RepairOptions};
use raisin_rocksdb::management::history_gc::{run_history_gc, GcOptions, HistoryRetention};
use raisin_rocksdb::translation_history;
use raisin_storage::{BranchRepository, Storage};
use std::time::Duration;

const NODE: &str = "page-idle";

async fn resync(from: &Node) {
    run_repair(
        &from.storage,
        T,
        R,
        Some(B),
        RepairKind::ResyncTranslations,
        RepairOptions {
            check_headroom: false,
            max_bytes_per_sec: 0,
            ..RepairOptions::default()
        },
    )
    .await
    .unwrap();
}

fn floor(node: &Node, branch: &str) -> Option<raisin_hlc::HLC> {
    translation_history::complete_from(node.storage.db(), T, R, branch).unwrap()
}

#[tokio::test]
async fn idle_branch_head_reads_on_a_replica_after_gc_and_resync() {
    let origin = node("origin").await;
    // Two versions, then the branch goes quiet: its HEAD (r3, in 2024) is far
    // below a 30-day retention cutoff.
    store(&origin.storage, NODE, "fr", title("one"), rev(1)).await;
    store(&origin.storage, NODE, "fr", title("two"), rev(2)).await;
    let head = rev(3);
    origin
        .storage
        .branches()
        .update_head(T, R, B, head)
        .await
        .unwrap();
    let after = highest_seq(&origin);

    let report = run_history_gc(
        &origin.storage,
        &GcOptions {
            retention_override: Some(HistoryRetention {
                keep_days: Some(30),
                keep_revisions: None,
            }),
            min_age: Duration::ZERO,
            tenant: Some(T.to_string()),
            ..GcOptions::default()
        },
    )
    .unwrap();
    assert!(report.versions_deleted > 0, "fr@r1 is pruned");
    // Capped at the HEAD the branch had when GC ran, not the cutoff.
    assert_eq!(
        translation_history::gc_cutoff(origin.storage.db(), T, R, B).unwrap(),
        Some(head)
    );
    // A branch GC deleted no translation version of holds full history.
    assert_eq!(
        translation_history::gc_cutoff(origin.storage.db(), T, R, "feature").unwrap(),
        None
    );

    resync(&origin).await;
    let replica = node("replica").await;
    receive(&replica, &translation_ops(&origin, after)).await;
    assert_eq!(floor(&replica, B), Some(head));
    assert_eq!(floor(&replica, "feature"), None);

    // The replica answers HEAD reads of the idle branch — translated or not —
    // exactly as the origin does, and refuses only what it cannot vouch for.
    assert_eq!(
        get(&replica.storage, NODE, "fr", head).await.unwrap(),
        Some(title("two"))
    );
    assert_eq!(
        get(&replica.storage, "never-translated", "fr", head)
            .await
            .unwrap(),
        None
    );
    assert!(get(&replica.storage, NODE, "fr", rev(1)).await.is_err());

    // The replica's own resync fans back out to the origin. What it SENDS is
    // its own GC floor (none), so nothing comes back to raise the origin's.
    let replica_after = highest_seq(&replica);
    resync(&replica).await;
    let back = translation_ops(&replica, replica_after);
    assert!(!back.is_empty());
    assert!(back.iter().all(|op| matches!(
        op.op_type,
        OpType::UpsertTranslationOverlay {
            history_complete_from: None,
            ..
        }
    )));
    receive(&origin, &back).await;
    assert_eq!(floor(&origin, B), None);
    assert_eq!(
        get(&origin.storage, NODE, "fr", head).await.unwrap(),
        Some(title("two"))
    );
}

/// Even a floor that does arrive back at the node that produced it (an
/// older sender, or a chain of resyncs) is ignored there: at or below this
/// node's own GC floor it says nothing new.
#[tokio::test]
async fn a_floor_returning_to_its_origin_is_ignored() {
    let origin = node("origin").await;
    let db = origin.storage.db();
    translation_history::raise_gc_cutoff(db, T, R, B, rev(5)).unwrap();
    assert!(!translation_history::accept_received_floor(db, T, R, B, rev(5)).unwrap());
    assert!(!translation_history::accept_received_floor(db, T, R, B, rev(4)).unwrap());
    assert_eq!(floor(&origin, B), None);
    // A higher one is news: another node lost history this one never had.
    assert!(translation_history::accept_received_floor(db, T, R, B, rev(6)).unwrap());
    assert_eq!(floor(&origin, B), Some(rev(6)));
}
