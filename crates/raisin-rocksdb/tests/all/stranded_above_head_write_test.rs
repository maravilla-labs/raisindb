//! Regression tests for nodes STRANDED ABOVE the branch HEAD.
//!
//! # The bug
//!
//! Within a single write, the "does this exist?" probe and the "is this id /
//! path taken?" uniqueness check used DIFFERENT visibility rules:
//!
//! * the probe (`transaction::context::nodes::read::get_node` /
//!   `get_node_by_path`) is HEAD-BOUNDED — it skips every revision newer than
//!   `branch.head`;
//! * the uniqueness check it routes into
//!   (`NodeRepositoryImpl::validate_for_create` -> `get_impl` /
//!   `get_by_path_impl`) resolves the LATEST revision, UNBOUNDED by HEAD.
//!
//! So a node whose latest revision sits ABOVE the branch HEAD is invisible to
//! the probe (which therefore routes to CREATE) but still rejects that CREATE
//! with `Conflict`. Such a node is PERMANENTLY unwritable: the only write that
//! would advance HEAD past it is the write that keeps failing.
//!
//! Nodes end up above HEAD through a branch-HEAD regression — see
//! `branch_head_monotonic_test`, whose monotonicity guard (added in v0.1.60)
//! stops NEW damage but cannot un-strand the pre-existing residue. These tests
//! reproduce that state deterministically with `BranchRepository::set_head`,
//! which is the deliberate-rollback API and the only remaining way to move a
//! HEAD backwards.
//!
//! # The invariant these tests pin
//!
//! For a given (branch, workspace), an id maps to at most one node and a path
//! to at most one live node, over the branch's ENTIRE revision history — not
//! just its HEAD-visible prefix. Conflict detection stays UNBOUNDED (it is not
//! weakened here); what is fixed is the CREATE-vs-UPDATE *routing*, which now
//! consults the same unbounded world. A node above HEAD is the SAME node, so
//! the write is an UPDATE — and that update's fresh revision advances HEAD past
//! the stranded revision, which is the self-heal.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::fractional_index;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeTypeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "stranded-test";
const REPO: &str = "main-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

fn build_node(path: &str, title: &str) -> Node {
    let name = path.trim_start_matches('/').to_string();
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String(title.to_string()),
    );

    Node {
        id: Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
        node_type: "raisin:Folder".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: Some("user".to_string()),
        created_by: Some("user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

async fn setup(temp_dir: &TempDir) -> Result<Arc<RocksDBStorage>> {
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;

    let repo_config = RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
        default_branch: BRANCH.to_string(),
        description: Some("Stranded-node test".to_string()),
        tags: HashMap::new(),
    };
    storage
        .repository_management()
        .create_repository(TENANT, REPO, repo_config)
        .await?;

    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;

    let folder_type = NodeType {
        id: Some("raisin:Folder".to_string()),
        strict: Some(false),
        name: "raisin:Folder".to_string(),
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
        versionable: Some(true),
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
            BranchScope::new(TENANT, REPO, BRANCH),
            folder_type,
            CommitMetadata::system("seed folder type"),
        )
        .await?;

    let mut workspace = Workspace::new(WORKSPACE.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await?;

    Ok(storage)
}

async fn begin(storage: &Arc<RocksDBStorage>) -> Result<Box<dyn TransactionalContext>> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_actor("test")?;
    tx.set_message("test write")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    Ok(tx)
}

/// Commit a node, then force it ABOVE the branch HEAD by rolling HEAD back to
/// where it was before the commit. Returns the node's id.
async fn strand_node(storage: &Arc<RocksDBStorage>, path: &str) -> Result<String> {
    let head_before = storage.branches().get_head(TENANT, REPO, BRANCH).await?;

    let node = build_node(path, "Stranded");
    let id = node.id.clone();
    let tx = begin(storage).await?;
    tx.add_node(WORKSPACE, &node).await?;
    tx.commit().await?;

    let head_after = storage.branches().get_head(TENANT, REPO, BRANCH).await?;
    assert!(
        head_after > head_before,
        "committing the node must have advanced HEAD (otherwise the test setup is a no-op)"
    );

    // Deliberate HEAD rollback: reproduces the residue the pre-v0.1.60
    // regression left behind. The node's latest revision now sits ABOVE HEAD.
    storage
        .branches()
        .set_head(TENANT, REPO, BRANCH, head_before)
        .await?;

    // Sanity: the node is genuinely invisible to a HEAD-bounded probe...
    let probe = begin(storage).await?;
    assert!(
        probe.get_node_by_path(WORKSPACE, path).await?.is_none(),
        "HEAD-bounded probe must not see a node stranded above HEAD"
    );
    assert!(
        probe.get_node(WORKSPACE, &id).await?.is_none(),
        "HEAD-bounded by-id probe must not see a node stranded above HEAD"
    );
    probe.rollback().await?;

    Ok(id)
}

/// A node stranded above HEAD must be upsertable by PATH, must keep its
/// identity (no duplicate node at the path), and the write must advance HEAD
/// past the stranded revision so the node becomes visible again.
#[tokio::test]
async fn stranded_node_can_be_upserted_by_path() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let storage = setup(&temp_dir).await?;

    let stranded_id = strand_node(&storage, "/stranded").await?;

    // The caller has no idea the node exists (its own reads say it doesn't),
    // so it upserts with a FRESH id.
    let mut incoming = build_node("/stranded", "Rewritten");
    incoming.id = Uuid::new_v4().to_string();

    let tx = begin(&storage).await?;
    tx.upsert_node(WORKSPACE, &incoming)
        .await
        .expect("upsert of a node stranded above HEAD must succeed");
    tx.commit().await?;

    // Self-heal: HEAD advanced past the stranded revision, so the node is
    // visible to HEAD-bounded reads again.
    let probe = begin(&storage).await?;
    let read = probe
        .get_node_by_path(WORKSPACE, "/stranded")
        .await?
        .expect("node must be readable via a HEAD-bounded read after the write");

    // Identity is ADOPTED, not duplicated: one node at the path, still the
    // original id, carrying the new content.
    assert_eq!(
        read.id, stranded_id,
        "upsert must adopt the stranded node's id rather than create a second node at the path"
    );
    assert_eq!(
        read.properties.get("title"),
        Some(&PropertyValue::String("Rewritten".to_string())),
        "the new content must have been written"
    );

    let all = probe.scan_nodes(WORKSPACE).await?;
    let at_path = all.iter().filter(|n| n.path == "/stranded").count();
    assert_eq!(at_path, 1, "exactly one live node may occupy the path");
    probe.rollback().await?;

    Ok(())
}

