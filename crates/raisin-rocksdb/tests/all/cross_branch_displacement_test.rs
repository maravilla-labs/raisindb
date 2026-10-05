//! A promotion that DELETES target nodes — prunes them (`delete_missing`) or
//! displaces them from a path the source re-created under a fresh id
//! (`deploy --install`) — must leave the same state on the origin and on every
//! replica, whether the replica applies the `ApplyRevision` as stored or
//! decomposed as the push path sends it.
//!
//! Each failure this guards was silent: a path that resolved to nothing on
//! the origin (the prune's tombstone written over the replacement's mapping),
//! an old generation still live on replicas only (the displacement was never
//! replicated), a child still listed under its parent on decomposed replicas
//! only (the delete's parent resolved by path at HEAD), and index families
//! live on the origin only (a hand-mirrored prune tombstoner).

use crate::cross_branch_prune_replication_test::{
    create, folder, on_publish, promote_with, with_publish_branch, PUBLISH,
};
use crate::delete_order_tombstone_test::live_children_in;
use crate::translation_replication_test::{node, receive, Node as Peer};
use crate::translation_substrate_test::{B, R, T, WS};
use raisin_models::nodes::Node;
use raisin_replication::operation::ReplicatedNodeChangeKind;
use raisin_replication::{decompose_operation, OpType, Operation};
use raisin_rocksdb::{cf, keys, OpLogRepository};
use raisin_storage::{DeleteNodeOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashSet;

pub(crate) fn child(name: &str, parent: &Node) -> Node {
    let mut node = folder(name, &format!("{}/{name}", parent.path));
    node.parent = Some(parent.name.clone());
    node
}

pub(crate) async fn delete_on_main(peer: &Peer, id: &str) {
    let deleted = peer
        .storage
        .nodes()
        .delete(
            StorageScope::new(T, R, B, WS),
            id,
            DeleteNodeOptions::default(),
        )
        .await
        .expect("delete");
    assert!(deleted);
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
}

pub(crate) async fn by_path(peer: &Peer, path: &str) -> Option<String> {
    peer.storage
        .nodes()
        .get_by_path(StorageScope::new(T, R, PUBLISH, WS), path, None)
        .await
        .unwrap()
        .map(|n| n.id)
}

/// The node id the newest PATH_INDEX entry for `path` names on publish, read
/// RAW (`None` when that entry is a tombstone): `get_by_path` can fall back
/// past a tombstoned entry, which would hide an erased mapping.
pub(crate) fn path_owner_raw(peer: &Peer, path: &str) -> Option<String> {
    let db = peer.storage.db();
    let prefix = keys::path_index_key_prefix(T, R, PUBLISH, WS, path);
    let cf = db.cf_handle(cf::PATH_INDEX).unwrap();
    let (key, value) = db.prefix_iterator_cf(cf, &prefix).next()?.unwrap();
    assert!(key.starts_with(&prefix));
    (!keys::is_tombstone_value(&value)).then(|| String::from_utf8(value.to_vec()).unwrap())
}

pub(crate) fn children(peer: &Peer, parent_id: &str) -> HashSet<String> {
    live_children_in(&peer.storage, (T, R, PUBLISH, WS), parent_id, None)
}

pub(crate) fn set(ids: &[&str]) -> HashSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

/// Every op the origin captured, in capture order.
pub(crate) fn origin_ops(origin: &Peer) -> Vec<Operation> {
    let mut ops = OpLogRepository::new(origin.storage.db().clone())
        .get_operations_from_node(T, R, &origin.id)
        .unwrap();
    ops.sort_by_key(|op| op.op_seq);
    ops
}

/// Replicas fed the origin's oplog as stored, decomposed as the push path
/// sends it, and decomposed with every `DeleteNodeSnapshot` of a revision
/// moved AFTER its upserts — the order an `ApplyRevision` captured before
/// deletes went first still holds, in which a delete erased its
/// replacement's path.
pub(crate) async fn replicas(origin: &Peer) -> Vec<(&'static str, Peer)> {
    replicas_with(origin, u64::MAX, |_| {}).await
}

/// [`replicas`], each fed the ops up to `split` (an op seq), then handed to
/// `between` (drop its definitions cache, plant a stranded entry), then fed
/// the rest.
pub(crate) async fn replicas_with(
    origin: &Peer,
    split: u64,
    between: impl Fn(&Peer),
) -> Vec<(&'static str, Peer)> {
    let ops = origin_ops(origin);
    let decomposed: Vec<Operation> = ops.iter().cloned().flat_map(decompose_operation).collect();
    let mut deletes_last = decomposed.clone();
    deletes_last.sort_by_key(|op| {
        let is_delete = matches!(op.op_type, OpType::DeleteNodeSnapshot { .. });
        (op.op_seq, is_delete)
    });
    let mut out = Vec::new();
    for (name, batch) in [
        ("replica-raw", ops),
        ("replica-decomposed", decomposed),
        ("replica-deletes-last", deletes_last),
    ] {
        let replica = node(name).await;
        with_publish_branch(&replica).await;
        let (before, after): (Vec<Operation>, Vec<Operation>) =
            batch.into_iter().partition(|op| op.op_seq <= split);
        receive(&replica, &before).await;
        between(&replica);
        receive(&replica, &after).await;
        out.push((name, replica));
    }
    out
}

/// The origin and every replica.
pub(crate) async fn every_peer<'a>(
    origin: &'a Peer,
    replicas: &'a [(&'static str, Peer)],
) -> Vec<(&'static str, &'a Peer)> {
    std::iter::once(("origin", origin))
        .chain(replicas.iter().map(|(name, peer)| (*name, peer)))
        .collect()
}

