//! Regressions from the Phase 12 review: sibling uniqueness is over
//! EFFECTIVE names (a translated name, else the canonical name) and covers
//! the repository move and same-branch tree copy.

use super::support::*;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage};

/// Enforcement on, built clean (zero collisions).
async fn enforced() -> (
    std::sync::Arc<raisin_rocksdb::RocksDBStorage>,
    tempfile::TempDir,
) {
    let (storage, dir) = open().await;
    let mut config = repo_config();
    config.localized_names.enforce_unique = true;
    set_config(&storage, config).await;
    build(&storage, B).await;
    (storage, dir)
}

fn is_conflict<T: std::fmt::Debug>(result: &raisin_error::Result<T>) -> bool {
    matches!(result, Err(raisin_error::Error::Conflict(_)))
}

/// A sibling without a French name is reached by its canonical name in
/// French: naming another sibling the same used to be accepted, and the
/// first sibling's localized URL then served the second.
#[tokio::test]
async fn translated_name_equal_to_a_sibling_canonical_name_is_refused() {
    let (storage, _dir) = enforced().await;
    create(&storage, "/products", &[]).await;
    create(&storage, "/products/chaise", &[]).await;
    let table = create(&storage, "/products/table", &[]).await;
    let refused = try_set_name(&storage, &table, "fr", "chaise").await;
    assert!(is_conflict(&refused), "{refused:?}");
    set_name(&storage, &table, "fr", "tableau").await;
}

/// The symmetric case: a new node whose canonical name is a sibling's
/// translated name.
#[tokio::test]
async fn canonical_name_equal_to_a_sibling_translated_name_is_refused() {
    let (storage, _dir) = enforced().await;
    create(&storage, "/products", &[]).await;
    let table = create(&storage, "/products/table", &[]).await;
    set_name(&storage, &table, "fr", "chaise").await;
    let refused = storage
        .nodes()
        .create(
            scope(B),
            page("/products/chaise", &[]),
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await;
    assert!(is_conflict(&refused), "{refused:?}");
}

/// Moving or copying a named node under a parent whose child already has
/// that name used to succeed; the newest claim then took the other node's
/// public URL.
#[tokio::test]
async fn move_and_copy_into_a_colliding_parent_are_refused() {
    let (storage, _dir) = enforced().await;
    create(&storage, "/a", &[]).await;
    create(&storage, "/b", &[]).await;
    let x = create(&storage, "/a/x", &[]).await;
    let y = create(&storage, "/b/y", &[]).await;
    set_name(&storage, &x, "fr", "chaise").await;
    set_name(&storage, &y, "fr", "chaise").await;

    let moved = storage.nodes().move_node(scope(B), &x, "/b/x", None).await;
    assert!(is_conflict(&moved), "{moved:?}");
    let copied = storage
        .nodes()
        .copy_node_tree(scope(B), "/a/x", "/b", None, None)
        .await;
    assert!(is_conflict(&copied), "{copied:?}");
    // Elsewhere both are fine.
    storage
        .nodes()
        .copy_node_tree(scope(B), "/a/x", "/", None, None)
        .await
        .unwrap();
}
