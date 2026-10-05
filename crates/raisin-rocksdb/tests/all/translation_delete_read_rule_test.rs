//! A node delete ends its node overlays by a READ rule, so the answer does not
//! depend on when, where or in what order the delete and a translation below
//! it were written.
//!
//! The tombstones used to be DERIVED at write time — the delete staged a `T`
//! for every locale it saw live, and a replicated version arriving after a
//! delete it preceded staged that delete's `T`. Each half was a read followed
//! by a separate write, and the second ran only in the replication apply arm:
//! - the origin kept a translation committed BELOW a delete that had already
//!   staged its tombstones, while every replica tombstoned it;
//! - a replica applying a peer's delete and another peer's translation
//!   concurrently missed both derivations and kept it live;
//! - a checkpoint ingest ran no derivation at all;
//! and history GC, which drops node tombstones, would now remove the rule's
//! evidence — so it writes the equivalent `T` first.

use crate::translation_delete_convergence_test::{
    create, delete, head, ops_since, overlay, translate, tx,
};
use crate::translation_replication_test::{highest_seq, node, receive, Node};
use crate::translation_substrate_test::{code, meta, rev, store, title, B, R, T, WS};
use raisin_hlc::HLC;
use raisin_replication::OpType;
use raisin_rocksdb::cf;
use raisin_rocksdb::management::count_translation_overlays;
use raisin_rocksdb::management::history_gc::{run_history_gc, GcOptions, HistoryRetention};
use raisin_storage::{Storage, TranslationRepository};
use std::time::Duration;

/// Everything a read can say about `page`'s `de` translation at `at` is "absent".
async fn assert_absent(name: &str, n: &Node, at: HLC) {
    assert_eq!(
        overlay(n, "page", "de", at).await,
        None,
        "{name}: get_translation"
    );
    let translations = n.storage.translations();
    let locales = translations
        .list_translations_for_node(T, R, B, WS, "page", &at)
        .await
        .unwrap();
    assert!(locales.is_empty(), "{name}: locales {locales:?}");
    let listed = translations
        .list_nodes_with_translation(T, R, B, WS, &code("de"), &at)
        .await
        .unwrap();
    assert!(
        !listed.contains(&"page".to_string()),
        "{name}: listed {listed:?}"
    );
}

fn is_translation(op: &raisin_replication::Operation) -> bool {
    matches!(op.op_type, OpType::UpsertTranslationOverlay { .. })
}

#[tokio::test]
async fn a_translation_committed_below_a_delete_that_staged_first_is_absent_everywhere() {
    let (a, b, c) = (node("a").await, node("b").await, node("c").await);
    let setup = highest_seq(&a);
    create(&a, "page").await;
    let created_at = head(&a).await;

    // The translation allocates its revision first (TranslationService takes
    // one, then reads the node) ...
    let translated_at = HLC::new(created_at.timestamp_ms, created_at.counter + 1);
    tokio::time::sleep(Duration::from_millis(5)).await;
    // ... a delete commits meanwhile, above it ...
    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    assert!(deleted_at > translated_at);
    // ... and the translation commits after the delete, below it.
    a.storage
        .translations()
        .store_translation(
            T,
            R,
            B,
            WS,
            "page",
            &code("de"),
            &title("Hallo"),
            &meta("de", translated_at),
        )
        .await
        .unwrap();

    // B gets the oplog in order (delete, then the translation); C gets the
    // translation first.
    let ops = ops_since(&a, setup);
    receive(&b, &ops).await;
    let (translations, rest): (Vec<_>, Vec<_>) = ops.into_iter().partition(is_translation);
    receive(&c, &translations).await;
    receive(&c, &rest).await;

    for (name, n) in [("origin", &a), ("in order", &b), ("reversed", &c)] {
        assert_eq!(
            overlay(n, "page", "de", translated_at).await,
            Some(title("Hallo")),
            "{name}: below the delete the translation is there"
        );
        assert_absent(name, n, deleted_at).await;
        let counted = count_translation_overlays(&n.storage, T, R, "de")
            .await
            .unwrap();
        assert_eq!(
            counted.node_overlays, 0,
            "{name}: counted a deleted node's overlay"
        );
    }
}

