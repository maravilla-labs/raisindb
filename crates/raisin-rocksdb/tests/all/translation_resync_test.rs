//! Plan Phase 11 item 5 and the mixed-version gate: the `resync_translations`
//! job re-emits overlay HISTORY, a replica honours the sender's history floor,
//! and an op type this binary does not know is skipped, never a stall.

use crate::translation_replication_test::{highest_seq, node, receive, translation_ops, Node};
use crate::translation_substrate_test::{code, get, rev, store, title, tombstone, B, R, T, WS};
use raisin_hlc::HLC;
use raisin_models::translations::LocaleOverlay;
use raisin_replication::{OpType, Operation, ReplicatedOverlay, ReplicationMessage, VectorClock};
use raisin_rocksdb::management::async_indexing::repair::{run_repair, RepairKind, RepairOptions};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::{translation_history, OpLogRepository};
use raisin_storage::{Storage, TranslationRepository};
use std::sync::Arc;

const NODE: &str = "page-h";

/// Origin history: fr r1 → r2 → Hidden r3 → deleted r4; de r2; a block r1.
/// Written, then the ops it captured are dropped: what a pre-Phase-11 origin
/// holds, never replicated. Returns the seq to resync after.
async fn history(origin: &Node) -> u64 {
    store(&origin.storage, NODE, "fr", title("one"), rev(1)).await;
    store(&origin.storage, NODE, "fr", title("two"), rev(2)).await;
    store(&origin.storage, NODE, "fr", LocaleOverlay::Hidden, rev(3)).await;
    tombstone(&origin.storage, NODE, "fr", &rev(4));
    store(&origin.storage, NODE, "de", title("eins"), rev(2)).await;
    origin
        .storage
        .translations()
        .store_block_translation(
            T,
            R,
            B,
            WS,
            NODE,
            "block-1",
            &code("fr"),
            &title("bloc"),
            &crate::translation_substrate_test::meta("fr", rev(1)),
        )
        .await
        .unwrap();
    highest_seq(origin)
}

fn options() -> RepairOptions {
    RepairOptions {
        // The test volume's free space says nothing about the feature.
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    }
}

async fn resync(
    origin: &Node,
    options: RepairOptions,
) -> raisin_rocksdb::management::async_indexing::repair::RepairReport {
    let mut reports = run_repair(
        &origin.storage,
        T,
        R,
        Some(B),
        RepairKind::ResyncTranslations,
        options,
    )
    .await
    .unwrap();
    reports.remove(0)
}

/// Every read the origin answers at r0..r5, the replica answers the same.
async fn assert_same_history(origin: &Node, replica: &Node) {
    for n in 0..=5 {
        for locale in ["fr", "de"] {
            assert_eq!(
                get(&replica.storage, NODE, locale, rev(n)).await.unwrap(),
                get(&origin.storage, NODE, locale, rev(n)).await.unwrap(),
                "{locale} at r{n}"
            );
        }
        let block = |s: Arc<raisin_rocksdb::RocksDBStorage>| async move {
            s.translations()
                .get_block_translation(T, R, B, WS, NODE, "block-1", &code("fr"), &rev(n))
                .await
                .unwrap()
        };
        assert_eq!(
            block(replica.storage.clone()).await,
            block(origin.storage.clone()).await
        );
    }
}

