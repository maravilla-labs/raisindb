use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_core::NodeService;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_replication::{
    operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
    OpType, Operation,
};
use raisin_rocksdb::fractional_index;
use raisin_rocksdb::{OpLogRepository, RocksDBConfig, RocksDBStorage};
use raisin_storage::{
    BranchRepository, CommitMetadata, CreateNodeOptions, DeleteNodeOptions, NodeRepository,
    NodeTypeRepository, RegistryRepository, RepositoryManagementRepository, Storage,
    UpdateNodeOptions,
};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "applyrev-test";
const REPO: &str = "main-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

struct ApplyRevisionCaptureEnv {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
}

impl ApplyRevisionCaptureEnv {
    async fn new() -> Result<Self> {
        let temp_dir = TempDir::new().unwrap();
        let mut config = RocksDBConfig::default();
        config.path = temp_dir.path().to_path_buf();
        config.replication_enabled = true;
        config.cluster_node_id = Some("node-capture".to_string());
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
            description: Some("Apply revision capture tests".to_string()),
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
        let workspace_service = WorkspaceService::new(storage.clone());
        workspace_service.put(TENANT, REPO, workspace).await?;

        Ok(Self {
            _dir: temp_dir,
            storage,
        })
    }

    fn storage(&self) -> Arc<RocksDBStorage> {
        self.storage.clone()
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

fn relaxed_update_options() -> UpdateNodeOptions {
    UpdateNodeOptions {
        validate_schema: false,
        allow_type_change: true,
        operation_meta: None,
    }
}

fn build_node(path: &str, node_type: &str, title: &str) -> Node {
    let segments: Vec<&str> = path.rsplitn(2, '/').collect();
    let name = segments[0].to_string();
    let parent = if segments.len() > 1 {
        let parent_str = if segments[1].is_empty() {
            "/"
        } else {
            segments[1]
        };
        Some(parent_str.to_string())
    } else {
        Some("/".to_string())
    };

    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String(title.to_string()),
    );

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

async fn seed_transaction_node_types(storage: &Arc<RocksDBStorage>) -> Result<()> {
    let folder = make_node_type("raisin:Folder", vec!["*"]);
    storage
        .node_types()
        .put(
            TENANT,
            REPO,
            BRANCH,
            folder,
            CommitMetadata::system("seed folder type"),
        )
        .await?;

    let article = make_node_type("raisin:Article", Vec::new());
    storage
        .node_types()
        .put(
            TENANT,
            REPO,
            BRANCH,
            article,
            CommitMetadata::system("seed article type"),
        )
        .await?;

    Ok(())
}

fn latest_apply_revision_op(storage: &Arc<RocksDBStorage>) -> Operation {
    let repo = OpLogRepository::new(storage.db().clone());
    let mut apply_ops = Vec::new();

    if let Ok(map) = repo.get_all_operations(TENANT, REPO) {
        for (_, ops) in map {
            for op in ops {
                if matches!(op.op_type, OpType::ApplyRevision { .. }) {
                    apply_ops.push(op);
                }
            }
        }
    }

    apply_ops.sort_by_key(|op| op.op_seq);
    apply_ops
        .last()
        .cloned()
        .expect("expected at least one ApplyRevision operation")
}

fn extract_change<'a>(
    op: &'a Operation,
    node_id: &str,
) -> Option<&'a raisin_replication::operation::ReplicatedNodeChange> {
    match &op.op_type {
        OpType::ApplyRevision { node_changes, .. } => {
            node_changes.iter().find(|change| change.node.id == node_id)
        }
        _ => None,
    }
}