#[tokio::test]
async fn a_delete_and_a_peers_translation_applied_concurrently_converge() {
    let (a, b, c) = (node("a").await, node("b").await, node("c").await);
    let setup = highest_seq(&a);
    create(&a, "page").await;
    let created = ops_since(&a, setup);
    receive(&b, &created).await;
    receive(&c, &created).await;
    let (a_seq, b_seq) = (highest_seq(&a), highest_seq(&b));

    // B translates; A, not having seen it, deletes the node above it.
    translate(&b, "page", "de", "Hallo").await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    let (from_a, from_b) = (ops_since(&a, a_seq), ops_since(&b, b_seq));

    // C takes both peers' batches at the same time (one task per connection
    // in production); A and B exchange theirs.
    tokio::join!(receive(&c, &from_a), receive(&c, &from_b));
    receive(&a, &from_b).await;
    receive(&b, &from_a).await;

    for (name, n) in [("a", &a), ("b", &b), ("c", &c)] {
        assert_absent(name, n, deleted_at).await;
    }
}

#[tokio::test]
async fn a_checkpoint_carrying_a_translation_below_a_stored_delete_does_not_revive_it() {
    let (a, c) = (node("a").await, node("c").await);
    let setup = highest_seq(&a);
    create(&a, "page").await;
    let created_at = head(&a).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    receive(&c, &ops_since(&a, setup)).await;

    // A checkpoint ingest put-merges TRANSLATION_DATA as stored on its peer,
    // with no derivation: a version below the delete C already holds.
    let below = HLC::new(created_at.timestamp_ms, created_at.counter + 1);
    let db = c.storage.db();
    let mut key = format!("{T}\0{R}\0{B}\0{WS}\0translations\0page\0de\0").into_bytes();
    key.extend_from_slice(&below.encode_descending());
    let value = serde_json::to_vec(&title("Hallo")).unwrap();
    db.put_cf(db.cf_handle(cf::TRANSLATION_DATA).unwrap(), key, value)
        .unwrap();
    let mut index = format!("{T}\0{R}\0translation_index\0de\0").into_bytes();
    index.extend_from_slice(&below.encode_descending());
    index.push(0);
    index.extend_from_slice(b"page");
    db.put_cf(db.cf_handle(cf::TRANSLATION_INDEX).unwrap(), index, b"")
        .unwrap();

    assert_eq!(overlay(&c, "page", "de", below).await, Some(title("Hallo")));
    assert_absent("c", &c, deleted_at).await;
}

#[tokio::test]
async fn history_gc_dropping_a_node_delete_keeps_its_translation_absent() {
    let a = node("a").await;
    create(&a, "page").await;
    translate(&a, "page", "de", "Hallo").await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    // Recreated under the same id: the delete is now a tombstone between two
    // live versions, below the cutoff and not the newest there — retention
    // drops it.
    create(&a, "page").await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_absent("before GC", &a, head(&a).await).await;

    let report = run_history_gc(
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
    assert!(report.versions_deleted > 0);

    let db = a.storage.db();
    let mut tombstone = format!("{T}\0{R}\0{B}\0{WS}\0nodes\0page\0").into_bytes();
    tombstone.extend_from_slice(&deleted_at.encode_descending());
    assert!(
        db.get_cf(db.cf_handle(cf::NODES).unwrap(), &tombstone)
            .unwrap()
            .is_none(),
        "the scenario needs GC to drop the node's delete tombstone"
    );
    let at = head(&a).await;
    assert_absent("after GC", &a, at).await;
    assert_absent("after GC, at the delete", &a, deleted_at).await;
}

async fn meta_of(n: &Node) -> raisin_models::translations::TranslationMeta {
    n.storage
        .translations()
        .get_translation_meta(T, R, B, WS, "page", &code("de"))
        .await
        .unwrap()
        .expect("meta")
}

/// LOW (Phase 11b review): a transaction translation write stored no
/// `TranslationMeta`, while the replica's apply arm stored one for the same
/// version, so "most recent translation update" differed between them.
#[tokio::test]
async fn a_transaction_translation_has_the_same_meta_on_origin_and_replica() {
    let (origin, replica) = (node("origin").await, node("replica").await);
    let setup = highest_seq(&origin);
    create(&origin, "page").await;
    // An earlier repository write by another actor.
    store(&origin.storage, "page", "de", title("alt"), rev(1)).await;

    let tx = tx(&origin).await;
    tx.store_translation(WS, "page", "de", title("neu"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let written_at = head(&origin).await;
    receive(&replica, &ops_since(&origin, setup)).await;

    let (on_origin, on_replica) = (meta_of(&origin).await, meta_of(&replica).await);
    assert_eq!(
        on_origin.revision, written_at,
        "origin: the transaction's version"
    );
    assert_eq!(
        (on_replica.revision, on_replica.actor, on_replica.message),
        (on_origin.revision, on_origin.actor, on_origin.message)
    );
}
