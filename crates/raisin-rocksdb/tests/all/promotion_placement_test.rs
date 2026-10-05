//! Plan Phase 13a: ONE promotion moves node A out of a path and node B into
//! it (or swaps the two paths). PATH_INDEX keys carry no node id and every
//! write of a promotion lands at one revision, so A's stale-path tombstone and
//! B's new mapping are the same key: whichever the batch wrote last won. Staged
//! B-first, the path resolved to nothing on the origin; staged A-first, B's
//! displacement check took A (still at the path in committed state) for a
//! stale occupant and deleted it — on the origin and on every replica.
//!
//! Each scenario runs in both staging orders (the source tree's sibling order
//! decides it) and is checked on the origin and on replicas fed the oplog as
//! stored, decomposed, and decomposed with deletes moved last.

use crate::cross_branch_displacement_test::{
    by_path, child, children, every_peer, origin_ops, path_owner_raw, replicas, replicas_with, set,
};
use crate::cross_branch_prune_replication_test::{
    create, folder, on_publish, promote_with, with_publish_branch,
};
use crate::translation_replication_test::{node, Node as Peer};
use crate::translation_substrate_test::{B, R, T, WS};
use raisin_models::nodes::Node;
use raisin_replication::operation::ReplicatedNodeChangeKind;
use raisin_replication::OpType;
use raisin_storage::{NodeRepository, Storage, StorageScope};

const PUBLISH: &str = crate::cross_branch_prune_replication_test::PUBLISH;

async fn rename_on_main(peer: &Peer, path: &str, name: &str) {
    peer.storage
        .nodes()
        .rename_node(StorageScope::new(T, R, B, WS), path, name)
        .await
        .expect("rename");
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
}

/// Put `name` before `before` among `/section`'s children on main.
async fn order_before(peer: &Peer, name: &str, before: &str) {
    peer.storage
        .nodes()
        .move_child_before(
            StorageScope::new(T, R, B, WS),
            "/section",
            name,
            before,
            None,
            None,
        )
        .await
        .expect("reorder");
}

/// The promotion's `ApplyRevision` (the newest on publish) deletes nothing.
fn last_promotion_deletes_nothing(origin: &Peer) {
    let changes = origin_ops(origin)
        .into_iter()
        .filter(|op| op.branch == PUBLISH)
        .filter_map(|op| match op.op_type {
            OpType::ApplyRevision { node_changes, .. } => Some(node_changes),
            _ => None,
        })
        .last()
        .expect("the promotion's ApplyRevision");
    let deleted: Vec<&str> = changes
        .iter()
        .filter(|c| c.kind == ReplicatedNodeChangeKind::Delete)
        .map(|c| c.node.path.as_str())
        .collect();
    assert!(deleted.is_empty(), "a mover was deleted: {deleted:?}");
}

/// On every peer: each `(node, path)` is live at that path and owns its
/// PATH_INDEX entry, and `/section` lists exactly these nodes.
async fn assert_placed(origin: &Peer, section: &Node, placed: &[(&Node, &str)]) {
    last_promotion_deletes_nothing(origin);
    let replicas = replicas(origin).await;
    let ids: Vec<&str> = placed.iter().map(|(n, _)| n.id.as_str()).collect();
    for (name, peer) in every_peer(origin, &replicas).await {
        for (n, path) in placed {
            assert_eq!(
                by_path(peer, path).await.as_deref(),
                Some(n.id.as_str()),
                "{name}: {path} does not resolve to its new owner"
            );
            assert_eq!(
                path_owner_raw(peer, path).as_deref(),
                Some(n.id.as_str()),
                "{name}: PATH_INDEX for {path} does not name its new owner"
            );
            let live = on_publish(peer, &n.id).await;
            assert_eq!(
                live.map(|l| l.path).as_deref(),
                Some(*path),
                "{name}: {} is not live at {path}",
                n.id
            );
        }
        assert_eq!(children(peer, &section.id), set(&ids), "{name}");
    }
}

