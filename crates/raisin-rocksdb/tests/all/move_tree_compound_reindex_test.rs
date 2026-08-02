//! A move must re-key compound indexes.
//!
//! `__parent_path` makes hierarchy usable as the leading equality column of a
//! compound index, which is what turns `CHILD_OF(p) ORDER BY <col> LIMIT k` into
//! a seek. It also makes a MOVE index-invalidating for the first time: the
//! column's value is derived from the node's path, so relocating a node changes
//! it.
//!
//! Compound indexes are the one index family NOT maintained by the shared
//! `add_node_indexes_to_batch` funnel — that path is synchronous so it stays
//! callable from replication apply, and reading a NodeType's index definitions
//! is async. So every write path must remember them individually, and two have
//! already shipped without doing so (spatial via the repository `add()` path,
//! and compound via the SQL DML path; both are documented in-tree).
//!
//! This test pins the move path so it does not become the third.

use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_models::nodes::{Node, NodeType};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, CompoundColumnValue, CompoundIndexRepository,
    CreateNodeOptions, NodeRepository, NodeTypeRepository, Storage, StorageScope,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t";
const REPO: &str = "r";
const BRANCH: &str = "main";
const WS: &str = "ws";
const NODE_TYPE: &str = "test:Message";
const INDEX: &str = "folder_time";

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

/// A NodeType carrying `(__parent_path, __created_at)` — the folder-listing
/// index shape.
fn message_type() -> NodeType {
    NodeType {
        id: Some(NODE_TYPE.to_string()),
        name: NODE_TYPE.to_string(),
        strict: Some(false),
        allowed_children: vec!["*".to_string()],
        indexable: Some(true),
        created_at: Some(chrono::Utc::now()),
        compound_indexes: Some(vec![CompoundIndexDefinition {
            name: INDEX.to_string(),
            columns: vec![
                CompoundIndexColumn {
                    property: "__parent_path".to_string(),
                    column_type: CompoundColumnType::String,
                    ascending: None,
                },
                CompoundIndexColumn {
                    property: "__created_at".to_string(),
                    column_type: CompoundColumnType::Timestamp,
                    ascending: None,
                },
            ],
            has_order_column: true,
        }]),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(true),
        auditable: Some(false),
        index_types: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        is_mixin: None,
    }
}

fn node(id: &str, path: &str, parent: &str) -> Node {
    Node {
        id: id.to_string(),
        path: path.to_string(),
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        parent: Some(parent.to_string()),
        node_type: NODE_TYPE.to_string(),
        properties: HashMap::new(),
        ..Default::default()
    }
}

async fn setup() -> (Arc<RocksDBStorage>, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let storage = Arc::new(RocksDBStorage::new(tmp.path()).expect("storage"));

    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
        .await;

    storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, REPO, BRANCH),
            message_type(),
            CommitMetadata::system("seed message type"),
        )
        .await
        .expect("upsert node type");

    (storage, tmp)
}

async fn create(storage: &Arc<RocksDBStorage>, n: Node) {
    storage
        .nodes()
        .create(
            scope(),
            n,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("create");
}

/// Node ids the compound index reports for a given parent path.
async fn ids_under(storage: &Arc<RocksDBStorage>, parent_path: &str) -> Vec<String> {
    storage
        .compound_index()
        .scan_compound_index(
            scope(),
            INDEX,
            &[CompoundColumnValue::String(parent_path.to_string())],
            false,
            true,
            None,
        )
        .await
        .expect("scan compound index")
        .into_iter()
        .map(|entry| entry.node_id)
        .collect()
}

#[tokio::test]
async fn moving_a_subtree_rekeys_its_compound_index_entries() {
    let (storage, _tmp) = setup().await;

    // /a and /b are two folders; /a holds two messages.
    create(&storage, node("a", "/a", "/")).await;
    create(&storage, node("b", "/b", "/")).await;
    create(&storage, node("m1", "/a/m1", "a")).await;
    create(&storage, node("m2", "/a/m2", "a")).await;

    let mut before = ids_under(&storage, "/a").await;
    before.sort();
    assert_eq!(
        before,
        vec!["m1".to_string(), "m2".to_string()],
        "both messages should be indexed under their folder before the move"
    );

    // Move one message into /b.
    storage
        .nodes()
        .move_node_tree(scope(), "m1", "/b/m1", None)
        .await
        .expect("move");

    let after_old = ids_under(&storage, "/a").await;
    assert_eq!(
        after_old,
        vec!["m2".to_string()],
        "the moved node must NOT still be indexed under its old parent — a stale \
         entry here means CHILD_OF('/a') keeps returning it forever"
    );

    let after_new = ids_under(&storage, "/b").await;
    assert_eq!(
        after_new,
        vec!["m1".to_string()],
        "the moved node must be indexed under its new parent, or CHILD_OF('/b') \
         will never find it"
    );
}

/// Moving a whole subtree must re-key every descendant, not just the root —
/// each descendant's `__parent_path` changes too.
#[tokio::test]
async fn moving_a_folder_rekeys_every_descendant() {
    let (storage, _tmp) = setup().await;

    create(&storage, node("root", "/root", "/")).await;
    create(&storage, node("dest", "/dest", "/")).await;
    create(&storage, node("f", "/root/f", "root")).await;
    create(&storage, node("c1", "/root/f/c1", "f")).await;
    create(&storage, node("c2", "/root/f/c2", "f")).await;

    let mut before = ids_under(&storage, "/root/f").await;
    before.sort();
    assert_eq!(before, vec!["c1".to_string(), "c2".to_string()]);

    // Move the folder — its children move with it.
    storage
        .nodes()
        .move_node_tree(scope(), "f", "/dest/f", None)
        .await
        .expect("move folder");

    assert!(
        ids_under(&storage, "/root/f").await.is_empty(),
        "no descendant may remain indexed under the old folder path"
    );

    let mut after = ids_under(&storage, "/dest/f").await;
    after.sort();
    assert_eq!(
        after,
        vec!["c1".to_string(), "c2".to_string()],
        "every descendant must be re-keyed to the new folder path"
    );
}
