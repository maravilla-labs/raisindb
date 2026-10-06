//! `NODE_DELETES` is complete: every path that writes a node delete
//! tombstone writes its entry — the transaction, the repository (plain and
//! cascading), the replication apply on a replica (a derived index, written
//! locally for a REMOTE delete), a merge (resolved deletions and replayed
//! source history), a fork's copy — and a tenant wipe takes it away.
//!
//! Completeness is checked against `NODES` itself: every tombstone stored on
//! the branch must have its entry ([`assert_complete`]).

use crate::merge_apply_funnel_test::{node as page, Env, REPO as MREPO, TENANT as MTENANT};
use crate::translation_delete_convergence_test::{create, delete, head, ops_since};
use crate::translation_replication_test::{highest_seq, node, receive};
use crate::translation_substrate_test::{B, R, T, WS};
use raisin_context::{MergeStrategy, ResolutionType};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node as Content;
use raisin_rocksdb::{cf, fractional_index, node_delete_index, RocksDBStorage};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, NodeRepository, Storage, StorageScope,
};
use std::collections::HashMap;

/// Every `NODES` tombstone of the branch, as `(workspace, node_id, revision)`.
pub(crate) fn stored_tombstones(
    storage: &RocksDBStorage,
    tenant: &str,
    repo: &str,
    branch: &str,
) -> Vec<(String, String, HLC)> {
    let db = storage.db();
    let prefix = format!("{tenant}\0{repo}\0{branch}\0").into_bytes();
    let mut out = Vec::new();
    for item in db.prefix_iterator_cf(db.cf_handle(cf::NODES).unwrap(), &prefix) {
        let (key, value) = item.unwrap();
        if !key.starts_with(&prefix) {
            break;
        }
        if value.as_ref() != b"T" || key.len() < prefix.len() + 17 {
            continue;
        }
        let (head, rev) = key[prefix.len()..].split_at(key.len() - prefix.len() - 16);
        let parts: Vec<&[u8]> = head.split(|b| *b == 0).collect();
        if let [ws, b"nodes", id, b""] = parts.as_slice() {
            out.push((
                String::from_utf8_lossy(ws).into_owned(),
                String::from_utf8_lossy(id).into_owned(),
                HLC::decode_descending(rev).unwrap(),
            ));
        }
    }
    out
}

/// Every tombstone of the branch has its entry. Returns how many there are.
pub(crate) fn assert_complete(
    storage: &RocksDBStorage,
    tenant: &str,
    repo: &str,
    branch: &str,
) -> usize {
    let tombstones = stored_tombstones(storage, tenant, repo, branch);
    for (ws, id, at) in &tombstones {
        let recorded =
            node_delete_index::recorded_deletes(storage.db(), (tenant, repo, branch, ws), id)
                .unwrap();
        assert!(
            recorded.contains(at),
            "{branch}/{ws}/{id}: tombstone at {at} has no entry (entries: {recorded:?})"
        );
    }
    tombstones.len()
}

fn recorded(storage: &RocksDBStorage, branch: &str, id: &str) -> Vec<HLC> {
    node_delete_index::recorded_deletes(storage.db(), (T, R, branch, WS), id).unwrap()
}

/// A node at `path` under the parent named `parent`.
async fn create_at(
    n: &crate::translation_replication_test::Node,
    id: &str,
    path: &str,
    parent: &str,
) {
    let content = Content {
        id: id.to_string(),
        name: path.rsplit('/').next().unwrap().to_string(),
        path: path.to_string(),
        node_type: "raisin:Folder".to_string(),
        properties: HashMap::from([("title".to_string(), PropertyValue::String(id.into()))]),
        order_key: fractional_index::first(),
        parent: Some(parent.to_string()),
        created_at: Some(chrono::Utc::now()),
        ..Content::default()
    };
    // Through the repository, which can skip the parent's NodeType check.
    let options = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };
    n.storage
        .nodes()
        .create(StorageScope::new(T, R, B, WS), content, options)
        .await
        .unwrap();
}

#[tokio::test]
async fn every_local_delete_path_writes_the_entry() {
    let a = node("a").await;
    let scope = StorageScope::new(T, R, B, WS);

    // Transaction delete.
    create(&a, "tx-deleted").await;
    delete(&a, "tx-deleted").await;
    assert_eq!(recorded(&a.storage, B, "tx-deleted"), vec![head(&a).await]);

    // Repository delete.
    create(&a, "repo-deleted").await;
    a.storage
        .nodes()
        .delete(scope, "repo-deleted", DeleteNodeOptions::default())
        .await
        .unwrap();
    assert_eq!(recorded(&a.storage, B, "repo-deleted").len(), 1);

    // Cascade: the parent and every descendant.
    create_at(&a, "parent", "/parent", "/").await;
    create_at(&a, "child", "/parent/child", "parent").await;
    create_at(&a, "grandchild", "/parent/child/gc", "child").await;
    let cascade = DeleteNodeOptions {
        cascade: true,
        check_has_children: false,
        operation_meta: None,
    };
    a.storage
        .nodes()
        .delete(scope, "parent", cascade)
        .await
        .unwrap();
    for id in ["parent", "child", "grandchild"] {
        assert_eq!(recorded(&a.storage, B, id).len(), 1, "{id}");
    }

    // Deleted, re-created, deleted again: two entries, newest first.
    create(&a, "twice").await;
    delete(&a, "twice").await;
    let first = head(&a).await;
    create(&a, "twice").await;
    delete(&a, "twice").await;
    let second = head(&a).await;
    assert_eq!(recorded(&a.storage, B, "twice"), vec![second, first]);

    assert!(assert_complete(&a.storage, T, R, B) >= 7);
}

