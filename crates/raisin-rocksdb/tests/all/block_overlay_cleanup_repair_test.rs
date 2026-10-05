//! Plan Phase 11c item 3: the `block_overlay_tombstones` repair stores the
//! `T` node deletes never wrote for their block overlays (every database
//! written before Phase 11c, and whatever a race or a checkpoint leaves).
//!
//! Reads do not depend on it (the read rule ends those overlays already);
//! it makes the stored history agree, so retention GC can reclaim the live
//! versions a delete ended and raw scans see the deletion.

use crate::block_overlay_delete_test::{
    block, block_key, listed, stored, translate_block, unstore,
};
use crate::translation_delete_convergence_test::{create, delete, head, ops_since};
use crate::translation_replication_test::{highest_seq, node, receive, Node};
use crate::translation_substrate_test::{title, B, R, T};
use raisin_hlc::HLC;
use raisin_rocksdb::cf;
use raisin_rocksdb::management::async_indexing::repair::{
    pending_block_overlay_branches, run_repair, RepairKind, RepairOptions, RepairReport,
};
use raisin_rocksdb::management::history_gc::{run_history_gc, GcOptions, HistoryRetention};
use std::time::Duration;

fn options() -> RepairOptions {
    RepairOptions {
        // The test volume's free space says nothing about the feature.
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    }
}

async fn repair(n: &Node, options: RepairOptions) -> RepairReport {
    let mut reports = run_repair(
        &n.storage,
        T,
        R,
        Some(B),
        RepairKind::BlockOverlayTombstones,
        options,
    )
    .await
    .unwrap();
    reports.remove(0)
}

async fn sleep() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

fn pending(n: &Node) -> bool {
    pending_block_overlay_branches(&n.storage)
        .unwrap()
        .contains(&(T.to_string(), R.to_string(), B.to_string()))
}

/// What a pre-Phase-11c database holds: `gone` deleted with two block
/// overlays, `again` deleted and recreated, `kept` live — and no block `T`
/// anywhere. Returns the two deletes' revisions.
async fn legacy_history(a: &Node) -> (HLC, HLC) {
    for id in ["gone", "again", "kept"] {
        create(a, id).await;
        translate_block(a, id, "b1", "fr", "Bloc").await;
    }
    translate_block(a, "gone", "b2", "de", "Block").await;
    sleep().await;
    delete(a, "gone").await;
    let gone_at = head(a).await;
    delete(a, "again").await;
    let again_at = head(a).await;
    sleep().await;
    create(a, "again").await;
    for (id, b, locale, at) in [
        ("gone", "b1", "fr", gone_at),
        ("gone", "b2", "de", gone_at),
        ("again", "b1", "fr", again_at),
    ] {
        unstore(a, id, b, locale, &at);
    }
    (gone_at, again_at)
}

#[tokio::test]
async fn the_cleanup_stores_every_missing_block_tombstone_once() {
    let a = node("a").await;
    let (gone_at, again_at) = legacy_history(&a).await;
    let now = head(&a).await;
    // Reads are already right; storage is not.
    assert_eq!(block(&a, "gone", "b1", "fr", now).await, None);
    assert_eq!(block(&a, "again", "b1", "fr", now).await, None);
    assert!(stored(&a, "gone", "b1", "fr", &gone_at).is_none());
    assert!(pending(&a), "queued for the automatic chain before it runs");

    let dry = repair(
        &a,
        RepairOptions {
            dry_run: true,
            ..options()
        },
    )
    .await;
    assert_eq!(dry.block_overlays.tombstones, 3);
    assert!(
        stored(&a, "gone", "b1", "fr", &gone_at).is_none(),
        "dry run"
    );

    let report = repair(&a, options()).await;
    assert!(report.completed);
    assert_eq!(report.block_overlays.nodes, 3);
    assert_eq!(report.block_overlays.deleted_nodes, 2);
    assert_eq!(report.block_overlays.tombstones, 3);
    for (id, b, locale, at) in [
        ("gone", "b1", "fr", gone_at),
        ("gone", "b2", "de", gone_at),
        ("again", "b1", "fr", again_at),
    ] {
        assert_eq!(
            stored(&a, id, b, locale, &at).as_deref(),
            Some(&b"T"[..]),
            "{id}/{b}/{locale}"
        );
    }
    // The live node is untouched, and reads are unchanged.
    assert_eq!(
        listed(&a, "kept").await,
        vec![("b1".to_string(), "fr".to_string())]
    );
    assert_eq!(block(&a, "gone", "b2", "de", now).await, None);
    assert!(!pending(&a), "done on this node");

    // Idempotent: a second run over clean data writes nothing.
    let again = repair(&a, options()).await;
    assert_eq!(again.block_overlays.tombstones, 0);
    assert_eq!(again.writes.written, 0);
}

