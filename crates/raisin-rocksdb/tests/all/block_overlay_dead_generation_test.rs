//! Plan Phase 11c review: what the first cut of "a node delete ends its
//! block overlays" still got wrong.
//!
//! - A version written ABOVE a delete while the node was still deleted (a
//!   peer that had not seen the delete; a merge replaying a branch's
//!   translation) stayed live at HEAD for good and came back with a
//!   recreate: the rule only ended versions with a delete in `[R, bound]`.
//! - (Not a defect, guarded: a transaction cannot recreate an id it deleted,
//!   so the delete's block `T` never outlives a recreate at its revision.)
//! - A time-travel read lost the blocks a LATER delete ended: the resolver
//!   chose blocks from a HEAD listing.
//! - History GC dropping the delete that ended a dead-generation version
//!   brought it back.

use crate::block_overlay_delete_test::{block, listed, stored, translate_block};
use crate::translation_delete_convergence_test::{
    create, delete, head, ops_since, overlay, translate, tx,
};
use crate::translation_replication_test::{highest_seq, node, receive};
use crate::translation_substrate_test::{code, title, B, R, T, WS};
use raisin_context::RepositoryConfig;
use raisin_core::TranslationResolver;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node as Content;
use raisin_rocksdb::cf;
use raisin_rocksdb::fractional_index;
use raisin_rocksdb::management::count_translation_overlays;
use raisin_rocksdb::management::history_gc::{run_history_gc, GcOptions, HistoryRetention};
use raisin_storage::Storage;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

async fn sleep() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

fn page(text: &str) -> Content {
    let mut block = HashMap::new();
    block.insert("uuid".to_string(), PropertyValue::String("b1".to_string()));
    block.insert("title".to_string(), PropertyValue::String(text.to_string()));
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String("page".to_string()),
    );
    properties.insert(
        "content".to_string(),
        PropertyValue::Array(vec![PropertyValue::Object(block)]),
    );
    Content {
        id: "page".to_string(),
        name: "page".to_string(),
        path: "/page".to_string(),
        node_type: "raisin:Folder".to_string(),
        properties,
        order_key: fractional_index::first(),
        parent: Some("/".to_string()),
        created_at: Some(chrono::Utc::now()),
        ..Content::default()
    }
}

