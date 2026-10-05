//! Regressions from the Phase 13c review of localized-name sibling
//! uniqueness, each named after the failure it pins: repository writes
//! checked outside the branch lock, reads bounded at a transaction's (or an
//! in-place rewrite's) revision instead of the newest state, a stored
//! collision wedging every later write of either node, and a sibling judged
//! by a staged overlay a newer stored one had superseded.

use super::support::*;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_rocksdb::indexing::node_lock::test_hooks::pause_commit_of;
use raisin_storage::scope::BranchScope;
use raisin_storage::{CommitMetadata, NodeRepository, NodeTypeRepository, Storage};
use std::time::Duration;

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

const PING: &str = "test:LocalizedPing";

/// The fixture's `raisin:Folder` (a transaction's `put_node` asks the parent
/// type whether it allows the child) and a `versionable: false` type, whose
/// update rewrites the node at its current revision instead of minting one.
async fn register_types(storage: &raisin_rocksdb::RocksDBStorage) {
    register_type(storage, "raisin:Folder", true).await;
    register_type(storage, PING, false).await;
}

async fn register_type(storage: &raisin_rocksdb::RocksDBStorage, name: &str, versionable: bool) {
    let ty = NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec!["*".to_string()],
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(versionable),
        immutable: None,
        publishable: Some(true),
        auditable: Some(false),
        indexable: Some(true),
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    };
    storage
        .node_types()
        .upsert(
            BranchScope::new(T, R, B),
            ty,
            CommitMetadata::system("seed"),
        )
        .await
        .unwrap();
}

/// A `versionable: false` node at `path` created with the French name
/// `fr_name` in ONE transaction, so its record and its overlay share the
/// revision every later in-place rewrite reuses.
async fn ping_named(storage: &raisin_rocksdb::RocksDBStorage, path: &str, fr_name: &str) -> String {
    let mut node = page(path, &[]);
    node.node_type = PING.to_string();
    let t = begin(storage).await;
    t.put_node(WS, &node).await.unwrap();
    t.store_translation(WS, &node.id, "fr", name_overlay(fr_name))
        .await
        .unwrap();
    t.commit().await.unwrap();
    node.id
}

/// Rewrite `id` in place (a property-only SQL UPDATE), in a transaction that
/// also writes another node, so the commit allocates a revision and runs its
/// localized-name step.
async fn refresh_in_place(storage: &raisin_rocksdb::RocksDBStorage, id: &str, other: &str) {
    let mut node = get(storage, id).await;
    node.properties.insert(
        "checked".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("now".to_string()),
    );
    let t = begin(storage).await;
    t.put_node(WS, &page(other, &[])).await.unwrap();
    t.put_node(WS, &node).await.unwrap();
    t.commit().await.unwrap();
}

/// Two repository translation writes naming two siblings alike, the first
/// paused between its check and its write: checked outside the branch lock,
/// the second passed against a state the first had not written yet, and
/// both claims were stored under a `Ready`, zero-collision state record.
#[tokio::test]
async fn concurrent_repository_translations_of_one_name_cannot_both_land() {
    let (storage, _dir) = setup(true).await;
    let p = create(&storage, "/p", &[]).await;
    let a = create(&storage, "/p/a", &[]).await;
    let b = create(&storage, "/p/b", &[]).await;

    let pause = pause_commit_of(storage.db(), &a);
    let first = try_set_name(&storage, &a, "fr", "x");
    let second = async {
        pause.reached().await;
        let mut contender = Box::pin(try_set_name(&storage, &b, "fr", "x"));
        match tokio::time::timeout(Duration::from_millis(400), &mut contender).await {
            Ok(result) => {
                pause.release();
                (true, result)
            }
            Err(_) => {
                pause.release();
                (false, contender.await)
            }
        }
    };
    let (first, (finished_early, second)) = tokio::join!(first, second);
    first.unwrap();
    assert!(!finished_early, "the second write must wait for the lock");
    assert!(is_conflict(&second), "{second:?}");
    assert_eq!(claimants(&storage, B, "fr", &p, "x"), vec![a]);
}

/// A transaction allocates its revision at its first write; a parent created
/// by another commit after that did not resolve "as of" it, so every stored
/// claim under the parent was skipped and the duplicate committed.
#[tokio::test]
async fn parent_created_after_the_transaction_revision_still_guards_its_children() {
    let (storage, _dir) = setup(true).await;
    register_types(&storage).await;
    let t = begin(&storage).await;
    t.put_node(WS, &page("/other", &[])).await.unwrap();

    let p = create(&storage, "/late", &[]).await;
    let s = create(&storage, "/late/s", &[]).await;
    set_name(&storage, &s, "fr", "x").await;

    let n = page("/late/n", &[]);
    t.put_node(WS, &n).await.unwrap();
    let refused = t
        .store_translation(WS, &n.id, "fr", name_overlay("x"))
        .await;
    assert!(is_conflict(&refused), "{refused:?}");
    t.commit().await.unwrap();
    assert_eq!(claimants(&storage, B, "fr", &p, "x"), vec![s]);
    assert!(availability(&storage, B).is_ready());
}