/// A run stopped after its first commit resumes where it stopped.
#[tokio::test]
async fn the_cleanup_resumes_after_a_crash() {
    let a = node("a").await;
    let (gone_at, again_at) = legacy_history(&a).await;
    let first = repair(
        &a,
        RepairOptions {
            batch_bytes: 1,
            stop_after_batches: Some(1),
            ..options()
        },
    )
    .await;
    assert!(!first.completed);
    let rest = repair(&a, options()).await;
    assert!(rest.completed && rest.resumed);
    assert_eq!(
        first.block_overlays.tombstones + rest.block_overlays.tombstones,
        3
    );
    assert!(stored(&a, "gone", "b2", "de", &gone_at).is_some());
    assert!(stored(&a, "again", "b1", "fr", &again_at).is_some());
}

/// The point of storing the `T`: retention GC can now reclaim the live
/// version the delete ended (it keeps the newest version at or below its
/// cutoff, which used to be that live version, forever).
#[tokio::test]
async fn after_the_cleanup_history_gc_reclaims_the_ended_versions() {
    let a = node("a").await;
    create(&a, "page").await;
    translate_block(&a, "page", "b1", "fr", "Bloc").await;
    let translated_at = head(&a).await;
    sleep().await;
    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    unstore(&a, "page", "b1", "fr", &deleted_at);
    repair(&a, options()).await;
    sleep().await;

    run_history_gc(
        &a.storage,
        &GcOptions {
            retention_override: Some(HistoryRetention {
                keep_days: Some(0),
                keep_revisions: None,
            }),
            min_age: Duration::ZERO,
            tenant: Some(T.to_string()),
            ..GcOptions::default()
        },
    )
    .unwrap();
    let db = a.storage.db();
    let live_version = db
        .get_cf(
            db.cf_handle(cf::BLOCK_TRANSLATIONS).unwrap(),
            block_key("page", "b1", "fr", &translated_at),
        )
        .unwrap();
    assert!(
        live_version.is_none(),
        "the ended live version is reclaimed"
    );
    assert_eq!(block(&a, "page", "b1", "fr", head(&a).await).await, None);
}

/// Two peers' batches applied concurrently on one replica: whatever the
/// materializations raced into, the overlay is absent there, and the
/// cleanup leaves the `T` stored.
#[tokio::test]
async fn after_a_concurrent_apply_the_cleanup_stores_the_tombstone() {
    let (a, b, c) = (node("a").await, node("b").await, node("c").await);
    let setup = highest_seq(&a);
    create(&a, "page").await;
    let created = ops_since(&a, setup);
    receive(&b, &created).await;
    receive(&c, &created).await;
    let (a_seq, b_seq) = (highest_seq(&a), highest_seq(&b));

    translate_block(&b, "page", "b1", "fr", "Bloc").await;
    let translated_at = head(&b).await;
    sleep().await;
    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    let (from_a, from_b) = (ops_since(&a, a_seq), ops_since(&b, b_seq));
    tokio::join!(receive(&c, &from_a), receive(&c, &from_b));

    assert_eq!(
        block(&c, "page", "b1", "fr", translated_at).await,
        Some(title("Bloc"))
    );
    assert_eq!(block(&c, "page", "b1", "fr", deleted_at).await, None);
    assert!(listed(&c, "page").await.is_empty());
    repair(&c, options()).await;
    assert_eq!(
        stored(&c, "page", "b1", "fr", &deleted_at).as_deref(),
        Some(&b"T"[..])
    );
}
