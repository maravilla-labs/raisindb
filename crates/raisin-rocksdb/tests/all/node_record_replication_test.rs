//! Phase 10b: replication applies node records through the one record writer.
//!
//! Every replicated node write is a snapshot (`ApplyRevision`, decomposed into
//! `UpsertNodeSnapshot`): it stores the one format — a path-less blob plus a
//! `NODE_PATH` entry at its own revision. The pre-v2 `SetProperty` /
//! `MoveNode` / `CreateNode` handlers, and the path-less "keep the current
//! path" record write only they needed, are gone (plan "Phase 11d"); the
//! saved-oplog side is `legacy_node_op_test`. Records those writers left on
//! disk are data and stay readable by the path read rule.

use crate::node_path_writer_test::{blob_at, node_path_entries, BRANCH, REPO, TENANT, WS};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind};
use raisin_replication::{OpType, Operation, VectorClock};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::{cf, keys, RocksDBConfig, RocksDBStorage};
use raisin_storage::{BranchRepository, NodeRepository, Storage, StorageScope};
use std::sync::Arc;
use tempfile::TempDir;

pub(crate) struct Replica {
    pub storage: Arc<RocksDBStorage>,
    pub applicator: OperationApplicator,
    _dir: TempDir,
}

pub(crate) async fn replica() -> Replica {
    let dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = dir.path().to_path_buf();
    config.replication_enabled = true;
    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await;
    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );
    Replica {
        storage,
        applicator,
        _dir: dir,
    }
}

pub(crate) fn op(rev: HLC, op_type: OpType) -> Operation {
    Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq: rev.timestamp_ms,
        cluster_node_id: "peer".to_string(),
        timestamp_ms: rev.timestamp_ms,
        vector_clock: VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        op_type,
        revision: Some(rev),
        actor: "peer-user".to_string(),
        message: None,
        is_system: false,
        agent: None,
        acknowledged_by: Default::default(),
    }
}

pub(crate) fn folder(id: &str, path: &str, label: &str) -> Node {
    let name = path.rsplit('/').next().unwrap().to_string();
    let parent = match path.rsplitn(2, '/').nth(1) {
        Some(p) if !p.is_empty() => p.rsplit('/').next().map(str::to_string),
        _ => Some("/".to_string()),
    };
    Node {
        id: id.to_string(),
        name,
        path: path.to_string(),
        parent,
        node_type: "raisin:Folder".to_string(),
        order_key: label.to_string(),
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        workspace: Some(WS.to_string()),
        tenant_id: Some(TENANT.to_string()),
        ..Node::default()
    }
}