/// A `versionable=false` rewrite reuses the node's old revision: bounded
/// there, the check judged the node by the French name it had then (`x`,
/// since renamed `y` and taken by a sibling) and refused every refresh.
#[tokio::test]
async fn in_place_refresh_is_judged_by_its_current_name_not_its_reused_revision() {
    let (storage, _dir) = setup(true).await;
    register_types(&storage).await;
    let p = create(&storage, "/p", &[]).await;
    let n = ping_named(&storage, "/p/n", "x").await;
    set_name(&storage, &n, "fr", "y").await;
    let s = create(&storage, "/p/s", &[]).await;
    set_name(&storage, &s, "fr", "x").await;

    refresh_in_place(&storage, &n, "/other").await;
    assert_eq!(claimants(&storage, B, "fr", &p, "y"), vec![n]);
    assert_eq!(claimants(&storage, B, "fr", &p, "x"), vec![s]);
}

/// The same rewrite under an ancestor renamed since its revision: the
/// parent path did not exist "as of" it, so the index writer found no parent
/// and invalidated the workspace's build on every refresh (and the check
/// skipped every stored claim under it).
#[tokio::test]
async fn in_place_refresh_under_a_renamed_ancestor_keeps_the_build_ready() {
    let (storage, _dir) = setup(true).await;
    register_types(&storage).await;
    let p = create(&storage, "/p", &[]).await;
    let c = create(&storage, "/p/c", &[]).await;
    // A grandchild: a rename rewrites the root and its direct children
    // (they store its name), never deeper records.
    let n = ping_named(&storage, "/p/c/n", "x").await;
    storage
        .nodes()
        .move_node(scope(B), &p, "/q", None)
        .await
        .unwrap();
    assert!(availability(&storage, B).is_ready());

    refresh_in_place(&storage, &n, "/other").await;
    assert!(availability(&storage, B).is_ready());
    assert_eq!(claimants(&storage, B, "fr", &c, "x"), vec![n]);
}

/// A collision stored while `Ready` with zero collisions (what replication
/// apply leaves; here written before enforcement was switched on, which does
/// not change the build's fingerprint). Every later write of either node was
/// refused — a property update, another locale's overlay, a transaction —
/// though none of them introduced it. A NEW collision is still refused.
#[tokio::test]
async fn stored_collision_does_not_wedge_later_writes_of_either_node() {
    let (storage, _dir) = setup(false).await;
    register_types(&storage).await;
    let p = create(&storage, "/p", &[]).await;
    let chaise = create(&storage, "/p/chaise", &[]).await;
    let table = create(&storage, "/p/table", &[]).await;
    let other = create(&storage, "/p/other", &[]).await;
    set_name(&storage, &table, "fr", "chaise").await;
    let mut config = repo_config();
    config.localized_names.enforce_unique = true;
    set_config(&storage, config).await;
    assert!(availability(&storage, B).is_ready());

    update_props(&storage, &chaise, &[("title", "Chaise")]).await;
    set_name(&storage, &table, "de", "tisch").await;
    let mut node = get(&storage, &table).await;
    node.properties.insert(
        "title".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("Table".to_string()),
    );
    let t = begin(&storage).await;
    t.put_node(WS, &node).await.unwrap();
    t.commit().await.unwrap();

    let refused = try_set_name(&storage, &other, "fr", "chaise").await;
    assert!(is_conflict(&refused), "{refused:?}");
    assert_eq!(claimants(&storage, B, "fr", &p, "chaise"), vec![table]);
}

/// A transaction staged `a` = `x`; a newer repository write renamed `a` to
/// `w`, which wins on read. Judged by the STAGED overlay, `a` still held
/// `x` and the transaction's next sibling naming was refused for a name
/// that was free.
#[tokio::test]
async fn newer_stored_overlay_beats_a_staged_one_when_judging_a_sibling() {
    let (storage, _dir) = setup(true).await;
    let p = create(&storage, "/p", &[]).await;
    let a = create(&storage, "/p/a", &[]).await;
    let c = create(&storage, "/p/c", &[]).await;

    let t = begin(&storage).await;
    t.store_translation(WS, &a, "fr", name_overlay("x"))
        .await
        .unwrap();
    set_name(&storage, &a, "fr", "w").await;
    t.store_translation(WS, &c, "fr", name_overlay("x"))
        .await
        .unwrap();
    t.commit().await.unwrap();
    assert_eq!(claimants(&storage, B, "fr", &p, "x"), vec![c]);
    assert_eq!(claimants(&storage, B, "fr", &p, "w"), vec![a]);
}
