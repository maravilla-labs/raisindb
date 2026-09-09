//! `NodeType.immutable` and `NodeType.versionable` enforcement, exercised
//! through both the transaction layer (`put_node`) and the repository-layer
//! backstop (`update_impl`) — the two independent write paths documented in
//! `crate::immutability`.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_models::workspace::Workspace;
use raisin_rocksdb::{fractional_index, RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeRepository, NodeTypeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "immutable-test";
const REPO: &str = "main-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

const IMMUTABLE_TYPE: &str = "test:LedgerEntry";
const PLAIN_TYPE: &str = "raisin:Folder";
const NON_VERSIONABLE_TYPE: &str = "test:HealthPing";

fn node_type(name: &str, immutable: Option<bool>, versionable: Option<bool>) -> NodeType {
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
        allowed_children: vec!["*".to_string()],
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable,
        immutable,
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

fn build_node(path: &str, ty: &str) -> Node {
    let name = path.trim_start_matches('/').to_string();
    let mut properties = HashMap::new();
    properties.insert("title".to_string(), PropertyValue::String(name.clone()));

    Node {
        id: Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
        node_type: ty.to_string(),
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
        updated_by: None,
        created_by: None,
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

async fn setup() -> Result<(Arc<RocksDBStorage>, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;
    storage
        .repository_management()
        .create_repository(
            TENANT,
            REPO,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string()],
                locale_fallback_chains: HashMap::new(),
                default_branch: BRANCH.to_string(),
                description: Some("immutable nodetype test".to_string()),
                tags: HashMap::new(),
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;

    for (name, immutable, versionable) in [
        (PLAIN_TYPE, None, None),
        (IMMUTABLE_TYPE, Some(true), None),
        (NON_VERSIONABLE_TYPE, None, Some(false)),
    ] {
        storage
            .node_types()
            .upsert(
                BranchScope::new(TENANT, REPO, BRANCH),
                node_type(name, immutable, versionable),
                CommitMetadata::system("seed type"),
            )
            .await?;
    }

    let mut workspace = Workspace::new(WORKSPACE.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await?;

    Ok((storage, temp_dir))
}

fn admin() -> AuthContext {
    AuthContext::for_user("test-admin").with_permissions(ResolvedPermissions {
        user_id: "test-admin".to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![Permission::new(
            "/**",
            vec![
                Operation::Read,
                Operation::Create,
                Operation::Update,
                Operation::Delete,
            ],
        )],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
}

async fn create(storage: &Arc<RocksDBStorage>, path: &str, ty: &str) -> Result<Node> {
    let node = build_node(path, ty);
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("create")?;
    tx.set_auth_context(admin())?;
    tx.set_validate_schema(false)?;
    tx.add_node(WORKSPACE, &node).await?;
    tx.commit().await?;
    Ok(node)
}

async fn put(storage: &Arc<RocksDBStorage>, node: &Node) -> Result<()> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("update")?;
    tx.set_auth_context(admin())?;
    tx.set_validate_schema(false)?;
    tx.put_node(WORKSPACE, node).await?;
    tx.commit().await
}

#[tokio::test]
async fn create_allowed_once() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    create(&storage, "/entry-1", IMMUTABLE_TYPE).await?;
    Ok(())
}

#[tokio::test]
async fn property_update_rejected_on_immutable_node() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let mut node = create(&storage, "/entry-2", IMMUTABLE_TYPE).await?;

    node.properties.insert(
        "title".to_string(),
        PropertyValue::String("tampered".to_string()),
    );
    let err = put(&storage, &node).await.unwrap_err();
    assert!(
        matches!(err, raisin_error::Error::Conflict(_)),
        "expected Conflict, got {:?}",
        err
    );
    Ok(())
}

#[tokio::test]
async fn move_allowed_on_immutable_node_when_properties_unchanged() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let mut node = create(&storage, "/entry-3", IMMUTABLE_TYPE).await?;

    // Path/parent move with no property change must be allowed — immutability
    // is scoped to `properties` only.
    node.path = "/entry-3-moved".to_string();
    node.parent = Some("/".to_string());
    put(&storage, &node).await?;
    Ok(())
}

#[tokio::test]
async fn plain_type_updates_freely() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let mut node = create(&storage, "/entry-4", PLAIN_TYPE).await?;

    node.properties.insert(
        "title".to_string(),
        PropertyValue::String("changed".to_string()),
    );
    put(&storage, &node).await?;
    Ok(())
}

#[tokio::test]
async fn versionable_false_does_not_mint_new_history_entries() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let mut node = create(&storage, "/ping-1", NON_VERSIONABLE_TYPE).await?;

    for i in 0..3 {
        node.properties.insert(
            "title".to_string(),
            PropertyValue::String(format!("tick-{i}")),
        );
        put(&storage, &node).await?;
    }

    let history = storage
        .nodes()
        .get_history(TENANT, REPO, BRANCH, WORKSPACE, &node.id, None)
        .await?;
    assert_eq!(
        history.len(),
        1,
        "versionable=false must overwrite in place, not mint a history entry per write"
    );

    // The latest read must still reflect the newest content.
    let latest = storage
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &node.id,
            None,
        )
        .await?
        .expect("node should still exist");
    assert_eq!(
        latest.properties.get("title"),
        Some(&PropertyValue::String("tick-2".to_string()))
    );
    Ok(())
}

#[tokio::test]
async fn versionable_default_mints_a_history_entry_per_write() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let mut node = create(&storage, "/entry-5", PLAIN_TYPE).await?;

    for i in 0..3 {
        node.properties
            .insert("title".to_string(), PropertyValue::String(format!("v{i}")));
        put(&storage, &node).await?;
    }

    let history = storage
        .nodes()
        .get_history(TENANT, REPO, BRANCH, WORKSPACE, &node.id, None)
        .await?;
    assert_eq!(
        history.len(),
        4,
        "default versionable behavior mints one entry per write (1 create + 3 updates)"
    );
    Ok(())
}
