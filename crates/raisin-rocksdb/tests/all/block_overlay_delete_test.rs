//! Plan Phase 11c: a node delete ends its BLOCK overlays.
//!
//! A node delete tombstoned its node overlays (later: ended them by a read
//! rule) but never its `BLOCK_TRANSLATIONS` overlays. They stayed live in
//! storage forever: history GC kept the newest (live) version, raw scans of
//! the CF saw them, and a node recreated under the same id got them back.
//! Now the read rule ends them like node overlays, and the delete funnel, the
//! one writer (for a version landing below a stored delete) and history GC
//! store the `T` too — the same result whichever arrived first.

use crate::translation_delete_convergence_test::{create, delete, head, ops_since, tx};
use crate::translation_replication_test::{highest_seq, node, receive, Node};
use crate::translation_substrate_test::{code, title, B, R, T, WS};
use raisin_hlc::HLC;
use raisin_models::translations::LocaleOverlay;
use raisin_replication::{OpType, Operation, ReplicatedOverlay};
use raisin_rocksdb::cf;
use raisin_rocksdb::management::count_translation_overlays;
use raisin_rocksdb::management::history_gc::{run_history_gc, GcOptions, HistoryRetention};
use raisin_storage::{Storage, TranslationRepository};
use std::time::Duration;

