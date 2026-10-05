//! Plan Phase 13c: localized-name sibling uniqueness on the TRANSACTION
//! translation write (package install, SQL `UPDATE … FOR LOCALE`, the core
//! copies) — refused at staging against stored state and the transaction's
//! own writes, each judged by its final view, and again at commit.

use super::support::*;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleOverlay};
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use std::collections::HashMap;

fn name_overlay(name: &str) -> LocaleOverlay {
    LocaleOverlay::properties(HashMap::from([(
        JsonPointer::new("/__node_name"),
        PropertyValue::String(name.to_string()),
    )]))
}

async fn tx(storage: &raisin_rocksdb::RocksDBStorage) -> Box<dyn TransactionalContext> {
    let tx = storage.begin_context().await.unwrap();
    tx.set_tenant_repo(T, R).unwrap();
    tx.set_branch(B).unwrap();
    tx.set_actor("test").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.set_validate_schema(false).unwrap();
    tx
}

/// Built clean, with or without enforcement.
async fn setup(
    enforce: bool,
) -> (
    std::sync::Arc<raisin_rocksdb::RocksDBStorage>,
    tempfile::TempDir,
) {
    let (storage, dir) = open().await;
    let mut config = repo_config();
    config.localized_names.enforce_unique = enforce;
    set_config(&storage, config).await;
    build(&storage, B).await;
    (storage, dir)
}

fn is_conflict<T: std::fmt::Debug>(result: &raisin_error::Result<T>) -> bool {
    matches!(result, Err(raisin_error::Error::Conflict(_)))
}

/// The owner's report: the transaction overlay write did not check, so a
/// package overlay (or `UPDATE … FOR LOCALE … SET __node_name`) took a
/// sibling's URL. The value is normalized by the one selector first
/// (`/chaise/` is `chaise`).
#[tokio::test]
async fn transaction_overlay_colliding_with_a_sibling_is_refused_when_enforced() {
    let (storage, _dir) = setup(true).await;
    let products = create(&storage, "/products", &[]).await;
    create(&storage, "/products/chaise", &[]).await;
    let table = create(&storage, "/products/table", &[]).await;

    let t = tx(&storage).await;
    let refused = t
        .store_translation(WS, &table, "fr", name_overlay(" /chaise/ "))
        .await;
    assert!(is_conflict(&refused), "{refused:?}");
    // Refused before staging: the transaction is untouched and still commits.
    t.store_translation(WS, &table, "fr", name_overlay("tableau"))
        .await
        .unwrap();
    t.commit().await.unwrap();
    assert_eq!(
        claimants(&storage, B, "fr", &products, "tableau"),
        vec![table]
    );
    assert!(claimants(&storage, B, "fr", &products, "chaise").is_empty());
}

#[tokio::test]
async fn transaction_overlay_colliding_with_a_sibling_is_accepted_when_not_enforced() {
    let (storage, _dir) = setup(false).await;
    let products = create(&storage, "/products", &[]).await;
    create(&storage, "/products/chaise", &[]).await;
    let table = create(&storage, "/products/table", &[]).await;

    let t = tx(&storage).await;
    t.store_translation(WS, &table, "fr", name_overlay("/chaise/"))
        .await
        .unwrap();
    t.commit().await.unwrap();
    assert_eq!(
        claimants(&storage, B, "fr", &products, "chaise"),
        vec![table]
    );
}

/// Two siblings named alike by ONE transaction: neither is stored when the
/// other is checked, so both used to pass (one statement of
/// `UPDATE … FOR LOCALE 'fr' SET __node_name = 'x' WHERE CHILD_OF(…)`).
#[tokio::test]
async fn two_siblings_named_alike_in_one_transaction_are_refused() {
    let (storage, _dir) = setup(true).await;
    create(&storage, "/products", &[]).await;
    let a = create(&storage, "/products/a", &[]).await;
    let b = create(&storage, "/products/b", &[]).await;

    let t = tx(&storage).await;
    t.store_translation(WS, &a, "fr", name_overlay("x"))
        .await
        .unwrap();
    let refused = t.store_translation(WS, &b, "fr", name_overlay("x")).await;
    assert!(is_conflict(&refused), "{refused:?}");
}

/// A sibling the same transaction CREATES is reached by its canonical name:
/// the package-install shape (`/site/a` and `/site/b` in one batch, then
/// `b.node.fr.yaml` with `__node_name: a`).
#[tokio::test]
async fn a_sibling_created_in_the_same_transaction_counts() {
    let (storage, _dir) = setup(true).await;
    create(&storage, "/site", &[]).await;
    let b = create(&storage, "/site/b", &[]).await;

    let t = tx(&storage).await;
    t.add_node(WS, &page("/site/a", &[])).await.unwrap();
    let refused = t.store_translation(WS, &b, "fr", name_overlay("a")).await;
    assert!(is_conflict(&refused), "{refused:?}");
}

/// A stored sibling the same transaction RENAMES is judged by its final
/// view: its old name is free for another sibling in the same commit.
#[tokio::test]
async fn a_name_given_up_in_the_same_transaction_is_free() {
    let (storage, _dir) = setup(true).await;
    let products = create(&storage, "/products", &[]).await;
    let a = create(&storage, "/products/a", &[]).await;
    let b = create(&storage, "/products/b", &[]).await;
    set_name(&storage, &a, "fr", "x").await;

    let t = tx(&storage).await;
    t.store_translation(WS, &a, "fr", name_overlay("y"))
        .await
        .unwrap();
    t.store_translation(WS, &b, "fr", name_overlay("x"))
        .await
        .unwrap();
    t.commit().await.unwrap();
    assert_eq!(claimants(&storage, B, "fr", &products, "x"), vec![b]);
    assert_eq!(claimants(&storage, B, "fr", &products, "y"), vec![a]);
}

/// Staging passed; a sibling claimed the name before the commit. The
/// commit's check, under the branch lock, is the one that decides.
#[tokio::test]
async fn the_commit_refuses_what_another_write_claimed_after_staging() {
    let (storage, _dir) = setup(true).await;
    let products = create(&storage, "/products", &[]).await;
    let a = create(&storage, "/products/a", &[]).await;
    let b = create(&storage, "/products/b", &[]).await;

    let t = tx(&storage).await;
    t.store_translation(WS, &b, "fr", name_overlay("x"))
        .await
        .unwrap();
    set_name(&storage, &a, "fr", "x").await;
    let refused = t.commit().await;
    assert!(is_conflict(&refused), "{refused:?}");
    assert_eq!(claimants(&storage, B, "fr", &products, "x"), vec![a]);
}
