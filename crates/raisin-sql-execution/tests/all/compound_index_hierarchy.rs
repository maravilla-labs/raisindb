//! Hierarchy as a compound-index column, end to end.
//!
//! A folder listing is normally written WITHOUT naming a node type:
//!
//!     SELECT ... WHERE CHILD_OF('/a') ORDER BY created_at DESC LIMIT 10
//!
//! Three things had to be true for that to use an index:
//!
//!  1. `CHILD_OF` had to be expressible as a compound-index column, so hierarchy
//!     could lead a sorted index and leave the trailing column free for the
//!     ORDER BY. That is `__parent_path`.
//!  2. The index had to be FOUND without a literal `node_type =`.
//!  3. The index had to hold EVERY child. A NodeType-owned index holds only its
//!     type's nodes, and the planner's ownership gate rightly refuses it for an
//!     untyped query (a silent subset otherwise). A WORKSPACE-owned index (plan
//!     Phase 13e) covers every node of its workspace, so it serves the bare
//!     listing — that is what these tests declare.

use futures::StreamExt;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_models::nodes::{Node, NodeType};
use raisin_models::workspace::Workspace;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, CreateNodeOptions, NodeRepository,
    NodeTypeRepository, RepoScope, Storage, StorageScope, WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

pub(super) const TENANT: &str = "t_cih";
pub(super) const BRANCH: &str = "main";
pub(super) const WS: &str = "ws";
pub(super) const NODE_TYPE: &str = "test:Message";
pub(super) const NOTE_TYPE: &str = "test:Note";

/// Who declares the `(__parent_path, __created_at)` index in a setup.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Owner {
    /// `test:Message` declares it (type-owned).
    NodeType,
    /// The workspace declares it (plan Phase 13e).
    Workspace,
    /// Nobody declares it: the workspace's BUILT-IN
    /// `@__children_by_created_at` (plan Phase 13f) is the only folder index.
    Builtin,
}

/// `(__parent_path, __created_at DESC)` — authored name `folder_time`.
pub(super) fn folder_time() -> CompoundIndexDefinition {
    CompoundIndexDefinition {
        name: "folder_time".to_string(),
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
        owner: None,
    }
}

/// `test:Message` declaring the type-owned `folder_time`.
pub(super) fn message_type() -> NodeType {
    typed(NODE_TYPE, Some(vec![folder_time()]))
}