#[tokio::test]
async fn resync_translations_preserves_overlay_history() {
    let origin = node("origin").await;
    let after = history(&origin).await;

    // Crash after the first chunk (one version per chunk), then resume.
    let first = resync(
        &origin,
        RepairOptions {
            batch_bytes: 1,
            stop_after_batches: Some(2),
            ..options()
        },
    )
    .await;
    assert!(!first.completed);
    let second = resync(&origin, options()).await;
    assert!(second.resumed && second.completed, "{second:?}");
    assert_eq!(
        first.translations.versions + second.translations.versions,
        6
    );

    let ops = translation_ops(&origin, after);
    assert_eq!(ops.len(), 6, "every version once: {ops:#?}");
    let mut revisions: Vec<HLC> = ops.iter().map(|op| op.revision.unwrap()).collect();
    revisions.sort();
    assert_eq!(
        revisions,
        vec![rev(1), rev(1), rev(2), rev(2), rev(3), rev(4)]
    );
    // T as a deletion, Hidden as Hidden.
    assert!(ops.iter().any(|op| matches!(
        op.op_type,
        OpType::UpsertTranslationOverlay {
            overlay: ReplicatedOverlay::Deleted,
            ..
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op.op_type,
        OpType::UpsertTranslationOverlay {
            overlay: ReplicatedOverlay::Hidden,
            ..
        }
    )));

    let replica = node("replica").await;
    receive(&replica, &ops).await;
    assert_same_history(&origin, &replica).await;
    // Full history: no floor recorded.
    assert_eq!(
        translation_history::complete_from(replica.storage.db(), T, R, B).unwrap(),
        None
    );

    // A run over a done state starts over and is idempotent on the replica.
    let again = resync(&origin, options()).await;
    assert!(!again.resumed && again.completed);
    receive(
        &replica,
        &translation_ops(&origin, highest_seq(&origin) - 6),
    )
    .await;
    assert_same_history(&origin, &replica).await;
}

/// History GC removed translation versions below r2 on the origin: its resync
/// says so, and the replica refuses locale-scoped reads below r2 rather than
/// serve versions it cannot vouch for.
#[tokio::test]
async fn resync_carries_the_history_floor() {
    let origin = node("origin").await;
    let after = history(&origin).await;
    translation_history::raise_gc_cutoff(origin.storage.db(), T, R, B, rev(2)).unwrap();

    let report = resync(&origin, options()).await;
    assert_eq!(
        report.translations.history_complete_from,
        Some(rev(2).to_string())
    );
    let replica = node("replica").await;
    receive(&replica, &translation_ops(&origin, after)).await;

    assert_eq!(
        translation_history::complete_from(replica.storage.db(), T, R, B).unwrap(),
        Some(rev(2))
    );
    assert!(get(&replica.storage, NODE, "fr", rev(1)).await.is_err());
    assert_eq!(
        get(&replica.storage, NODE, "fr", rev(2)).await.unwrap(),
        Some(title("two"))
    );
}

/// The newer peer's batch carries an op this binary cannot decode. It must
/// decode, persist, be skipped, and let the ops after it apply — and the
/// vector clock must move past it, or the next sync asks for it forever.
#[tokio::test]
async fn unknown_optype_is_skipped_not_stalled() {
    let replica = node("replica").await;
    let op = |seq: u64, op_type: OpType| {
        let mut op = Operation::new(
            seq,
            "newer-peer".to_string(),
            VectorClock::new(),
            T.to_string(),
            R.to_string(),
            B.to_string(),
            op_type,
            "peer-user".to_string(),
        );
        op.revision = Some(rev(seq));
        op
    };
    let upsert = |seq: u64, locale: &str, text: &str| {
        op(
            seq,
            OpType::UpsertTranslationOverlay {
                workspace: WS.to_string(),
                node_id: NODE.to_string(),
                locale: locale.to_string(),
                block_uuid: None,
                overlay: ReplicatedOverlay::from_stored(Some(&title(text))),
                revision: rev(seq),
                history_complete_from: None,
            },
        )
    };
    // The unknown op, as a newer peer encodes it.
    let mut future = serde_json::to_value(op(
        2,
        OpType::DeleteNodeSnapshot {
            node_id: "x".into(),
            revision: rev(2),
            node: None,
            parent_id: None,
        },
    ))
    .unwrap();
    future["op_type"] = serde_json::json!({ "split_shard_v9": { "shard": 7 } });
    let future: Operation = serde_json::from_value(future).unwrap();

    // Over the wire (the TCP message codec), as one batch.
    let bytes = ReplicationMessage::PushOperations {
        operations: vec![upsert(1, "fr", "un"), future, upsert(3, "de", "drei")],
    }
    .to_bytes()
    .unwrap();
    let ReplicationMessage::PushOperations { operations } =
        ReplicationMessage::from_bytes(&bytes).expect("a batch with an unknown op must decode")
    else {
        panic!("wrong message");
    };
    assert!(
        matches!(&operations[1].op_type, OpType::Unknown { tag, .. } if tag == "split_shard_v9")
    );

    // The applier skips it without an error...
    let applicator = OperationApplicator::new(
        replica.storage.db().clone(),
        replica.storage.event_bus(),
        Arc::new(replica.storage.branches_impl().clone()),
    );
    applicator.apply_operation(&operations[1]).await.unwrap();

    // ...and the production receive path applies what surrounds it.
    receive(&replica, &operations).await;
    assert_eq!(
        get(&replica.storage, NODE, "fr", rev(3)).await.unwrap(),
        Some(title("un"))
    );
    assert_eq!(
        get(&replica.storage, NODE, "de", rev(3)).await.unwrap(),
        Some(title("drei"))
    );

    let oplog = OpLogRepository::new(replica.storage.db().clone());
    assert_eq!(
        oplog
            .get_vector_clock_snapshot(T, R)
            .unwrap()
            .get("newer-peer"),
        3,
        "the clock moves past the unknown op"
    );
    let persisted = oplog.get_operations_from_node(T, R, "newer-peer").unwrap();
    let kept = persisted
        .iter()
        .find(|op| op.op_seq == 2)
        .expect("persisted for forwarding");
    assert!(matches!(&kept.op_type, OpType::Unknown { tag, .. } if tag == "split_shard_v9"));
}