#[tokio::test]
async fn apply_revision_captures_node_mutations() -> Result<()> {
    let env = ApplyRevisionCaptureEnv::new().await?;
    let storage = env.storage();
    let nodes = storage.nodes();

    let parent = build_node("/articles", "raisin:Folder", "Articles");
    nodes
        .create(
            TENANT,
            REPO,
            BRANCH,
            WORKSPACE,
            parent,
            relaxed_create_options(),
        )
        .await?;

    let mut article = build_node("/articles/hello-world", "raisin:Article", "Hello World");
    nodes
        .create(
            TENANT,
            REPO,
            BRANCH,
            WORKSPACE,
            article.clone(),
            relaxed_create_options(),
        )
        .await?;

    let op = latest_apply_revision_op(&storage);
    let change = extract_change(&op, &article.id).expect("change for create");
    assert_eq!(change.kind, ReplicatedNodeChangeKind::Upsert);
    assert_eq!(change.node.path, "/articles/hello-world");

    let mut stored = nodes
        .get(TENANT, REPO, BRANCH, WORKSPACE, &article.id, None)
        .await?
        .expect("node present");
    stored.properties.insert(
        "title".to_string(),
        PropertyValue::String("Updated Title".to_string()),
    );

    nodes
        .update(
            TENANT,
            REPO,
            BRANCH,
            WORKSPACE,
            stored.clone(),
            relaxed_update_options(),
        )
        .await?;

    let op = latest_apply_revision_op(&storage);
    let change = extract_change(&op, &article.id).expect("change for update");
    if let Some(PropertyValue::String(value)) = change.node.properties.get("title") {
        assert_eq!(value, "Updated Title");
    } else {
        panic!("title property missing from ApplyRevision update");
    }

    nodes
        .move_node(
            TENANT,
            REPO,
            BRANCH,
            WORKSPACE,
            &article.id,
            "/articles/moved-node",
            None,
        )
        .await?;

    let op = latest_apply_revision_op(&storage);
    let change = extract_change(&op, &article.id).expect("change for move");
    assert_eq!(change.node.path, "/articles/moved-node");

    nodes
        .delete(
            TENANT,
            REPO,
            BRANCH,
            WORKSPACE,
            &article.id,
            DeleteNodeOptions::default(),
        )
        .await?;

    let op = latest_apply_revision_op(&storage);
    let change = extract_change(&op, &article.id).expect("change for delete");
    assert_eq!(change.kind, ReplicatedNodeChangeKind::Delete);
    assert!(nodes
        .get(TENANT, REPO, BRANCH, WORKSPACE, &article.id, None)
        .await?
        .is_none());

    Ok(())
}

#[tokio::test]
async fn apply_revision_captures_transaction_mutations() -> Result<()> {
    let env = ApplyRevisionCaptureEnv::new().await?;
    let storage = env.storage();
    seed_transaction_node_types(&storage).await?;

    let nodes = storage.nodes();
    let parent = build_node("/articles", "raisin:Folder", "Articles");
    nodes
        .create(
            TENANT,
            REPO,
            BRANCH,
            WORKSPACE,
            parent,
            relaxed_create_options(),
        )
        .await?;

    let mut article = build_node("/articles/tx-article", "raisin:Article", "Tx Article");

    let node_service = NodeService::new_with_context(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
        WORKSPACE.to_string(),
    );

    let mut create_tx = node_service.transaction();
    create_tx.create(article.clone());
    create_tx.commit("tx create article", "txn-user").await?;

    let op = latest_apply_revision_op(&storage);
    let change = extract_change(&op, &article.id).expect("transaction create change");
    assert_eq!(change.kind, ReplicatedNodeChangeKind::Upsert);
    assert_eq!(change.node.path, article.path);

    let mut update_tx = node_service.transaction();
    update_tx.update(
        article.id.clone(),
        json!({
            "title": "Tx Updated Title"
        }),
    );
    update_tx.move_node(
        article.id.clone(),
        "/articles/tx-article-updated".to_string(),
    );
    update_tx.commit("tx update article", "txn-user").await?;

    let op = latest_apply_revision_op(&storage);
    let change = extract_change(&op, &article.id).expect("transaction update change");
    assert_eq!(change.node.path, "/articles/tx-article-updated");
    match change.node.properties.get("title") {
        Some(PropertyValue::String(value)) => assert_eq!(value, "Tx Updated Title"),
        other => panic!("expected updated title, got {:?}", other),
    }

    let mut delete_tx = node_service.transaction();
    delete_tx.delete(article.id.clone());
    delete_tx.commit("tx delete article", "txn-user").await?;

    let op = latest_apply_revision_op(&storage);
    let change = extract_change(&op, &article.id).expect("transaction delete change");
    assert_eq!(change.kind, ReplicatedNodeChangeKind::Delete);

    assert!(storage
        .nodes()
        .get(TENANT, REPO, BRANCH, WORKSPACE, &article.id, None)
        .await?
        .is_none());

    Ok(())
}
