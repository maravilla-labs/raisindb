//! Direct-repository writes must replicate with the SAME attribution the origin
//! recorded locally.
//!
//! The repository layer holds no `AuthContext`, so `replication_capture.rs` used
//! to hardcode `actor = "system"` and `agent = None` on every `ApplyRevision` it
//! captured. Reorder / move / copy-tree / direct update therefore named the real
//! user on the ORIGIN and "system" on every replica -- and in a masterless
//! cluster an entry that differs between nodes is simply wrong.
//!
//! These tests pin the actor+agent that lands in the captured operation.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::operations::OperationMeta;
use raisin_models::workspace::Workspace;
use raisin_replication::{OpType, Operation};
use raisin_rocksdb::fractional_index;
use raisin_rocksdb::{OpLogRepository, RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::{BranchScope, StorageScope};
use raisin_storage::{
    BranchRepository, CommitMetadata, CreateNodeOptions, NodeRepository, NodeTypeRepository,
    RegistryRepository, RepositoryManagementRepository, RevisionRepository, Storage,
    UpdateNodeOptions,
};
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "repl-attr-test";
const REPO: &str = "main-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
}

impl Env {
    async fn new(node_id: &str) -> Result<Self> {
        let temp_dir = TempDir::new().unwrap();
        let mut config = RocksDBConfig::default();
        config.path = temp_dir.path().to_path_buf();
        // Capture only runs when replication is on -- otherwise there is no
        // operation to assert against.
        config.replication_enabled = true;
        config.cluster_node_id = Some(node_id.to_string());
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
            description: Some("Replication attribution tests".to_string()),
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

        let mut workspace = Workspace::new(WORKSPACE.to_string());
        workspace.config.default_branch = BRANCH.to_string();
        WorkspaceService::new(storage.clone())
            .put(TENANT, REPO, workspace)
            .await?;

        seed_node_types(&storage).await?;

        Ok(Self {
            _dir: temp_dir,
            storage,
        })
    }
}

fn relaxed_create_options() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    }
}

