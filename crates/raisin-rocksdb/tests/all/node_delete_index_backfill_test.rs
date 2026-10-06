//! The `node_delete_index` backfill: derives `NODE_DELETES` from the `NODES`
//! tombstones of a database written before the index existed, resumes after
//! a stop, writes nothing over a complete index, and only then makes the
//! branch `Ready`. Translation reads answer the same before (the history
//! walk) and after (the index): node overlays, listings and block overlays,
//! at every revision of a history with deletes, re-creates and edits.

use crate::block_overlay_delete_test::{block, translate_block};
use crate::node_delete_index_test::{assert_complete, stored_tombstones};
use crate::translation_delete_convergence_test::{create, delete, head, overlay, translate};
use crate::translation_replication_test::{node, Node};
use crate::translation_substrate_test::{B, R, T, WS};
use raisin_hlc::HLC;
use raisin_rocksdb::management::async_indexing::repair::{
    run_repair, RepairKind, RepairOptions, RepairReport,
};
use raisin_rocksdb::{cf, node_delete_index};
use raisin_storage::{Storage, TranslationRepository};

fn options() -> RepairOptions {
    RepairOptions {
        // The test volume's free space says nothing about the feature.
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    }
}

async fn run(n: &Node, branch: &str, options: RepairOptions) -> RepairReport {
    run_repair(
        &n.storage,
        T,
        R,
        Some(branch),
        RepairKind::NodeDeleteIndex,
        options,
    )
    .await
    .unwrap()
    .remove(0)
}

/// Run the backfill on `branch` to completion.
pub(crate) async fn backfill(n: &Node, branch: &str) -> RepairReport {
    let report = run(n, branch, options()).await;
    assert!(report.completed && report.node_deletes.ready, "{report:?}");
    report
}

/// What a database from before the index holds: the tombstones, no entry,
/// no readiness.
fn forget_the_index(n: &Node) {
    let db = n.storage.db();
    let cf = db.cf_handle(cf::NODE_DELETES).unwrap();
    db.delete_range_cf(cf, b"".as_slice(), [0xFFu8; 8].as_slice())
        .unwrap();
    node_delete_index::state::mark_all_not_built(db).unwrap();
}

async fn sleep() {
    tokio::time::sleep(std::time::Duration::from_millis(3)).await;
}

/// Deletes, re-creates, edits after translations, a node left dead, block
/// overlays. Returns every revision the history passed through.
async fn history(a: &Node) -> Vec<HLC> {
    let mut points = Vec::new();
    for id in ["p1", "p2", "p3", "p4"] {
        create(a, id).await;
    }
    points.push(head(a).await);
    translate(a, "p1", "fr", "Bonjour").await;
    translate_block(a, "p1", "b1", "fr", "Bloc").await;
    translate(a, "p2", "fr", "Salut").await;
    points.push(head(a).await);
    for _ in 0..3 {
        sleep().await;
        create(a, "p1").await; // an edit
        points.push(head(a).await);
    }
    delete(a, "p1").await;
    points.push(head(a).await);
    delete(a, "p2").await; // stays dead
    points.push(head(a).await);
    sleep().await;
    create(a, "p1").await; // re-created under the same id
    points.push(head(a).await);
    translate(a, "p1", "de", "Hallo").await;
    translate(a, "p3", "fr", "Coucou").await;
    points.push(head(a).await);
    delete(a, "p3").await;
    sleep().await;
    create(a, "p3").await;
    translate(a, "p3", "de", "Moin").await;
    for _ in 0..2 {
        sleep().await;
        create(a, "p3").await;
        create(a, "p1").await;
        points.push(head(a).await);
    }
    translate(a, "p4", "fr", "Allo").await;
    delete(a, "p4").await;
    points.push(head(a).await);
    points
}

/// Every translation read the history can be asked, in order.
async fn answers(a: &Node, points: &[HLC]) -> Vec<String> {
    let mut out = Vec::new();
    for at in points {
        for id in ["p1", "p2", "p3", "p4"] {
            for locale in ["fr", "de"] {
                out.push(format!(
                    "{id}/{locale}@{at}: {:?}",
                    overlay(a, id, locale, *at).await
                ));
            }
            let listed = a
                .storage
                .translations()
                .list_translations_for_node(T, R, B, WS, id, at)
                .await
                .unwrap();
            out.push(format!("{id} listed@{at}: {listed:?}"));
            out.push(format!(
                "{id}/b1@{at}: {:?}",
                block(a, id, "b1", "fr", *at).await
            ));
        }
    }
    out
}