#[tokio::test]
async fn a_replica_writes_the_entry_for_a_remote_delete() {
    let a = node("a").await;
    let b = node("b").await;
    let setup = highest_seq(&a);
    create(&a, "page").await;
    create(&a, "kept").await;
    delete(&a, "page").await;
    let deleted_at = head(&a).await;
    receive(&b, &ops_since(&a, setup)).await;

    assert_eq!(recorded(&b.storage, B, "page"), vec![deleted_at]);
    assert!(recorded(&b.storage, B, "kept").is_empty());
    assert_eq!(assert_complete(&b.storage, T, R, B), 1);
}

#[tokio::test]
async fn a_merge_writes_the_entries_of_the_deletes_it_applies() {
    // A resolved deletion (a Manual resolution with no properties deletes).
    let env = Env::new().await.unwrap();
    env.add("main", page("c", "/c", &[("title", "base")]))
        .await
        .unwrap();
    env.fork().await.unwrap();
    env.put("main", page("c", "/c", &[("title", "ours")]))
        .await
        .unwrap();
    env.put("feature", page("c", "/c", &[("title", "theirs")]))
        .await
        .unwrap();
    env.conflict_and_resolve("c", ResolutionType::Manual)
        .await
        .unwrap();
    assert_eq!(env.id_at("/c").await, None);
    assert!(assert_complete(&env.storage, MTENANT, MREPO, "main") >= 1);

    // A delete made on the source, merged without conflict.
    let env = Env::new().await.unwrap();
    env.add("main", page("d", "/d", &[("title", "base")]))
        .await
        .unwrap();
    env.fork().await.unwrap();
    env.delete("feature", "d").await.unwrap();
    env.storage
        .branches_impl()
        .merge_branches(
            MTENANT,
            MREPO,
            "main",
            "feature",
            MergeStrategy::ThreeWay,
            "merge",
            "test-user",
        )
        .await
        .unwrap();
    assert_eq!(env.id_at("/d").await, None, "the merge applied the delete");
    assert!(assert_complete(&env.storage, MTENANT, MREPO, "main") >= 1);
    assert!(assert_complete(&env.storage, MTENANT, MREPO, "feature") >= 1);
}

#[tokio::test]
async fn a_fork_copies_the_entries_and_reads_by_the_walk_until_backfilled() {
    let a = node("a").await;
    create(&a, "gone").await;
    delete(&a, "gone").await;
    crate::node_delete_index_backfill_test::backfill(&a, B).await;
    assert!(node_delete_index::is_ready(a.storage.db(), T, R, B));

    a.storage
        .branches()
        .create_branch(
            T,
            R,
            "feature",
            "test",
            None,
            Some(B.to_string()),
            false,
            false,
        )
        .await
        .unwrap();
    assert_eq!(recorded(&a.storage, "feature", "gone").len(), 1, "copied");
    assert_eq!(assert_complete(&a.storage, T, R, "feature"), 1);
    assert!(
        !node_delete_index::is_ready(a.storage.db(), T, R, "feature"),
        "a fork's readiness is its own"
    );
    let report = crate::node_delete_index_backfill_test::backfill(&a, "feature").await;
    assert_eq!(
        report.node_deletes.written, 0,
        "the copy brought every entry"
    );
    assert!(node_delete_index::is_ready(a.storage.db(), T, R, "feature"));

    // Deleting the branch forgets its readiness: a branch re-created under
    // the name starts over.
    a.storage
        .branches()
        .delete_branch(T, R, "feature")
        .await
        .unwrap();
    assert!(!node_delete_index::is_ready(
        a.storage.db(),
        T,
        R,
        "feature"
    ));
}

#[tokio::test]
async fn a_tenant_wipe_removes_the_index() {
    let a = node("a").await;
    create(&a, "gone").await;
    delete(&a, "gone").await;
    crate::node_delete_index_backfill_test::backfill(&a, B).await;
    assert_eq!(recorded(&a.storage, B, "gone").len(), 1);
    assert!(node_delete_index::is_ready(a.storage.db(), T, R, B));

    a.storage.delete_tenant_data(T).unwrap();
    assert!(recorded(&a.storage, B, "gone").is_empty());
    assert!(!node_delete_index::is_ready(a.storage.db(), T, R, B));
    let db = a.storage.db();
    let mut iter = db.raw_iterator_cf(db.cf_handle(cf::NODE_DELETES).unwrap());
    iter.seek(T.as_bytes());
    assert!(
        !iter.valid() || !iter.key().unwrap().starts_with(T.as_bytes()),
        "no entry of the wiped tenant is left"
    );
}
