//! Regression test for the branch-HEAD monotonicity bug.
//!
//! `RocksDBTransaction::update_branch_head` used to unconditionally overwrite
//! a branch's `head` on every commit. If a transaction that allocated an
//! OLDER revision commits its batch AFTER a transaction that allocated a
//! NEWER revision, the older commit would regress branch HEAD backward,
//! silently hiding the newer commit's nodes from any `at_revision(head)`-bound
//! read even though the node data itself stayed durably stored. This is
//! exactly what was observed in prod: a node visible via an unbounded SQL
//! scan but 404ing via the normal (revision-bound) HTTP read path.
//!
//! This test forces that exact interleaving deterministically (allocate
//! revision A, allocate revision B > A, commit B first, then commit A) and
//! asserts HEAD does not regress and both nodes remain reachable.

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
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeRepository, NodeTypeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "branchhead-test";
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

#[tokio::test]
async fn commit_with_older_revision_does_not_regress_branch_head() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
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
        description: Some("Branch head monotonicity test".to_string()),
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

    // Seed the node type BEFORE creating the workspace: WorkspaceService::put
    // bootstraps a ROOT node for new workspaces, which requires the default
    // folder type to already be registered.
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

    // Transaction A: write a node, allocating the OLDER revision, but don't
    // commit yet.
    let node_a = build_node("/node-a", "Node A");
    let tx_a = storage.begin_context().await?;
    tx_a.set_tenant_repo(TENANT, REPO)?;
    tx_a.set_branch(BRANCH)?;
    tx_a.set_actor("test")?;
    tx_a.set_message("create node-a")?;
    tx_a.set_auth_context(AuthContext::system())?;
    tx_a.set_validate_schema(false)?;
    tx_a.add_node(WORKSPACE, &node_a).await?;

    // Transaction B: write a second node, allocating a NEWER revision
    // (allocate_revision is monotonic per call), and commit it FIRST -
    // advancing branch HEAD.
    let node_b = build_node("/node-b", "Node B");
    let tx_b = storage.begin_context().await?;
    tx_b.set_tenant_repo(TENANT, REPO)?;
    tx_b.set_branch(BRANCH)?;
    tx_b.set_actor("test")?;
    tx_b.set_message("create node-b")?;
    tx_b.set_auth_context(AuthContext::system())?;
    tx_b.set_validate_schema(false)?;
    tx_b.add_node(WORKSPACE, &node_b).await?;
    tx_b.commit().await?;

    let head_after_b = storage.branches().get_head(TENANT, REPO, BRANCH).await?;

    // Now commit transaction A - carrying the OLDER revision - AFTER B. Before
    // the fix this unconditionally overwrote HEAD with A's older revision,
    // regressing it and hiding node-b from any at_revision(head) read.
    tx_a.commit().await?;

    let head_after_a = storage.branches().get_head(TENANT, REPO, BRANCH).await?;

    assert_eq!(
        head_after_a, head_after_b,
        "branch HEAD must not regress when an older-revision transaction commits after a newer one"
    );

    // Both nodes must still be durably readable - the older commit's data is
    // never lost, only its HEAD-advance is (correctly) skipped.
    let read_a = storage
        .nodes()
        .get_by_path(
            raisin_storage::scope::StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "/node-a",
            None,
        )
        .await?;
    let read_b = storage
        .nodes()
        .get_by_path(
            raisin_storage::scope::StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "/node-b",
            None,
        )
        .await?;
    assert!(read_a.is_some(), "node-a must remain readable");
    assert!(read_b.is_some(), "node-b must remain readable");

    Ok(())
}
