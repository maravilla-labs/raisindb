//! A node delete's translation tombstones converge whatever order the delete
//! and the translations reach a node in.
//!
//! Nothing is shipped or derived: a delete of the node at or above a version's
//! revision ends it by the read rule (`translation_read`). Tombstones used to
//! be derived from whatever locales were live on that node at that moment, so
//! an origin that deleted the node before a peer's translation reached it
//! kept the translation live when it arrived, and served it for good while the
//! peer served it deleted. `translation_delete_read_rule_test` has the timing
//! cases the write-time derivation could not close.
//!
//! Also: a transaction that translates a node and then deletes it committed
//! `T` locally (or, for a first translation, kept it live) while replicating
//! the live overlay from its read cache.

use crate::translation_replication_test::{highest_seq, node, receive, Node};
use crate::translation_substrate_test::{code, title, B, R, T, WS};
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node as Content;
use raisin_models::translations::LocaleOverlay;
use raisin_replication::Operation;
use raisin_rocksdb::{fractional_index, OpLogRepository};
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{BranchRepository, Storage, TranslationRepository};
use std::collections::HashMap;

/// Every op `from` captured after `after_seq`.
pub(crate) fn ops_since(from: &Node, after_seq: u64) -> Vec<Operation> {
    let mut ops: Vec<Operation> = OpLogRepository::new(from.storage.db().clone())
        .get_operations_from_node(T, R, &from.id)
        .unwrap()
        .into_iter()
        .filter(|op| op.op_seq > after_seq)
        .collect();
    ops.sort_by_key(|op| op.op_seq);
    ops
}

pub(crate) async fn tx(node: &Node) -> Box<dyn TransactionalContext> {
    let tx = node.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(T, R).unwrap();
    tx.set_branch(B).unwrap();
    tx.set_message("write").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.set_validate_schema(false).unwrap();
    tx
}

pub(crate) async fn create(node: &Node, id: &str) {
    let mut properties = HashMap::new();
    properties.insert("title".to_string(), PropertyValue::String(id.to_string()));
    let content = Content {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/{id}"),
        node_type: "raisin:Folder".to_string(),
        properties,
        order_key: fractional_index::first(),
        parent: Some("/".to_string()),
        created_at: Some(chrono::Utc::now()),
        ..Content::default()
    };
    let tx = tx(node).await;
    tx.put_node(WS, &content).await.unwrap();
    tx.commit().await.unwrap();
}

pub(crate) async fn translate(node: &Node, id: &str, locale: &str, text: &str) {
    let tx = tx(node).await;
    tx.store_translation(WS, id, locale, title(text))
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

pub(crate) async fn delete(node: &Node, id: &str) {
    let tx = tx(node).await;
    tx.delete_node(WS, id).await.unwrap();
    tx.commit().await.unwrap();
}

pub(crate) async fn head(node: &Node) -> HLC {
    node.storage.branches().get_head(T, R, B).await.unwrap()
}

pub(crate) async fn overlay(node: &Node, id: &str, locale: &str, at: HLC) -> Option<LocaleOverlay> {
    node.storage
        .translations()
        .get_translation(T, R, B, WS, id, &code(locale), &at)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_translation_racing_a_delete_converges_in_both_orders() {
    let a = node("a").await;
    let b = node("b").await;
    let setup = highest_seq(&a);
    create(&a, "page").await;
    receive(&b, &ops_since(&a, setup)).await;
    let (a_seq, b_seq) = (highest_seq(&a), highest_seq(&b));

    // A translates; B, not having seen it, deletes the node.
    translate(&a, "page", "de", "Hallo").await;
    let translated_at = head(&a).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    delete(&b, "page").await;
    let deleted_at = head(&b).await;
    assert!(deleted_at > translated_at);

    // A gets the delete after its translation; B the translation after its
    // delete.
    receive(&a, &ops_since(&b, b_seq)).await;
    receive(&b, &ops_since(&a, a_seq)).await;

    for (name, n) in [("a", &a), ("b", &b)] {
        assert_eq!(
            overlay(n, "page", "de", translated_at).await,
            Some(title("Hallo")),
            "{name}: before the delete the translation is there"
        );
        assert_eq!(
            overlay(n, "page", "de", deleted_at).await,
            None,
            "{name}: the delete removed it"
        );
        let listed = n
            .storage
            .translations()
            .list_translations_for_node(T, R, B, WS, "page", &deleted_at)
            .await
            .unwrap();
        assert!(listed.is_empty(), "{name}: {listed:?}");
    }
}

#[tokio::test]
async fn translate_then_delete_in_one_transaction_replicates_the_deletion() {
    let origin = node("origin").await;
    let replica = node("replica").await;
    let setup = highest_seq(&origin);
    create(&origin, "known").await;
    create(&origin, "fresh").await;
    // `known` was translated before; `fresh` is translated for the first time
    // inside the transaction that deletes it.
    translate(&origin, "known", "de", "alt").await;
    receive(&replica, &ops_since(&origin, setup)).await;
    let seq = highest_seq(&origin);

    let tx = tx(&origin).await;
    tx.store_translation(WS, "known", "de", title("neu"))
        .await
        .unwrap();
    tx.store_translation(WS, "fresh", "de", title("neu"))
        .await
        .unwrap();
    tx.delete_node(WS, "known").await.unwrap();
    tx.delete_node(WS, "fresh").await.unwrap();
    tx.commit().await.unwrap();
    let at = head(&origin).await;

    receive(&replica, &ops_since(&origin, seq)).await;
    for id in ["known", "fresh"] {
        assert_eq!(overlay(&origin, id, "de", at).await, None, "origin {id}");
        assert_eq!(overlay(&replica, id, "de", at).await, None, "replica {id}");
    }
}