fn block_title(node: &Content) -> Option<String> {
    match node.properties.get("content")? {
        PropertyValue::Array(items) => match items.first()? {
            PropertyValue::Object(block) => match block.get("title")? {
                PropertyValue::String(text) => Some(text.clone()),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Peer `b` has not seen `a`'s delete and translates above it. Once the two
/// have exchanged, the node is deleted everywhere and the overlays — block
/// and node — are absent at HEAD, unlisted, uncounted, and stay absent when
/// the node is recreated under the same id.
#[tokio::test]
async fn a_translation_written_above_a_delete_by_a_peer_that_had_not_seen_it_stays_ended() {
    let (a, b) = (node("a").await, node("b").await);
    let setup = highest_seq(&a);
    create(&a, "page").await;
    receive(&b, &ops_since(&a, setup)).await;
    let (a_seq, b_seq) = (highest_seq(&a), highest_seq(&b));

    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    sleep().await;
    translate_block(&b, "page", "b1", "fr", "Bloc").await;
    translate(&b, "page", "fr", "Page").await;
    let translated_at = head(&b).await;
    assert!(
        translated_at > deleted_at,
        "the version is above the delete"
    );
    let (from_a, from_b) = (ops_since(&a, a_seq), ops_since(&b, b_seq));
    receive(&a, &from_b).await;
    receive(&b, &from_a).await;

    for (name, n) in [("a", &a), ("b", &b)] {
        let at = head(n).await;
        assert_eq!(block(n, "page", "b1", "fr", at).await, None, "{name}");
        assert!(listed(n, "page").await.is_empty(), "{name}: listed");
        assert_eq!(overlay(n, "page", "fr", at).await, None, "{name}: node");
        let counted = count_translation_overlays(&n.storage, T, R, "fr")
            .await
            .unwrap();
        assert_eq!(counted.total(), 0, "{name}: counted");
    }

    sleep().await;
    create(&a, "page").await;
    let at = head(&a).await;
    assert_eq!(block(&a, "page", "b1", "fr", at).await, None, "recreated");
    assert!(listed(&a, "page").await.is_empty(), "recreated: listed");
    assert_eq!(overlay(&a, "page", "fr", at).await, None, "recreated: node");
}

/// The delete funnel stores a deleted node's block `T`s at the delete's
/// revision as soon as the delete is staged. That is right only while no live
/// `NODES` record lands at the SAME revision — a transaction recreating the
/// id it just deleted would overwrite the tombstone and leave the `T` ending
/// the overlays of a node that was never deleted. A transaction cannot:
/// creating an id checks it against committed state. If that ever changes,
/// step 13 of `tombstones::add_node_tombstones_with_parent` must move to the
/// commit, for the nodes still deleted there.
#[tokio::test]
async fn a_transaction_cannot_recreate_a_node_it_deleted() {
    let a = node("a").await;
    create(&a, "page").await;
    translate_block(&a, "page", "b1", "fr", "Bloc").await;

    let recreate = tx(&a).await;
    recreate.delete_node(WS, "page").await.unwrap();
    let refused = recreate.put_node(WS, &page("Block")).await;
    assert!(
        matches!(refused, Err(raisin_error::Error::Conflict(_))),
        "{refused:?}"
    );
    drop(recreate);
    let at = head(&a).await;
    assert_eq!(block(&a, "page", "b1", "fr", at).await, Some(title("Bloc")));

    // Deleted in a transaction: the delete stores the block `T`.
    let remove = tx(&a).await;
    remove.delete_node(WS, "page").await.unwrap();
    remove.commit().await.unwrap();
    let deleted_at = head(&a).await;
    assert_eq!(
        stored(&a, "page", "b1", "fr", &deleted_at).as_deref(),
        Some(&b"T"[..])
    );
    assert_eq!(block(&a, "page", "b1", "fr", deleted_at).await, None);
}

/// A read at a revision between a block translation and a later delete of
/// the node applies the block translation. The resolver used to pick the
/// blocks from a HEAD listing, where the delete had already ended them.
#[tokio::test]
async fn a_time_travel_read_applies_the_block_translations_a_later_delete_ended() {
    let a = node("a").await;
    create(&a, "page").await;
    translate_block(&a, "page", "b1", "fr", "Bloc").await;
    let translated_at = head(&a).await;
    sleep().await;
    delete(&a, "page").await;

    let resolver = TranslationResolver::new(
        Arc::new(a.storage.translations().clone()),
        RepositoryConfig::default(),
    );
    let resolved = resolver
        .resolve_node(T, R, B, WS, page("Block"), &code("fr"), &translated_at)
        .await
        .unwrap()
        .expect("visible");
    assert_eq!(block_title(&resolved).as_deref(), Some("Bloc"));
    let batch = resolver
        .resolve_nodes_batch(
            T,
            R,
            B,
            WS,
            vec![page("Block")],
            &code("fr"),
            &translated_at,
        )
        .await
        .unwrap();
    assert_eq!(block_title(&batch[0]).as_deref(), Some("Bloc"), "batch");

    // At HEAD the delete has ended it.
    let resolved = resolver
        .resolve_node(T, R, B, WS, page("Block"), &code("fr"), &head(&a).await)
        .await
        .unwrap()
        .expect("visible");
    assert_eq!(block_title(&resolved).as_deref(), Some("Block"));
}

/// History GC drops the delete that ended a dead-generation version (the
/// node was recreated above it). It must store the version's `T` first, or
/// the version — the rule's evidence gone — is live again.
#[tokio::test]
async fn history_gc_dropping_a_delete_keeps_a_dead_generation_version_ended() {
    let (a, b) = (node("a").await, node("b").await);
    let setup = highest_seq(&a);
    create(&a, "page").await;
    receive(&b, &ops_since(&a, setup)).await;
    let b_seq = highest_seq(&b);
    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    sleep().await;
    translate_block(&b, "page", "b1", "fr", "Bloc").await;
    let translated_at = head(&b).await;
    assert!(translated_at > deleted_at);
    receive(&a, &ops_since(&b, b_seq)).await;
    sleep().await;
    create(&a, "page").await;
    sleep().await;
    assert!(listed(&a, "page").await.is_empty());

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
    assert!(listed(&a, "page").await.is_empty(), "listed after GC");
    assert_eq!(block(&a, "page", "b1", "fr", head(&a).await).await, None);
    assert_ne!(
        stored(&a, "page", "b1", "fr", &translated_at),
        Some(title_bytes("Bloc")),
        "the dead-generation version is no longer stored live"
    );
}

fn title_bytes(text: &str) -> Vec<u8> {
    serde_json::to_vec(&title(text)).unwrap()
}