pub(crate) async fn translate_block(n: &Node, id: &str, block: &str, locale: &str, text: &str) {
    let tx = tx(n).await;
    tx.store_block_translation(WS, id, block, locale, title(text))
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// The repository's block read at `at`.
pub(crate) async fn block(
    n: &Node,
    id: &str,
    block: &str,
    locale: &str,
    at: HLC,
) -> Option<LocaleOverlay> {
    n.storage
        .translations()
        .get_block_translation(T, R, B, WS, id, block, &code(locale), &at)
        .await
        .unwrap()
}

/// The repository's HEAD listing of `id`'s block overlays.
pub(crate) async fn listed(n: &Node, id: &str) -> Vec<(String, String)> {
    n.storage
        .translations()
        .list_block_translations_for_node(T, R, B, WS, id)
        .await
        .unwrap()
        .into_iter()
        .map(|(block, locale)| (block, locale.as_str().to_string()))
        .collect()
}

/// The `BLOCK_TRANSLATIONS` key of one version.
pub(crate) fn block_key(id: &str, block: &str, locale: &str, at: &HLC) -> Vec<u8> {
    let mut key =
        format!("{T}\0{R}\0{B}\0{WS}\0block_trans\0{id}\0{block}\0{locale}\0").into_bytes();
    key.extend_from_slice(&at.encode_descending());
    key
}

/// The bytes stored at exactly that version, if any.
pub(crate) fn stored(n: &Node, id: &str, block: &str, locale: &str, at: &HLC) -> Option<Vec<u8>> {
    let db = n.storage.db();
    db.get_cf(
        db.cf_handle(cf::BLOCK_TRANSLATIONS).unwrap(),
        block_key(id, block, locale, at),
    )
    .unwrap()
}

/// Remove a stored version — what a database written before Phase 11c (or a
/// checkpoint from such a peer) looks like: no `T` at the delete.
pub(crate) fn unstore(n: &Node, id: &str, block: &str, locale: &str, at: &HLC) {
    let db = n.storage.db();
    db.delete_cf(
        db.cf_handle(cf::BLOCK_TRANSLATIONS).unwrap(),
        block_key(id, block, locale, at),
    )
    .unwrap();
}

async fn sleep() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

#[tokio::test]
async fn a_node_delete_tombstones_its_block_overlays() {
    let a = node("a").await;
    create(&a, "page").await;
    translate_block(&a, "page", "b1", "fr", "Bloc").await;
    translate_block(&a, "page", "b2", "de", "Block").await;
    let translated_at = head(&a).await;
    sleep().await;
    delete(&a, "page").await;
    let deleted_at = head(&a).await;

    for (b, locale) in [("b1", "fr"), ("b2", "de")] {
        assert!(
            block(&a, "page", b, locale, translated_at).await.is_some(),
            "{b}/{locale}: present before the delete"
        );
        assert_eq!(block(&a, "page", b, locale, deleted_at).await, None);
        assert_eq!(
            stored(&a, "page", b, locale, &deleted_at).as_deref(),
            Some(&b"T"[..]),
            "{b}/{locale}: the delete stored its T"
        );
    }
    assert!(listed(&a, "page").await.is_empty());
}

fn is_translation(op: &Operation) -> bool {
    matches!(op.op_type, OpType::UpsertTranslationOverlay { .. })
}

/// A peer translates a block; another, not having seen it, deletes the node
/// above it. Every node — both peers and two replicas taking the two batches
/// in opposite orders — ends the overlay AND stores the delete's `T`.
#[tokio::test]
async fn block_overlays_converge_whichever_of_the_delete_and_the_version_arrives_first() {
    let (a, b) = (node("a").await, node("b").await);
    let (c1, c2) = (node("c1").await, node("c2").await);
    let setup = highest_seq(&a);
    create(&a, "page").await;
    let created = ops_since(&a, setup);
    for n in [&b, &c1, &c2] {
        receive(n, &created).await;
    }
    let (a_seq, b_seq) = (highest_seq(&a), highest_seq(&b));

    translate_block(&a, "page", "b1", "fr", "Bloc").await;
    let translated_at = head(&a).await;
    sleep().await;
    delete(&b, "page").await;
    let deleted_at = head(&b).await;
    assert!(deleted_at > translated_at);
    let (from_a, from_b) = (ops_since(&a, a_seq), ops_since(&b, b_seq));
    assert!(from_a.iter().any(is_translation));

    receive(&a, &from_b).await; // delete after the version
    receive(&b, &from_a).await; // version after the delete
    receive(&c1, &from_a).await;
    receive(&c1, &from_b).await;
    receive(&c2, &from_b).await;
    receive(&c2, &from_a).await;

    for (name, n) in [("a", &a), ("b", &b), ("c1", &c1), ("c2", &c2)] {
        assert_eq!(
            block(n, "page", "b1", "fr", translated_at).await,
            Some(title("Bloc")),
            "{name}: before the delete the block overlay is there"
        );
        assert_eq!(
            block(n, "page", "b1", "fr", deleted_at).await,
            None,
            "{name}"
        );
        assert!(listed(n, "page").await.is_empty(), "{name}: listed");
        assert_eq!(
            stored(n, "page", "b1", "fr", &deleted_at).as_deref(),
            Some(&b"T"[..]),
            "{name}: the T is stored at the delete"
        );
    }
}

/// A transaction that translates a block and then deletes the node
/// replicates the deletion, not the live overlay from its read cache.
#[tokio::test]
async fn translate_block_then_delete_in_one_transaction_replicates_the_deletion() {
    let (origin, replica) = (node("origin").await, node("replica").await);
    let setup = highest_seq(&origin);
    create(&origin, "page").await;
    receive(&replica, &ops_since(&origin, setup)).await;
    let seq = highest_seq(&origin);

    let tx = tx(&origin).await;
    tx.store_block_translation(WS, "page", "b1", "fr", title("Bloc"))
        .await
        .unwrap();
    tx.delete_node(WS, "page").await.unwrap();
    tx.commit().await.unwrap();
    let at = head(&origin).await;

    let ops = ops_since(&origin, seq);
    let block_ops: Vec<_> = ops
        .iter()
        .filter_map(|op| match &op.op_type {
            OpType::UpsertTranslationOverlay {
                block_uuid: Some(_),
                overlay,
                ..
            } => Some(overlay.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(block_ops, vec![ReplicatedOverlay::Deleted]);
    receive(&replica, &ops).await;
    for (name, n) in [("origin", &origin), ("replica", &replica)] {
        assert_eq!(block(n, "page", "b1", "fr", at).await, None, "{name}");
        assert!(listed(n, "page").await.is_empty(), "{name}");
    }
}

/// Recreated under the same id, a node does not get its block overlays
/// back — even with no `T` stored (a pre-Phase-11c database) — unless they
/// are written again on purpose.
#[tokio::test]
async fn a_recreated_node_does_not_get_its_block_overlays_back() {
    let a = node("a").await;
    create(&a, "page").await;
    translate_block(&a, "page", "b1", "fr", "Bloc").await;
    sleep().await;
    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    unstore(&a, "page", "b1", "fr", &deleted_at);
    sleep().await;
    create(&a, "page").await;

    let at = head(&a).await;
    assert_eq!(block(&a, "page", "b1", "fr", at).await, None);
    assert!(listed(&a, "page").await.is_empty());
    let tx = tx(&a).await;
    assert_eq!(
        tx.get_block_translation(WS, "page", "b1", "fr")
            .await
            .unwrap(),
        None,
        "transaction read"
    );
    drop(tx);

    // Written again on purpose: there.
    translate_block(&a, "page", "b1", "fr", "Neu").await;
    let at = head(&a).await;
    assert_eq!(block(&a, "page", "b1", "fr", at).await, Some(title("Neu")));
    assert_eq!(
        listed(&a, "page").await,
        vec![("b1".to_string(), "fr".to_string())]
    );
}

/// History GC drops a node delete tombstone that sits between two live
/// versions. With no block `T` stored (a pre-Phase-11c database), it must
/// store one first, or the block overlay the delete ended comes back.
#[tokio::test]
async fn history_gc_dropping_a_node_delete_keeps_its_block_overlays_absent() {
    let a = node("a").await;
    create(&a, "page").await;
    translate_block(&a, "page", "b1", "fr", "Bloc").await;
    sleep().await;
    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    unstore(&a, "page", "b1", "fr", &deleted_at);
    sleep().await;
    create(&a, "page").await;
    sleep().await;
    assert_eq!(block(&a, "page", "b1", "fr", head(&a).await).await, None);
    let counted = count_translation_overlays(&a.storage, T, R, "fr")
        .await
        .unwrap();
    assert_eq!(counted.block_overlays, 0, "a deleted node's block overlay");

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
    let mut tombstone = format!("{T}\0{R}\0{B}\0{WS}\0nodes\0page\0").into_bytes();
    tombstone.extend_from_slice(&deleted_at.encode_descending());
    assert!(
        db.get_cf(db.cf_handle(cf::NODES).unwrap(), &tombstone)
            .unwrap()
            .is_none(),
        "the scenario needs GC to drop the node's delete tombstone"
    );
    assert_eq!(block(&a, "page", "b1", "fr", head(&a).await).await, None);
    assert_eq!(block(&a, "page", "b1", "fr", deleted_at).await, None);
    assert!(listed(&a, "page").await.is_empty());
}