#[tokio::test]
async fn a_prune_does_not_erase_the_path_of_the_node_that_displaced_its_target() {
    let origin = node("origin").await;
    with_publish_branch(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    let old = child("page", &section);
    create(&origin, &old).await;
    promote_with(&origin, true).await;

    // Re-created on main under a fresh id; the root keeps its id.
    delete_on_main(&origin, &old.id).await;
    let new = child("page", &section);
    create(&origin, &new).await;
    promote_with(&origin, true).await;

    // The old generation travels as ONE Delete, ahead of the Upsert that
    // takes its path.
    let changes = origin_ops(&origin)
        .into_iter()
        .filter(|op| op.branch == PUBLISH)
        .filter_map(|op| match op.op_type {
            OpType::ApplyRevision { node_changes, .. } => Some(node_changes),
            _ => None,
        })
        .last()
        .expect("the promotion's ApplyRevision");
    let at = |id: &str, kind| {
        let found: Vec<usize> = (0..changes.len())
            .filter(|i| changes[*i].node.id == id && changes[*i].kind == kind)
            .collect();
        assert_eq!(found.len(), 1, "{id}: exactly one {kind:?} change");
        found[0]
    };
    assert!(
        at(&old.id, ReplicatedNodeChangeKind::Delete)
            < at(&new.id, ReplicatedNodeChangeKind::Upsert)
    );

    let replicas = replicas(&origin).await;
    for (name, peer) in every_peer(&origin, &replicas).await {
        assert_eq!(
            by_path(peer, "/section/page").await.as_deref(),
            Some(new.id.as_str()),
            "{name}: the replacement's path resolves"
        );
        assert_eq!(
            path_owner_raw(peer, "/section/page"),
            Some(new.id.clone()),
            "{name}: PATH_INDEX names the replacement"
        );
        assert!(
            on_publish(peer, &old.id).await.is_none(),
            "{name}: old gone"
        );
        assert_eq!(children(peer, &section.id), set(&[&new.id]), "{name}");
    }
}

#[tokio::test]
async fn a_displaced_occupant_is_deleted_on_replicas_too() {
    let origin = node("origin").await;
    with_publish_branch(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    let page = child("page", &section);
    create(&origin, &page).await;
    promote_with(&origin, false).await;

    // Root AND child re-created under fresh ids; no delete_missing.
    delete_on_main(&origin, &section.id).await;
    let section2 = folder("section", "/section");
    create(&origin, &section2).await;
    let page2 = child("page", &section2);
    create(&origin, &page2).await;
    promote_with(&origin, false).await;

    let replicas = replicas(&origin).await;
    for (name, peer) in every_peer(&origin, &replicas).await {
        assert_eq!(by_path(peer, "/section").await, Some(section2.id.clone()));
        assert_eq!(by_path(peer, "/section/page").await, Some(page2.id.clone()));
        assert_eq!(
            path_owner_raw(peer, "/section"),
            Some(section2.id.clone()),
            "{name}"
        );
        assert_eq!(
            path_owner_raw(peer, "/section/page"),
            Some(page2.id.clone()),
            "{name}"
        );
        for old in [&section, &page] {
            assert!(
                on_publish(peer, &old.id).await.is_none(),
                "{name}: displaced {} still live",
                old.path
            );
        }
        let roots = children(peer, "/");
        assert!(roots.contains(&section2.id), "{name}");
        assert!(
            !roots.contains(&section.id),
            "{name}: old root still listed"
        );
        assert_eq!(children(peer, &section.id), set(&[]), "{name}");
        assert_eq!(children(peer, &section2.id), set(&[&page2.id]), "{name}");
    }
}

#[tokio::test]
async fn a_recreated_roots_previous_subtree_is_pruned_with_delete_missing() {
    let origin = node("origin").await;
    with_publish_branch(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    let extra = child("extra", &section);
    create(&origin, &extra).await;
    promote_with(&origin, true).await;

    // The root is re-created; `extra` is not part of the new tree.
    delete_on_main(&origin, &section.id).await;
    let section2 = folder("section", "/section");
    create(&origin, &section2).await;
    promote_with(&origin, true).await;

    let replicas = replicas(&origin).await;
    for (name, peer) in every_peer(&origin, &replicas).await {
        assert_eq!(by_path(peer, "/section").await, Some(section2.id.clone()));
        assert_eq!(
            path_owner_raw(peer, "/section"),
            Some(section2.id.clone()),
            "{name}"
        );
        assert!(on_publish(peer, &section.id).await.is_none(), "{name}");
        assert!(
            on_publish(peer, &extra.id).await.is_none(),
            "{name}: the old root's child is orphaned, not pruned"
        );
        assert_eq!(children(peer, &section.id), set(&[]), "{name}");
    }
}

#[tokio::test]
async fn a_pruned_childs_order_entry_is_ended_on_decomposed_replicas() {
    let origin = node("origin").await;
    with_publish_branch(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    // Renamed parent: P keeps its id, moves /section/a -> /section/b.
    let renamed = child("a", &section);
    create(&origin, &renamed).await;
    let x = child("x", &renamed);
    create(&origin, &x).await;
    // Nested prune: `old` and its child both go.
    let old = child("old", &section);
    create(&origin, &old).await;
    let y = child("y", &old);
    create(&origin, &y).await;
    promote_with(&origin, true).await;

    origin
        .storage
        .nodes()
        .rename_node(StorageScope::new(T, R, B, WS), "/section/a", "b")
        .await
        .expect("rename");
    delete_on_main(&origin, &x.id).await;
    delete_on_main(&origin, &old.id).await;
    promote_with(&origin, true).await;

    let replicas = replicas(&origin).await;
    for (name, peer) in every_peer(&origin, &replicas).await {
        assert_eq!(by_path(peer, "/section/b").await, Some(renamed.id.clone()));
        for gone in [&x, &old, &y] {
            assert!(on_publish(peer, &gone.id).await.is_none(), "{name}");
        }
        assert_eq!(
            children(peer, &renamed.id),
            set(&[]),
            "{name}: the pruned child is still listed under its renamed parent"
        );
        assert_eq!(
            children(peer, &old.id),
            set(&[]),
            "{name}: the pruned child is still listed under its pruned parent"
        );
    }
}

#[tokio::test]
async fn a_pruned_node_gets_the_full_delete_tombstone_set() {
    let origin = node("origin").await;
    with_publish_branch(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    let page = child("page", &section);
    create(&origin, &page).await;
    promote_with(&origin, true).await;
    delete_on_main(&origin, &page.id).await;
    promote_with(&origin, true).await;

    // NODE_PATH is one of the families the hand-mirrored prune never wrote
    // (with COMPOUND, SPATIAL, SECRETS and the vmount registry); the shared
    // tombstoner the replicas use always does.
    let replicas = replicas(&origin).await;
    for (name, peer) in every_peer(&origin, &replicas).await {
        let db = peer.storage.db();
        let prefix = keys::node_path_key_prefix(T, R, PUBLISH, WS, &page.id);
        let cf = db.cf_handle(cf::NODE_PATH).unwrap();
        let (key, value) = db
            .prefix_iterator_cf(cf, &prefix)
            .next()
            .expect("a NODE_PATH entry")
            .unwrap();
        assert!(key.starts_with(&prefix));
        assert!(
            keys::is_tombstone_value(&value),
            "{name}: the pruned node's NODE_PATH entry is still live"
        );
    }
}