fn build_node(path: &str, node_type: &str) -> Node {
    let segments: Vec<&str> = path.rsplitn(2, '/').collect();
    let name = segments[0].to_string();
    let parent = if segments.len() > 1 && !segments[1].is_empty() {
        Some(segments[1].to_string())
    } else {
        Some("/".to_string())
    };

    let mut properties = HashMap::new();
    properties.insert("title".to_string(), PropertyValue::String(name.clone()));

    Node {
        id: Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
        node_type: node_type.to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent,
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

fn make_node_type(name: &str, allowed_children: Vec<&str>) -> NodeType {
    NodeType {
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
        allowed_children: allowed_children
            .into_iter()
            .map(|c| c.to_string())
            .collect(),
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
    }
}

async fn seed_node_types(storage: &Arc<RocksDBStorage>) -> Result<()> {
    for ty in [
        make_node_type("raisin:Folder", vec!["*"]),
        make_node_type("raisin:Article", Vec::new()),
    ] {
        let label = ty.name.clone();
        storage
            .node_types()
            .put(
                BranchScope::new(TENANT, REPO, BRANCH),
                ty,
                CommitMetadata::system(&format!("seed {}", label)),
            )
            .await?;
    }
    Ok(())
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE)
}

/// Every `ApplyRevision` captured so far, oldest first.
fn apply_revision_ops(storage: &Arc<RocksDBStorage>) -> Vec<Operation> {
    let repo = OpLogRepository::new(storage.db().clone());
    let mut ops = Vec::new();
    if let Ok(map) = repo.get_all_operations(TENANT, REPO) {
        for (_, batch) in map {
            for op in batch {
                if matches!(op.op_type, OpType::ApplyRevision { .. }) {
                    ops.push(op);
                }
            }
        }
    }
    ops.sort_by_key(|op| op.op_seq);
    ops
}

/// The newest `ApplyRevision` that carries `node_id`.
fn latest_op_for(storage: &Arc<RocksDBStorage>, node_id: &str) -> Operation {
    apply_revision_ops(storage)
        .into_iter()
        .filter(|op| match &op.op_type {
            OpType::ApplyRevision { node_changes, .. } => {
                node_changes.iter().any(|c| c.node.id == node_id)
            }
            _ => false,
        })
        .next_back()
        .expect("expected an ApplyRevision covering the node")
}

#[tokio::test]
async fn reorder_replicates_the_real_actor_not_system() -> Result<()> {
    let env = Env::new("node-reorder").await?;
    let storage = env.storage.clone();
    let nodes = storage.nodes();

    let parent = build_node("/menu", "raisin:Folder");
    nodes
        .create(scope(), parent, relaxed_create_options())
        .await?;
    for name in ["a", "b", "c"] {
        let child = build_node(&format!("/menu/{}", name), "raisin:Article");
        nodes
            .create(scope(), child, relaxed_create_options())
            .await?;
    }

    let moved = nodes
        .get_by_path(scope(), "/menu/c", None)
        .await?
        .expect("child c exists");

    nodes
        .reorder_child(scope(), "/menu", "c", 0, Some("promote c"), Some("alice"))
        .await?;

    let op = latest_op_for(&storage, &moved.id);
    assert_eq!(
        op.actor, "alice",
        "the reorder must replicate as the user who performed it, not \"system\""
    );

    Ok(())
}

#[tokio::test]
async fn move_tree_replicates_the_operation_meta_actor_and_agent() -> Result<()> {
    let env = Env::new("node-move").await?;
    let storage = env.storage.clone();
    let nodes = storage.nodes();

    for path in ["/source", "/archive"] {
        let folder = build_node(path, "raisin:Folder");
        nodes
            .create(scope(), folder, relaxed_create_options())
            .await?;
    }
    let leaf = build_node("/source/doc", "raisin:Article");
    nodes
        .create(scope(), leaf, relaxed_create_options())
        .await?;

    let source = nodes
        .get_by_path(scope(), "/source", None)
        .await?
        .expect("source exists");

    let revision = storage.revisions().allocate_revision();
    let meta = OperationMeta::new_move(
        source.id.clone(),
        "/source".to_string(),
        "/".to_string(),
        "/archive/source".to_string(),
        "/archive".to_string(),
        &revision,
        None,
        "bob".to_string(),
        "archive the source tree".to_string(),
    )
    .with_agent(Some("trigger:/triggers/nightly-archive".to_string()));

    nodes
        .move_node_tree(scope(), &source.id, "/archive/source", Some(meta))
        .await?;

    let op = latest_op_for(&storage, &source.id);
    assert_eq!(op.actor, "bob", "move must replicate the real actor");
    assert_eq!(
        op.agent.as_deref(),
        Some("trigger:/triggers/nightly-archive"),
        "move must replicate the initiating agent alongside the actor"
    );

    Ok(())
}

#[tokio::test]
async fn direct_update_replicates_the_operation_meta_actor_and_agent() -> Result<()> {
    let env = Env::new("node-update").await?;
    let storage = env.storage.clone();
    let nodes = storage.nodes();

    let node = build_node("/post", "raisin:Article");
    let node_id = node.id.clone();
    nodes
        .create(scope(), node, relaxed_create_options())
        .await?;

    let mut stored = nodes
        .get(scope(), &node_id, None)
        .await?
        .expect("node exists");
    stored.properties.insert(
        "title".to_string(),
        PropertyValue::String("edited".to_string()),
    );

    let revision = storage.revisions().allocate_revision();
    let meta = OperationMeta::new_rename(
        node_id.clone(),
        "post".to_string(),
        "post".to_string(),
        &revision,
        None,
        "carol".to_string(),
        "edit title".to_string(),
    )
    .with_agent(Some(
        "flow:/flows/publish@trigger:/triggers/on-edit".to_string(),
    ));

    nodes
        .update(
            scope(),
            stored,
            UpdateNodeOptions {
                validate_schema: false,
                allow_type_change: false,
                operation_meta: Some(meta),
            },
        )
        .await?;

    let op = latest_op_for(&storage, &node_id);
    assert_eq!(op.actor, "carol", "update must replicate the real actor");
    assert_eq!(
        op.agent.as_deref(),
        Some("flow:/flows/publish@trigger:/triggers/on-edit"),
        "update must replicate the initiating agent"
    );

    Ok(())
}

/// A write with no attribution keeps recording `"system"` -- the fallback must
/// not change for system jobs and seeds that genuinely have no principal.
#[tokio::test]
async fn a_write_without_attribution_still_records_system() -> Result<()> {
    let env = Env::new("node-default").await?;
    let storage = env.storage.clone();
    let nodes = storage.nodes();

    let node = build_node("/plain", "raisin:Article");
    let node_id = node.id.clone();
    nodes
        .create(scope(), node, relaxed_create_options())
        .await?;

    let op = latest_op_for(&storage, &node_id);
    assert_eq!(op.actor, "system");
    assert_eq!(op.agent, None);

    Ok(())
}