/// `(node, parent id, label)` upserts at `rev` — what a peer's transaction
/// (a create, or a move with its descendants' snapshots) replicates.
pub(crate) fn revision(rev: HLC, upserts: &[(&Node, &str, &str)]) -> Operation {
    let node_changes = upserts
        .iter()
        .map(|(node, parent, label)| ReplicatedNodeChange {
            node: (*node).clone(),
            parent_id: Some(parent.to_string()),
            kind: ReplicatedNodeChangeKind::Upsert,
            cf_order_key: label.to_string(),
        })
        .collect();
    op(
        rev,
        OpType::ApplyRevision {
            branch_head: rev,
            node_changes,
        },
    )
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

pub(crate) async fn node_at(r: &Replica, id: &str, at: Option<&HLC>) -> Option<Node> {
    r.storage.nodes().get(scope(), id, at).await.unwrap()
}

async fn id_by_path(r: &Replica, path: &str) -> Option<String> {
    r.storage
        .nodes()
        .get_by_path(scope(), path, None)
        .await
        .unwrap()
        .map(|n| n.id)
}

pub(crate) fn title(node: &Node) -> Option<&PropertyValue> {
    node.properties.get("title")
}

pub(crate) async fn apply_all(r: &Replica, ops: &[&Operation]) {
    for op in ops {
        r.applicator.apply_operation(op).await.expect("apply");
    }
}

/// `/a`, `/a/c`, `/x` at r1; then the ancestor move `/a -> /x/a` (with c's
/// snapshot) at r2 — replicated as a transaction move replicates it.
fn tree_and_move(r1: HLC, r2: HLC) -> (Operation, Operation) {
    let a = folder("a", "/a", "a0::a");
    let c = folder("c", "/a/c", "m0::c");
    let x = folder("x", "/x", "x0::x");
    let create = revision(
        r1,
        &[(&a, "/", "a0::a"), (&c, "a", "m0::c"), (&x, "/", "x0::x")],
    );
    let moved_a = folder("a", "/x/a", "a1::a");
    let moved_c = folder("c", "/x/a/c", "m0::c");
    let mv = revision(r2, &[(&moved_a, "x", "a1::a"), (&moved_c, "a", "m0::c")]);
    (create, mv)
}

/// A node only a pre-Phase-10 writer ever wrote — a full blob, NO NODE_PATH
/// entry — is data the read rule keeps readable; a replicated snapshot of it
/// then stores the one format on top.
#[tokio::test]
async fn a_legacy_only_node_stays_readable_and_a_snapshot_stores_the_one_format() {
    let r = replica().await;
    let r1 = HLC::new(1_000, 0);
    let r2 = HLC::new(2_000, 0);
    let db = r.storage.db();
    let legacy = folder("n", "/n", "n0::n");
    db.put_cf(
        db.cf_handle(cf::NODES).unwrap(),
        keys::node_key_versioned(TENANT, REPO, BRANCH, WS, "n", &r1),
        rmp_serde::to_vec_named(&legacy).unwrap(),
    )
    .unwrap();
    db.put_cf(
        db.cf_handle(cf::PATH_INDEX).unwrap(),
        keys::path_index_key_versioned(TENANT, REPO, BRANCH, WS, "/n", &r1),
        b"n",
    )
    .unwrap();
    assert!(node_path_entries(&r.storage, "n").is_empty());
    assert_eq!(
        node_at(&r, "n", Some(&r1)).await.expect("legacy n").path,
        "/n",
        "the read rule reads a legacy-only record"
    );

    let mut titled = legacy.clone();
    titled
        .properties
        .insert("title".to_string(), PropertyValue::String("hello".into()));
    apply_all(&r, &[&revision(r2, &[(&titled, "/", "n0::n")])]).await;

    let n = node_at(&r, "n", None).await.expect("n is readable");
    assert_eq!(n.path, "/n");
    assert_eq!(title(&n), Some(&PropertyValue::String("hello".into())));
    assert_eq!(node_at(&r, "n", Some(&r1)).await.unwrap().path, "/n");
    assert_eq!(id_by_path(&r, "/n").await.as_deref(), Some("n"));
    assert_eq!(
        node_path_entries(&r.storage, "n"),
        vec![(r2, "/n".to_string())]
    );
    let (decoded, _) = raisin_rocksdb::decode_node_blob(&blob_at(&r.storage, "n", &r2)).unwrap();
    assert_eq!(decoded.path, "", "the snapshot's record embeds no path");
}

/// Replicated upserts store the one format: a path-less blob plus NODE_PATH.
#[tokio::test]
async fn replicated_records_are_the_one_format() {
    let r = replica().await;
    let r1 = HLC::new(1_000, 0);
    let (create, _) = tree_and_move(r1, HLC::new(1_500, 0));
    apply_all(&r, &[&create]).await;
    for (id, at, path) in [("a", r1, "/a"), ("c", r1, "/a/c"), ("x", r1, "/x")] {
        let (decoded, parent_id) =
            raisin_rocksdb::decode_node_blob(&blob_at(&r.storage, id, &at)).unwrap();
        assert_eq!(decoded.path, "", "{id}: the blob embeds no path");
        assert_eq!(
            node_path_entries(&r.storage, id),
            vec![(at, path.to_string())]
        );
        assert_eq!(node_at(&r, id, Some(&at)).await.unwrap().path, path);
        if id == "c" {
            assert_eq!(parent_id.as_deref(), Some("a"));
        }
    }
}