/// The same for `put_node` (create-or-update by ID).
#[tokio::test]
async fn stranded_node_can_be_put_by_id() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let storage = setup(&temp_dir).await?;

    let stranded_id = strand_node(&storage, "/stranded-put").await?;

    let mut incoming = build_node("/stranded-put", "Rewritten via put");
    incoming.id = stranded_id.clone();

    let tx = begin(&storage).await?;
    tx.put_node(WORKSPACE, &incoming)
        .await
        .expect("put_node of a node stranded above HEAD must succeed");
    tx.commit().await?;

    let probe = begin(&storage).await?;
    let read = probe
        .get_node(WORKSPACE, &stranded_id)
        .await?
        .expect("node must be readable via a HEAD-bounded read after the write");
    assert_eq!(
        read.properties.get("title"),
        Some(&PropertyValue::String("Rewritten via put".to_string()))
    );

    let all = probe.scan_nodes(WORKSPACE).await?;
    let at_path = all.iter().filter(|n| n.path == "/stranded-put").count();
    assert_eq!(at_path, 1, "exactly one live node may occupy the path");
    probe.rollback().await?;

    Ok(())
}

/// The uniqueness checks must NOT be weakened: two DIFFERENT nodes sharing an
/// id, or sharing a path, at or below HEAD still conflict.
#[tokio::test]
async fn genuine_conflicts_below_head_still_error() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let storage = setup(&temp_dir).await?;

    let original = build_node("/original", "Original");
    let tx = begin(&storage).await?;
    tx.add_node(WORKSPACE, &original).await?;
    tx.commit().await?;

    // (a) same id, different node (different path) -> id conflict on CREATE.
    let mut same_id = build_node("/elsewhere", "Impostor");
    same_id.id = original.id.clone();
    let tx = begin(&storage).await?;
    let err = tx.add_node(WORKSPACE, &same_id).await;
    assert!(
        err.is_err(),
        "creating a second node under an existing id must still conflict, got {:?}",
        err.map(|_| ())
    );
    tx.rollback().await?;

    // (b) different id, same path -> path conflict on CREATE.
    let occupied = build_node("/original", "Impostor");
    let tx = begin(&storage).await?;
    let err = tx.add_node(WORKSPACE, &occupied).await;
    assert!(
        err.is_err(),
        "creating a second node at an occupied path must still conflict"
    );
    tx.rollback().await?;

    // (c) put_node with a FRESH id at an occupied path is a create-at-occupied
    //     -path and must still conflict.
    let occupied_put = build_node("/original", "Impostor via put");
    let tx = begin(&storage).await?;
    let err = tx.put_node(WORKSPACE, &occupied_put).await;
    assert!(
        err.is_err(),
        "put_node with a new id at an occupied path must still conflict"
    );
    tx.rollback().await?;

    Ok(())
}

/// A node stranded above HEAD that was DELETED above HEAD must not resurrect:
/// the tombstone is the newest entry, so identity resolution finds nothing and
/// the write is a genuine CREATE.
#[tokio::test]
async fn stranded_tombstone_does_not_block_or_resurrect() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let storage = setup(&temp_dir).await?;

    let head_before = storage.branches().get_head(TENANT, REPO, BRANCH).await?;

    let node = build_node("/gone", "Gone");
    let tx = begin(&storage).await?;
    tx.add_node(WORKSPACE, &node).await?;
    tx.commit().await?;

    let tx = begin(&storage).await?;
    tx.delete_node(WORKSPACE, &node.id).await?;
    tx.commit().await?;

    storage
        .branches()
        .set_head(TENANT, REPO, BRANCH, head_before)
        .await?;

    let fresh = build_node("/gone", "Fresh");
    let fresh_id = fresh.id.clone();
    let tx = begin(&storage).await?;
    tx.upsert_node(WORKSPACE, &fresh)
        .await
        .expect("a path whose newest entry above HEAD is a tombstone must be creatable");
    tx.commit().await?;

    let probe = begin(&storage).await?;
    let read = probe
        .get_node_by_path(WORKSPACE, "/gone")
        .await?
        .expect("newly created node must be readable");
    assert_eq!(
        read.id, fresh_id,
        "a deleted stranded node must not be resurrected - the create keeps its own id"
    );
    probe.rollback().await?;

    Ok(())
}