/// `/section` with `first` then `second` as children, promoted once.
async fn seeded(first: &str, second: &str) -> (Peer, Node, Node, Node) {
    let origin = node("origin").await;
    with_publish_branch(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    let n1 = child(first, &section);
    create(&origin, &n1).await;
    let n2 = child(second, &section);
    create(&origin, &n2).await;
    promote_with(&origin, false).await;
    (origin, section, n1, n2)
}

#[tokio::test]
async fn a_move_in_staged_before_the_move_out_keeps_the_path() {
    // B is listed first on the source: it is staged before A.
    let (origin, section, b, a) = seeded("b", "p").await;
    rename_on_main(&origin, "/section/p", "q").await; // A out of /section/p
    rename_on_main(&origin, "/section/b", "p").await; // B into it
    promote_with(&origin, false).await;
    assert_placed(&origin, &section, &[(&a, "/section/q"), (&b, "/section/p")]).await;
}

#[tokio::test]
async fn a_move_out_staged_before_the_move_in_keeps_both_nodes() {
    // A is listed first on the source: it is staged before B.
    let (origin, section, a, b) = seeded("p", "b").await;
    rename_on_main(&origin, "/section/p", "q").await;
    rename_on_main(&origin, "/section/b", "p").await;
    promote_with(&origin, true).await;
    assert_placed(&origin, &section, &[(&a, "/section/q"), (&b, "/section/p")]).await;
}

#[tokio::test]
async fn a_node_created_into_a_vacated_path_keeps_it_in_both_orders() {
    for created_first in [false, true] {
        let origin = node("origin").await;
        with_publish_branch(&origin).await;
        let section = folder("section", "/section");
        create(&origin, &section).await;
        let a = child("p", &section);
        create(&origin, &a).await;
        promote_with(&origin, true).await;

        rename_on_main(&origin, "/section/p", "q").await;
        let b = child("p", &section);
        create(&origin, &b).await;
        if created_first {
            order_before(&origin, "p", "q").await;
        }
        promote_with(&origin, true).await;
        assert_placed(&origin, &section, &[(&a, "/section/q"), (&b, "/section/p")]).await;
    }
}

#[tokio::test]
async fn a_swap_of_two_paths_in_one_promotion() {
    let (origin, section, a, b) = seeded("x", "y").await;
    rename_on_main(&origin, "/section/x", "tmp").await;
    rename_on_main(&origin, "/section/y", "x").await;
    rename_on_main(&origin, "/section/tmp", "y").await;
    promote_with(&origin, true).await;
    assert_placed(&origin, &section, &[(&a, "/section/y"), (&b, "/section/x")]).await;
}

/// Plant, on `peer`'s publish branch, a PATH_INDEX entry mapping `path` to
/// [`STRANDED`] at `at` — a stranded duplicate owning a path a node's record
/// still claims (the shape the `path_index` repair reports).
fn plant_stranded(peer: &Peer, path: &str, at: &raisin_hlc::HLC) {
    let db = peer.storage.db();
    let cf = db.cf_handle(raisin_rocksdb::cf::PATH_INDEX).unwrap();
    let key = raisin_rocksdb::keys::path_index_key_versioned(T, R, PUBLISH, WS, path, at);
    db.put_cf(cf, key, STRANDED.as_bytes()).unwrap();
}

const STRANDED: &str = "stranded-occupant";

#[tokio::test]
async fn a_promoted_move_leaves_a_stranded_occupants_path_alone_on_origin_and_replicas() {
    let origin = node("origin").await;
    with_publish_branch(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    let a = child("a", &section);
    create(&origin, &a).await;
    promote_with(&origin, false).await;

    // On publish, `/section/a` now belongs to another node while A's record
    // still says it lives there.
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let stranded_at = raisin_hlc::HLC::now();
    plant_stranded(&origin, "/section/a", &stranded_at);
    let split = crate::translation_replication_test::highest_seq(&origin);
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;

    rename_on_main(&origin, "/section/a", "q").await;
    promote_with(&origin, false).await;

    let replicas = replicas_with(&origin, split, |replica| {
        plant_stranded(replica, "/section/a", &stranded_at)
    })
    .await;
    for (name, peer) in every_peer(&origin, &replicas).await {
        assert_eq!(
            path_owner_raw(peer, "/section/a").as_deref(),
            Some(STRANDED),
            "{name}: the mover's stale-path tombstone erased the path's owner"
        );
        assert_eq!(
            path_owner_raw(peer, "/section/q"),
            Some(a.id.clone()),
            "{name}: the mover owns its new path"
        );
    }
}