#[tokio::test]
async fn reads_answer_the_same_by_the_walk_and_by_the_index() {
    let a = node("a").await;
    let points = history(&a).await;
    assert!(!node_delete_index::is_ready(a.storage.db(), T, R, B));
    let walked = answers(&a, &points).await;

    backfill(&a, B).await;
    assert!(node_delete_index::is_ready(a.storage.db(), T, R, B));
    let indexed = answers(&a, &points).await;
    assert_eq!(walked, indexed);

    // The history exercises what it claims: present and ended overlays.
    assert!(walked.iter().any(|l| l.contains("Some(")));
    assert!(walked.iter().filter(|l| l.ends_with("None")).count() > 10);
    let head = points.last().unwrap();
    assert_eq!(
        overlay(&a, "p1", "fr", *head).await,
        None,
        "ended by the delete"
    );
    assert!(
        overlay(&a, "p1", "de", *head).await.is_some(),
        "after the re-create"
    );
}

#[tokio::test]
async fn the_backfill_derives_the_index_resumes_and_is_idempotent() {
    let a = node("a").await;
    history(&a).await;
    let tombstones = stored_tombstones(&a.storage, T, R, B).len();
    assert!(tombstones >= 4);
    forget_the_index(&a);
    assert!(
        node_delete_index::recorded_deletes(a.storage.db(), (T, R, B, WS), "p1")
            .unwrap()
            .is_empty()
    );

    // Stop after the first batch, as if the process had died.
    let stopped = run(
        &a,
        B,
        RepairOptions {
            batch_bytes: 64,
            stop_after_batches: Some(1),
            ..options()
        },
    )
    .await;
    assert!(!stopped.completed && !stopped.node_deletes.ready);
    assert!(!node_delete_index::is_ready(a.storage.db(), T, R, B));

    // The next run continues from the cursor and completes.
    let resumed = run(
        &a,
        B,
        RepairOptions {
            batch_bytes: 64,
            ..options()
        },
    )
    .await;
    assert!(resumed.resumed && resumed.completed && resumed.node_deletes.ready);
    assert_eq!(
        (stopped.node_deletes.written + resumed.node_deletes.written) as usize,
        tombstones,
        "every tombstone's entry written exactly once"
    );
    assert_eq!(assert_complete(&a.storage, T, R, B), tombstones);
    assert!(node_delete_index::is_ready(a.storage.db(), T, R, B));

    // Over a complete index a run writes nothing.
    let again = backfill(&a, B).await;
    assert_eq!(again.node_deletes.written, 0);
    assert_eq!(again.node_deletes.tombstones as usize, tombstones);

    // A dry run counts and changes nothing, readiness included.
    node_delete_index::state::mark_all_not_built(a.storage.db()).unwrap();
    let dry = run(
        &a,
        B,
        RepairOptions {
            dry_run: true,
            ..options()
        },
    )
    .await;
    assert!(dry.completed && !dry.node_deletes.ready);
    assert!(!node_delete_index::is_ready(a.storage.db(), T, R, B));
}

#[tokio::test]
async fn an_invalidation_while_the_backfill_runs_keeps_the_branch_walking() {
    let a = node("a").await;
    history(&a).await;
    forget_the_index(&a);
    // A checkpoint ingest lands between two batches of the backfill.
    let db = a.storage.db().clone();
    let hook = raisin_rocksdb::management::async_indexing::repair::CommitHook(std::sync::Arc::new(
        move || {
            node_delete_index::state::mark_all_not_built(&db).unwrap();
        },
    ));
    let report = run(
        &a,
        B,
        RepairOptions {
            batch_bytes: 64,
            before_commit: Some(hook),
            ..options()
        },
    )
    .await;
    assert!(report.completed);
    assert!(
        !report.node_deletes.ready,
        "a run that saw an ingest cannot vouch for it"
    );
    assert!(!node_delete_index::is_ready(a.storage.db(), T, R, B));
    // The next run starts over under a new generation and completes.
    backfill(&a, B).await;
}