/// A plain type `name` with `compound` declarations.
pub(super) fn typed(name: &str, compound: Option<Vec<CompoundIndexDefinition>>) -> NodeType {
    NodeType {
        id: Some(name.to_string()),
        name: name.to_string(),
        strict: Some(false),
        allowed_children: vec!["*".to_string()],
        indexable: Some(true),
        created_at: Some(chrono::Utc::now()),
        compound_indexes: compound,
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
        immutable: None,
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

pub(super) fn node(id: &str, path: &str, parent: &str) -> Node {
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

/// The engine over a storage for `repo` (a repo name per test: the planner's
/// definition caches are process-wide and keyed by tenant/repo/branch, so two
/// tests declaring differently must not share one).
pub(super) type Engine = QueryEngine<raisin_rocksdb::RocksDBStorage>;

/// A repository where `owner` declares `folder_time`, with `/a` and three
/// `test:Message` children; built when `build` (the production build path).
/// The built-in folder index (plan Phase 13f) is switched OFF unless `owner`
/// is [`Owner::Builtin`], so these tests see the declared index alone.
pub(super) async fn setup(
    repo: &str,
    owner: Owner,
    build: bool,
) -> (Engine, Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    setup_with(repo, owner, build, owner == Owner::Builtin).await
}

/// [`setup`] with the built-in folder index on or off (`builtin`).
pub(super) async fn setup_with(
    repo: &str,
    owner: Owner,
    build: bool,
    builtin: bool,
) -> (Engine, Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(tmp.path()).expect("storage"));

    let _ = storage
        .branches()
        .create_branch(TENANT, repo, BRANCH, "test", None, None, false, false)
        .await;

    let type_index = (owner == Owner::NodeType).then(|| vec![folder_time()]);
    for ty in [typed(NODE_TYPE, type_index), typed(NOTE_TYPE, None)] {
        storage
            .node_types()
            .upsert(
                BranchScope::new(TENANT, repo, BRANCH),
                ty,
                CommitMetadata::system("seed"),
            )
            .await
            .expect("upsert node type");
    }
    let mut workspace = Workspace::new(WS.to_string());
    if owner == Owner::Workspace {
        workspace.compound_indexes = Some(vec![folder_time()]);
    }
    if !builtin {
        workspace.config.builtin_indexes = Some(raisin_models::workspace::BuiltinIndexes {
            children_by_created_at: false,
        });
    }
    storage
        .workspaces()
        .put(RepoScope::new(TENANT, repo), workspace)
        .await
        .expect("workspace");

    for (id, path, parent) in [
        ("a", "/a", "/"),
        ("m0", "/a/m0", "a"),
        ("m1", "/a/m1", "a"),
        ("m2", "/a/m2", "a"),
    ] {
        storage
            .nodes()
            .create(
                StorageScope::new(TENANT, repo, BRANCH, WS),
                node(id, path, parent),
                CreateNodeOptions {
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .expect("create");
    }

    // BUILD THE INDEX, the way production does: a declaration is not a built
    // index, and the planner declines anything not `Ready`.
    // `rebuild_indexes(.., IndexType::Compound)` is the real (synchronous)
    // build path; it builds the workspace's own indexes too.
    if build {
        build_compound(&storage, repo).await;
    }

    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    let engine = QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        repo.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog));

    (engine, storage, tmp)
}

pub(super) async fn build_compound(storage: &raisin_rocksdb::RocksDBStorage, repo: &str) {
    raisin_rocksdb::management::async_indexing::rebuild_indexes(
        storage,
        TENANT,
        repo,
        BRANCH,
        WS,
        raisin_storage::IndexType::Compound,
    )
    .await
    .expect("build the compound index");
}

pub(super) async fn explain(engine: &Engine, sql: &str) -> String {
    let mut stream = engine.execute(sql).await.expect("explain");
    let row = stream.next().await.expect("row").expect("decode");
    match row.columns.get("QUERY PLAN") {
        Some(raisin_models::nodes::properties::PropertyValue::String(p)) => p.clone(),
        other => panic!("unexpected EXPLAIN output: {other:?}"),
    }
}

/// The `column` of every row `sql` returns, in order.
pub(super) async fn strings(engine: &Engine, sql: &str, column: &str) -> Vec<String> {
    let mut stream = engine.execute(sql).await.expect("query");
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.expect("decode");
        if let Some(raisin_models::nodes::properties::PropertyValue::String(v)) = row
            .columns
            .iter()
            .find(|(k, _)| k.rsplit('.').next() == Some(column))
            .map(|(_, v)| v.clone())
        {
            out.push(v);
        }
    }
    out
}

pub(super) const BARE: &str = "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a') \
                    ORDER BY created_at DESC LIMIT 3";
pub(super) const TYPED: &str = "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a') \
                     AND node_type = 'test:Message' ORDER BY created_at DESC LIMIT 3";

/// The whole point: a bare folder listing, ordered, bounded — no node type
/// named — served by the WORKSPACE's index, with the sort elided.
#[tokio::test]
async fn bare_child_of_order_by_uses_the_compound_index_and_elides_the_sort() {
    let (engine, _storage, _tmp) = setup("r_cih_ws", Owner::Workspace, true).await;

    let plan = explain(&engine, BARE).await;
    assert!(
        plan.contains("CompoundIndexScan") && plan.contains("@folder_time"),
        "a bare CHILD_OF + ORDER BY must use the workspace's \
         (__parent_path, __created_at) index; plan:\n{plan}"
    );
    assert!(
        plan.contains("owner: workspace ws"),
        "EXPLAIN names the owner; plan:\n{plan}"
    );
    assert!(
        !plan.contains("Sort") && !plan.contains("TopN"),
        "and the trailing order column must elide the sort; plan:\n{plan}"
    );
    // A typed listing on the same workspace may use it too.
    assert!(explain(&engine, TYPED).await.contains("@folder_time"));
}

/// A NodeType-owned index holds only that type's nodes: an untyped listing
/// must NOT be served from it (a silent subset), a typed one still is.
#[tokio::test]
async fn a_type_owned_index_is_refused_for_an_untyped_listing() {
    let (engine, _storage, _tmp) = setup("r_cih_type_refused", Owner::NodeType, true).await;

    let bare = explain(&engine, BARE).await;
    assert!(
        !bare.contains("CompoundIndexScan"),
        "an untyped listing must not use a type-owned index; plan:\n{bare}"
    );
    let typed = explain(&engine, TYPED).await;
    assert!(
        typed.contains("CompoundIndexScan") && typed.contains("owner: node type test:Message"),
        "the typed form must still match its type's index; plan:\n{typed}"
    );
}

/// Naming the node type must keep working with a type-owned index — that path
/// loads only that type's definitions rather than the whole branch.
#[tokio::test]
async fn naming_the_node_type_still_finds_the_index() {
    let (engine, _storage, _tmp) = setup("r_cih_type_named", Owner::NodeType, true).await;
    assert!(explain(&engine, TYPED).await.contains("CompoundIndexScan"));
}

/// Declared but not built: fail closed, the listing scans.
#[tokio::test]
async fn an_unbuilt_workspace_index_is_not_used() {
    let (engine, storage, _tmp) = setup("r_cih_unbuilt", Owner::Workspace, false).await;
    assert!(!explain(&engine, BARE).await.contains("CompoundIndexScan"));
    build_compound(&storage, "r_cih_unbuilt").await;
    assert!(explain(&engine, BARE).await.contains("CompoundIndexScan"));
}

/// Results must be correct, not merely indexed: the rows come back and the
/// LIMIT is respected.
#[tokio::test]
async fn the_indexed_listing_returns_the_right_rows() {
    let (engine, _storage, _tmp) = setup("r_cih_rows", Owner::Workspace, true).await;

    let names = strings(
        &engine,
        "SELECT name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at DESC LIMIT 2",
        "name",
    )
    .await;
    assert_eq!(
        names.len(),
        2,
        "LIMIT 2 must return two rows; got {names:?}"
    );
    assert!(
        names.iter().all(|n| n.starts_with('m')),
        "only the folder's children may be returned; got {names:?}"
    );
}

/// A workspace index matched on `__parent_path` alone holds exactly what the
/// CHILD_OF scan reads, so it is used only when it serves the ORDER BY: an
/// editorial listing keeps the child scan and its sort elision, and a listing
/// with no ORDER BY keeps the order it always came back in.
#[tokio::test]
async fn an_editorial_listing_keeps_the_child_scan() {
    let (engine, _storage, _tmp) = setup("r_cih_editorial", Owner::Workspace, true).await;
    for sql in [
        "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY __order",
        "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a')",
    ] {
        let plan = explain(&engine, sql).await;
        assert!(
            !plan.contains("CompoundIndexScan") && !plan.contains("Sort"),
            "{sql}:\n{plan}"
        );
    }
}

/// The declaration surface over SQL: `UPDATE Workspaces SET compound_indexes`
/// declares the workspace's own index (no node type involved), and once it is
/// built the bare listing is index-served.
#[tokio::test]
async fn a_workspace_index_can_be_declared_over_sql() {
    let repo = "r_cih_sql_decl";
    let (engine, storage, _tmp) = setup(repo, Owner::NodeType, true).await;
    assert!(!explain(&engine, BARE).await.contains("CompoundIndexScan"));
    let declaration = serde_json::to_string(&vec![folder_time()]).unwrap();
    let mut done = engine
        .execute(&format!(
            "UPDATE Workspaces SET compound_indexes = '{declaration}'::jsonb WHERE name = '{WS}'"
        ))
        .await
        .expect("declare over SQL");
    while let Some(row) = done.next().await {
        row.expect("update row");
    }
    let stored = storage
        .workspaces()
        .get(RepoScope::new(TENANT, repo), WS)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.owned_compound_indexes()[0].name, "@folder_time");
    build_compound(&storage, repo).await;
    raisin_sql_execution::invalidate_compound_index_cache();
    assert!(explain(&engine, BARE).await.contains("@folder_time"));
}
