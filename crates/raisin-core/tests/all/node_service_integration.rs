//! Integration tests for NodeService with both InMemory and RocksDB storage
//!
//! Run with: cargo test --package raisin-core --test node_service_integration
//! Run with RocksDB: cargo test --package raisin-core --test node_service_integration --features store-rocks

use std::collections::HashMap;
use std::sync::Arc;

use raisin_core::NodeService;
use raisin_models::nodes::properties::schema::{PropertyType, PropertyValueSchema};
use raisin_models::nodes::types::{
    initial_structure::{InitialChild, InitialNodeStructure},
    NodeType,
};
use raisin_models::nodes::{properties::PropertyValue, Node};
use raisin_storage::{BranchScope, CommitMetadata, NodeTypeRepository, Storage};

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;

/// Setup test storage - works with both backends
#[cfg(not(feature = "storage-rocksdb"))]
fn setup_storage() -> Arc<InMemoryStorage> {
    Arc::new(InMemoryStorage::default())
}

#[cfg(feature = "storage-rocksdb")]
fn setup_storage() -> Arc<RocksDBStorage> {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let storage = RocksDBStorage::open(path).unwrap();
    // Keep the tempdir alive by leaking it (for test duration)
    std::mem::forget(dir);
    Arc::new(storage)
}

async fn create_test_node_type<S: Storage>(
    storage: &S,
    name: &str,
    properties: Vec<PropertyValueSchema>,
    strict: bool,
    published: bool,
) {
    let node_type = NodeType {
        id: Some(name.to_string()),
        strict: Some(strict),
        name: name.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: Some(properties),
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(published),
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
            BranchScope::new("default", "default", "main"),
            node_type,
            CommitMetadata::system("create test node type"),
        )
        .await
        .unwrap();
}

fn create_property_schema(
    name: &str,
    property_type: PropertyType,
    required: bool,
    unique: bool,
) -> PropertyValueSchema {
    PropertyValueSchema {
        name: Some(name.to_string()),
        property_type,
        required: Some(required),
        unique: Some(unique),
        default: None,
        constraints: None,
        structure: None,
        items: None,
        value: None,
        meta: None,
        is_translatable: None,
        allow_additional_properties: None,
        index: None,
        spatial: None,
        encrypted: None,
    }
}

fn create_test_node(
    name: &str,
    node_type: &str,
    properties: HashMap<String, PropertyValue>,
) -> Node {
    Node {
        id: String::new(),
        name: name.to_string(),
        path: String::new(),
        node_type: node_type.to_string(),
        archetype: None,
        properties,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: None,
        version: 1,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: None,
        created_by: None,
        translations: None,
        tenant_id: None,
        workspace: None,
        owner_id: None,
        relations: Vec::new(),
    }
}

#[tokio::test]
async fn test_node_creation_with_validation() {
    let storage = setup_storage();
    let service = NodeService::new(storage.clone());

    // Create a published NodeType with required property
    create_test_node_type(
        &*storage,
        "test:Article",
        vec![create_property_schema(
            "title",
            PropertyType::String,
            true,
            false,
        )],
        false,
        true,
    )
    .await;

    // Create node with required property
    let mut props = HashMap::new();
    props.insert(
        "title".to_string(),
        PropertyValue::String("My Article".to_string()),
    );
    let node = create_test_node("test-article", "test:Article", props);

    // Should succeed
    let result = service.add_node("/", node).await;
    assert!(
        result.is_ok(),
        "Node creation should succeed with valid data"
    );

    let created = result.unwrap();
    assert_eq!(created.node_type, "test:Article");
    assert!(!created.id.is_empty());
    assert_eq!(created.path, "/test-article");
}

#[tokio::test]
async fn test_validation_prevents_invalid_nodes() {
    let storage = setup_storage();
    let service = NodeService::new(storage.clone());

    // Create a published NodeType with required property
    create_test_node_type(
        &*storage,
        "test:Article",
        vec![create_property_schema(
            "title",
            PropertyType::String,
            true,
            false,
        )],
        false,
        true,
    )
    .await;

    // Try to create node WITHOUT required property
    let node = create_test_node("test-article", "test:Article", HashMap::new());

    // Should fail
    let result = service.add_node("/", node).await;
    assert!(
        result.is_err(),
        "Should fail when missing required property"
    );
    assert!(result.unwrap_err().to_string().contains("required"));
}

#[tokio::test]
async fn test_strict_mode_enforcement() {
    let storage = setup_storage();
    let service = NodeService::new(storage.clone());

    // Create strict NodeType
    create_test_node_type(
        &*storage,
        "test:Strict",
        vec![create_property_schema(
            "title",
            PropertyType::String,
            false,
            false,
        )],
        true, // strict mode
        true,
    )
    .await;

    // Try to create node with undefined property
    let mut props = HashMap::new();
    props.insert(
        "title".to_string(),
        PropertyValue::String("Valid".to_string()),
    );
    props.insert(
        "undefined_prop".to_string(),
        PropertyValue::String("Invalid".to_string()),
    );
    let node = create_test_node("test-node", "test:Strict", props);

    // Should fail
    let result = service.add_node("/", node).await;
    assert!(
        result.is_err(),
        "Should fail with undefined property in strict mode"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Undefined property"));
}

#[tokio::test]
async fn test_unique_property_constraint() {
    let storage = setup_storage();
    let service = NodeService::new(storage.clone());

    // Create NodeType with unique property
    create_test_node_type(
        &*storage,
        "test:User",
        vec![create_property_schema(
            "email",
            PropertyType::String,
            true,
            true,
        )],
        false,
        true,
    )
    .await;

    // Create first node
    let mut props1 = HashMap::new();
    props1.insert(
        "email".to_string(),
        PropertyValue::String("user@example.com".to_string()),
    );
    let node1 = create_test_node("user1", "test:User", props1);
    service.add_node("/", node1).await.unwrap();

    // Try to create second node with same email
    let mut props2 = HashMap::new();
    props2.insert(
        "email".to_string(),
        PropertyValue::String("user@example.com".to_string()),
    );
    let node2 = create_test_node("user2", "test:User", props2);

    // Should fail
    let result = service.add_node("/", node2).await;
    assert!(result.is_err(), "Should fail due to unique constraint");
    assert!(result.unwrap_err().to_string().contains("must be unique"));
}

#[tokio::test]
async fn test_prevent_nodetype_change() {
    let storage = setup_storage();
    let service = NodeService::new(storage.clone());

    // Create two NodeTypes
    create_test_node_type(&*storage, "test:TypeA", vec![], false, true).await;
    create_test_node_type(&*storage, "test:TypeB", vec![], false, true).await;

    // Create node with TypeA
    let node = create_test_node("test-node", "test:TypeA", HashMap::new());
    let created = service.add_node("/", node).await.unwrap();

    // Try to update with different NodeType
    let mut updated_node = created.clone();
    updated_node.node_type = "test:TypeB".to_string();

    // Should fail
    let result = service.put(updated_node).await;
    assert!(result.is_err(), "Should prevent NodeType changes");
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Cannot change node_type"));
}

#[tokio::test]
async fn test_initial_structure_auto_creation() {
    let storage = setup_storage();
    let service = NodeService::new(storage.clone());

    // Create child NodeType
    create_test_node_type(&*storage, "test:File", vec![], false, true).await;

    // Create parent NodeType with initial_structure
    let initial_structure = InitialNodeStructure {
        properties: None,
        children: Some(vec![
            InitialChild {
                name: "README.md".to_string(),
                node_type: "test:File".to_string(),
                archetype: Some("text/markdown".to_string()),
                properties: None,
                translations: None,
                children: None,
            },
            InitialChild {
                name: "index.html".to_string(),
                node_type: "test:File".to_string(),
                archetype: Some("text/html".to_string()),
                properties: None,
                translations: None,
                children: None,
            },
        ]),
    };

    let folder_type = NodeType {
        id: Some("test:Folder".to_string()),
        strict: Some(false),
        name: "test:Folder".to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: Some(initial_structure),
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
            BranchScope::new("default", "default", "main"),
            folder_type,
            CommitMetadata::system("create folder type"),
        )
        .await
        .unwrap();

    // Create folder node
    let folder = create_test_node("my-folder", "test:Folder", HashMap::new());
    let created_folder = service.add_node("/", folder).await.unwrap();

    // Verify children were auto-created
    let children = service.list_children(&created_folder.path).await.unwrap();
    assert_eq!(
        children.len(),
        2,
        "Should auto-create 2 children from initial_structure"
    );

    // Names are sanitized (dots removed)
    let has_readme = children
        .iter()
        .any(|c| c.name == "readmemd" && c.node_type == "test:File");
    let has_index = children
        .iter()
        .any(|c| c.name == "indexhtml" && c.node_type == "test:File");

    assert!(has_readme, "Should have created README child");
    assert!(has_index, "Should have created index child");
}

#[tokio::test]
async fn test_nested_initial_structure() {
    let storage = setup_storage();
    let service = NodeService::new(storage.clone());

    // Create child NodeTypes
    create_test_node_type(&*storage, "test:Folder", vec![], false, true).await;
    create_test_node_type(&*storage, "test:File", vec![], false, true).await;

    // Create parent NodeType with nested initial_structure
    let initial_structure = InitialNodeStructure {
        properties: None,
        children: Some(vec![InitialChild {
            name: "src".to_string(),
            node_type: "test:Folder".to_string(),
            archetype: None,
            properties: None,
            translations: None,
            children: Some(vec![InitialChild {
                name: "main.rs".to_string(),
                node_type: "test:File".to_string(),
                archetype: Some("text/rust".to_string()),
                properties: None,
                translations: None,
                children: None,
            }]),
        }]),
    };

    let project_type = NodeType {
        id: Some("test:Project".to_string()),
        strict: Some(false),
        name: "test:Project".to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: Some(initial_structure),
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
            BranchScope::new("default", "default", "main"),
            project_type,
            CommitMetadata::system("create project node type"),
        )
        .await
        .unwrap();

    // Create project node
    let project = create_test_node("my-project", "test:Project", HashMap::new());
    let created_project = service.add_node("/", project).await.unwrap();

    // Verify top-level children
    let children = service.list_children(&created_project.path).await.unwrap();
    assert_eq!(children.len(), 1, "Should have one top-level child");

    let src_folder = &children[0];
    assert_eq!(src_folder.name, "src");

    // Verify nested children
    let nested_children = service.list_children(&src_folder.path).await.unwrap();
    assert_eq!(nested_children.len(), 1, "Should have nested child");

    let main_file = &nested_children[0];
    assert_eq!(main_file.name, "mainrs"); // sanitized
    assert_eq!(main_file.node_type, "test:File");
}

#[tokio::test]
async fn test_initial_structure_with_transaction_api() {
    let storage = setup_storage();
    let service = NodeService::new(storage.clone());

    // Create child NodeType
    create_test_node_type(&*storage, "test:File", vec![], false, true).await;

    // Create parent NodeType with initial_structure
    let initial_structure = InitialNodeStructure {
        properties: None,
        children: Some(vec![
            InitialChild {
                name: "README.md".to_string(),
                node_type: "test:File".to_string(),
                archetype: Some("text/markdown".to_string()),
                properties: None,
                translations: None,
                children: None,
            },
            InitialChild {
                name: "LICENSE".to_string(),
                node_type: "test:File".to_string(),
                archetype: Some("text/plain".to_string()),
                properties: None,
                translations: None,
                children: None,
            },
        ]),
    };

    let folder_type = NodeType {
        id: Some("test:Folder".to_string()),
        strict: Some(false),
        name: "test:Folder".to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: Some(initial_structure),
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
            BranchScope::new("default", "default", "main"),
            folder_type,
            CommitMetadata::system("create folder type"),
        )
        .await
        .unwrap();

    // First, create a simple node without initial_structure to test basic Transaction API
    let mut simple_node = create_test_node("simple", "test:File", HashMap::new());
    simple_node.id = nanoid::nanoid!();
    simple_node.path = "/simple".to_string();
    simple_node.created_at = Some(chrono::Utc::now());

    let mut tx = service.transaction();
    tx.create(simple_node);
    tx.commit("Create simple node", "test-user").await.unwrap();

    // Verify the simple node was created
    let simple_created = service.get_by_path("/simple").await.unwrap();
    assert!(simple_created.is_some(), "Simple node should be created");

    // Now create folder node with initial_structure using Transaction API
    let mut folder = create_test_node("tx-folder", "test:Folder", HashMap::new());
    folder.id = nanoid::nanoid!();
    folder.path = "/tx-folder".to_string();
    folder.created_at = Some(chrono::Utc::now());

    let mut tx2 = service.transaction();
    tx2.create(folder);
    let result = tx2
        .commit("Create folder with initial structure", "test-user")
        .await;
    if let Err(e) = &result {
        eprintln!("Transaction commit failed: {:?}", e);
    }
    result.unwrap();

    // Verify the parent folder was created
    let created_folder = service.get_by_path("/tx-folder").await.unwrap();
    assert!(created_folder.is_some(), "Folder should be created");
    let created_folder = created_folder.unwrap();

    // Verify children were auto-created via Transaction API
    let children = service.list_children(&created_folder.path).await.unwrap();
    assert_eq!(
        children.len(),
        2,
        "Should auto-create 2 children from initial_structure via Transaction API"
    );

    // Names are sanitized (dots removed)
    let has_readme = children
        .iter()
        .any(|c| c.name == "readmemd" && c.node_type == "test:File");
    let has_license = children
        .iter()
        .any(|c| c.name == "license" && c.node_type == "test:File");

    assert!(
        has_readme,
        "Should have created README child via Transaction API"
    );
    assert!(
        has_license,
        "Should have created LICENSE child via Transaction API"
    );
}

#[tokio::test]
async fn test_nested_initial_structure_with_transaction_api() {
    let storage = setup_storage();
    let service = NodeService::new(storage.clone());

    // Create child NodeTypes
    create_test_node_type(&*storage, "test:Folder", vec![], false, true).await;
    create_test_node_type(&*storage, "test:File", vec![], false, true).await;

    // Create parent NodeType with nested initial_structure
    let initial_structure = InitialNodeStructure {
        properties: None,
        children: Some(vec![InitialChild {
            name: "docs".to_string(),
            node_type: "test:Folder".to_string(),
            archetype: None,
            properties: None,
            translations: None,
            children: Some(vec![
                InitialChild {
                    name: "guide.md".to_string(),
                    node_type: "test:File".to_string(),
                    archetype: Some("text/markdown".to_string()),
                    properties: None,
                    translations: None,
                    children: None,
                },
                InitialChild {
                    name: "api.md".to_string(),
                    node_type: "test:File".to_string(),
                    archetype: Some("text/markdown".to_string()),
                    properties: None,
                    translations: None,
                    children: None,
                },
            ]),
        }]),
    };

    let project_type = NodeType {
        id: Some("test:Project".to_string()),
        strict: Some(false),
        name: "test:Project".to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: Some(initial_structure),
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
            BranchScope::new("default", "default", "main"),
            project_type,
            CommitMetadata::system("create project node type"),
        )
        .await
        .unwrap();

    // Create project node using Transaction API
    let mut project = create_test_node("tx-project", "test:Project", HashMap::new());
    project.id = nanoid::nanoid!();
    project.path = "/tx-project".to_string();
    project.created_at = Some(chrono::Utc::now());

    let mut tx = service.transaction();
    tx.create(project);
    tx.commit("Create project with nested initial structure", "test-user")
        .await
        .unwrap();

    // Verify the parent project was created
    let created_project = service.get_by_path("/tx-project").await.unwrap();
    assert!(created_project.is_some(), "Project should be created");
    let created_project = created_project.unwrap();

    // Verify top-level children
    let children = service.list_children(&created_project.path).await.unwrap();
    assert_eq!(
        children.len(),
        1,
        "Should have one top-level child via Transaction API"
    );

    let docs_folder = &children[0];
    assert_eq!(docs_folder.name, "docs");

    // Verify nested children
    let nested_children = service.list_children(&docs_folder.path).await.unwrap();
    assert_eq!(
        nested_children.len(),
        2,
        "Should have 2 nested children via Transaction API"
    );

    // Verify both nested files were created
    let has_guide = nested_children
        .iter()
        .any(|c| c.name == "guidemd" && c.node_type == "test:File");
    let has_api = nested_children
        .iter()
        .any(|c| c.name == "apimd" && c.node_type == "test:File");

    assert!(
        has_guide,
        "Should have created guide.md via Transaction API"
    );
    assert!(has_api, "Should have created api.md via Transaction API");
}

#[tokio::test]
async fn test_workspace_isolation() {
    let storage = setup_storage();
    // Create two services scoped to different workspaces
    let service_ws1 = NodeService::new_with_context(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
        "ws1".to_string(),
    );
    let service_ws2 = NodeService::new_with_context(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
        "ws2".to_string(),
    );

    // Create same NodeType in two workspaces
    create_test_node_type(&*storage, "test:Article", vec![], false, true).await;
    create_test_node_type(&*storage, "test:Article", vec![], false, true).await;

    // Create node in ws1
    let node1 = create_test_node("article1", "test:Article", HashMap::new());
    service_ws1.add_node("/", node1).await.unwrap();

    // Create node in ws2
    let node2 = create_test_node("article2", "test:Article", HashMap::new());
    service_ws2.add_node("/", node2).await.unwrap();

    // Verify isolation
    let ws1_nodes = service_ws1.list_all().await.unwrap();
    let ws2_nodes = service_ws2.list_all().await.unwrap();

    assert_eq!(ws1_nodes.len(), 1);
    assert_eq!(ws2_nodes.len(), 1);
    assert_eq!(ws1_nodes[0].name, "article1");
    assert_eq!(ws2_nodes[0].name, "article2");
}

/// Authorship stamping (created_by/updated_by) + revision history listing.
///
/// Authorship is stamped at the RocksDB transaction layer (`put_node`) and
/// history is read from MVCC revisions, so this test builds a RocksDB-backed
/// store directly (the in-memory store retains no history and doesn't stamp
/// authorship).
#[tokio::test]
async fn test_authorship_and_revision_history() {
    use raisin_models::auth::AuthContext;
    use raisin_models::permissions::ResolvedPermissions;
    use raisin_models::workspace::Workspace;
    use raisin_storage::{
        BranchRepository, RepoScope, RepositoryManagementRepository, WorkspaceRepository,
    };

    // Resolved user context: distinct identity, full permissions (so RLS allows
    // the write while authorship still records the real user id).
    let user_ctx = |uid: &str| {
        AuthContext::for_user(uid.to_string()).with_permissions(ResolvedPermissions::system_admin())
    };

    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(dir.path()).unwrap());
    // Bootstrap the default repo / workspace / branch the NodeService writes against.
    storage
        .repository_management()
        .create_repository(
            "default",
            "default",
            raisin_context::RepositoryConfig::default(),
        )
        .await
        .unwrap();
    storage
        .workspaces()
        .put(
            RepoScope::new("default", "default"),
            Workspace::new("default".to_string()),
        )
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(
            "default",
            "default",
            "main",
            "test-user",
            None,
            None,
            false,
            false,
        )
        .await
        .unwrap();
    create_test_node_type(
        &*storage,
        "test:Doc",
        vec![create_property_schema(
            "title",
            PropertyType::String,
            true,
            false,
        )],
        false,
        true,
    )
    .await;

    // CREATE as user123
    let svc_create = NodeService::new(storage.clone()).with_auth(user_ctx("user123"));
    let mut props = HashMap::new();
    props.insert("title".to_string(), PropertyValue::String("v1".to_string()));
    let created = svc_create
        .add_node("/", create_test_node("doc", "test:Doc", props))
        .await
        .unwrap();
    let id = created.id.clone();

    // Authorship stamped on create.
    let svc_read = NodeService::new(storage.clone()).with_auth(AuthContext::system());
    let fetched = svc_read.get(&id).await.unwrap().expect("node exists");
    assert_eq!(
        fetched.created_by.as_deref(),
        Some("user123"),
        "created_by stamped on create"
    );
    assert_eq!(
        fetched.updated_by.as_deref(),
        Some("user123"),
        "updated_by stamped on create"
    );
    let created_at = fetched.created_at;
    assert!(created_at.is_some(), "created_at set on create");

    // UPDATE as user456
    let svc_update = NodeService::new(storage.clone()).with_auth(user_ctx("user456"));
    let mut to_update = svc_read.get(&id).await.unwrap().unwrap();
    to_update
        .properties
        .insert("title".to_string(), PropertyValue::String("v2".to_string()));
    svc_update.update_node(to_update).await.unwrap();

    let after = svc_read.get(&id).await.unwrap().unwrap();
    assert_eq!(
        after.created_by.as_deref(),
        Some("user123"),
        "created_by preserved on update"
    );
    assert_eq!(
        after.updated_by.as_deref(),
        Some("user456"),
        "updated_by reflects the updater"
    );
    assert_eq!(
        after.created_at, created_at,
        "created_at preserved across update"
    );

    // HISTORY lists revisions newest-first with per-revision authorship.
    let history = svc_read.history(&id, None).await.unwrap();
    assert!(
        history.len() >= 2,
        "expected >= 2 revisions, got {}",
        history.len()
    );
    assert_eq!(
        history[0].updated_by.as_deref(),
        Some("user456"),
        "newest revision authored by the updater"
    );
    assert!(!history[0].deleted, "newest revision is not a tombstone");
    assert_eq!(
        history.last().unwrap().updated_by.as_deref(),
        Some("user123"),
        "oldest revision authored by the creator"
    );

    // limit caps the result.
    let limited = svc_read.history(&id, Some(1)).await.unwrap();
    assert_eq!(limited.len(), 1, "limit caps the number of revisions");

    // update_property_by_path goes through the non-transactional repository
    // update path — it must still bump updated_at and preserve creation
    // metadata (stamped at the repository level, not the transaction layer).
    let before = svc_read.get(&id).await.unwrap().unwrap();
    let prev_updated_at = before
        .updated_at
        .expect("updated_at set before property update");
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    svc_read
        .update_property_by_path(
            &created.path,
            "title",
            PropertyValue::String("v3".to_string()),
        )
        .await
        .unwrap();
    let after_prop = svc_read.get(&id).await.unwrap().unwrap();
    assert!(
        after_prop
            .updated_at
            .expect("updated_at set after property update")
            > prev_updated_at,
        "update_property_by_path bumps updated_at"
    );
    assert_eq!(
        after_prop.created_at, created_at,
        "created_at preserved across property update"
    );
    assert_eq!(
        after_prop.created_by.as_deref(),
        Some("user123"),
        "created_by preserved across property update"
    );
}

/// Event-bus audit: a node write produces an audit-log entry via the
/// `AuditEventHandler` subscriber (the path SQL DML and functions also use,
/// since every write commits through the same event-emitting transaction layer).
/// Gated on the NodeType `auditable` flag.
#[tokio::test]
async fn test_event_bus_audit_logging() {
    use raisin_audit::{AuditRepository, InMemoryAuditRepo};
    use raisin_core::AuditEventHandler;
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::audit_log::AuditLogAction;
    use raisin_models::permissions::ResolvedPermissions;
    use raisin_models::workspace::Workspace;
    use raisin_storage::{
        BranchRepository, RepoScope, RepositoryManagementRepository, WorkspaceRepository,
    };

    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(dir.path()).unwrap());
    storage
        .repository_management()
        .create_repository(
            "default",
            "default",
            raisin_context::RepositoryConfig::default(),
        )
        .await
        .unwrap();
    storage
        .workspaces()
        .put(
            RepoScope::new("default", "default"),
            Workspace::new("default".to_string()),
        )
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(
            "default",
            "default",
            "main",
            "test-user",
            None,
            None,
            false,
            false,
        )
        .await
        .unwrap();

    // Register the global audit subscriber on the storage event bus — exactly
    // as main.rs does for the running server.
    let audit_repo = Arc::new(InMemoryAuditRepo::default());
    storage
        .event_bus()
        .subscribe(Arc::new(AuditEventHandler::new(
            audit_repo.clone(),
            storage.clone(),
        )));

    // Auditable NodeType.
    create_test_node_type_auditable(&*storage, "audit:Doc", true).await;
    // Non-auditable NodeType.
    create_test_node_type_auditable(&*storage, "plain:Doc", false).await;

    let svc = NodeService::new(storage.clone()).with_auth(
        AuthContext::for_user("alice").with_permissions(ResolvedPermissions::system_admin()),
    );

    // Write an auditable node.
    let audited = svc
        .add_node("/", create_test_node("doc1", "audit:Doc", HashMap::new()))
        .await
        .unwrap();
    // Write a non-auditable node.
    let plain = svc
        .add_node("/", create_test_node("doc2", "plain:Doc", HashMap::new()))
        .await
        .unwrap();

    // Event handlers run asynchronously (tokio::spawn) after commit, so poll briefly.
    let mut logs = Vec::new();
    for _ in 0..40 {
        logs = audit_repo.get_logs_by_node_id(&audited.id).await.unwrap();
        if !logs.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    assert_eq!(
        logs.len(),
        1,
        "auditable node should have exactly one audit entry"
    );
    assert_eq!(logs[0].action, AuditLogAction::Create);
    assert_eq!(
        logs[0].user_id.as_deref(),
        Some("alice"),
        "actor recorded from commit"
    );

    // Non-auditable node produces no audit entry.
    let plain_logs = audit_repo.get_logs_by_node_id(&plain.id).await.unwrap();
    assert!(
        plain_logs.is_empty(),
        "non-auditable node must not be audited"
    );
}

/// Helper: upsert a minimal NodeType with a chosen `auditable` flag.
async fn create_test_node_type_auditable<S: Storage>(storage: &S, name: &str, auditable: bool) {
    let node_type = NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: Some(vec![]),
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(true),
        auditable: Some(auditable),
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
            BranchScope::new("default", "default", "main"),
            node_type,
            CommitMetadata::system("create test node type"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn test_add_deep_node_creates_ancestors() {
    use raisin_models::auth::AuthContext;
    let storage = setup_storage();
    let service = NodeService::new(storage.clone()).with_auth(AuthContext::system());

    create_test_node_type(&*storage, "test:Folder", vec![], false, true).await;
    create_test_node_type(
        &*storage,
        "test:Article",
        vec![create_property_schema(
            "title",
            PropertyType::String,
            true,
            false,
        )],
        false,
        true,
    )
    .await;

    let mut props = HashMap::new();
    props.insert("title".to_string(), PropertyValue::String("Hi".to_string()));
    let node = create_test_node("leaf", "test:Article", props);

    // Parent path /reports/2026 does not exist yet -- it must be auto-created.
    let created = service
        .add_deep_node_typed("/reports/2026", node, "test:Folder")
        .await
        .expect("deep create should succeed");
    assert_eq!(created.path, "/reports/2026/leaf");

    // Ancestors are auto-created using the requested folder type.
    let reports = service
        .get_by_path("/reports")
        .await
        .unwrap()
        .expect("/reports should be auto-created");
    assert_eq!(reports.node_type, "test:Folder");
    let y2026 = service
        .get_by_path("/reports/2026")
        .await
        .unwrap()
        .expect("/reports/2026 should be auto-created");
    assert_eq!(y2026.node_type, "test:Folder");
}

#[tokio::test]
async fn test_upsert_deep_node_creates_then_updates_in_place() {
    use raisin_models::auth::AuthContext;
    let storage = setup_storage();
    let service = NodeService::new(storage.clone()).with_auth(AuthContext::system());

    create_test_node_type(&*storage, "test:Folder", vec![], false, true).await;
    create_test_node_type(
        &*storage,
        "test:Doc",
        vec![create_property_schema(
            "title",
            PropertyType::String,
            false,
            false,
        )],
        false,
        true,
    )
    .await;

    // First upsert: node + ancestors created.
    let mut props = HashMap::new();
    props.insert("title".to_string(), PropertyValue::String("v1".to_string()));
    let mut node = create_test_node("doc", "test:Doc", props);
    node.id = "doc-1".to_string();
    node.path = "/space/docs/doc".to_string();

    let first = service
        .upsert_deep_node("/space/docs", node, "test:Folder")
        .await
        .expect("first upsert should succeed");
    assert_eq!(first.path, "/space/docs/doc");

    // Second upsert at the same path: should update in place, reusing the id.
    let mut props2 = HashMap::new();
    props2.insert("title".to_string(), PropertyValue::String("v2".to_string()));
    let mut node2 = create_test_node("doc", "test:Doc", props2);
    node2.path = "/space/docs/doc".to_string();

    let second = service
        .upsert_deep_node("/space/docs", node2, "test:Folder")
        .await
        .expect("second upsert should succeed");
    assert_eq!(second.id, first.id, "upsert-by-path should reuse the id");

    let fetched = service
        .get_by_path("/space/docs/doc")
        .await
        .unwrap()
        .expect("node should exist");
    assert_eq!(
        fetched.properties.get("title"),
        Some(&PropertyValue::String("v2".to_string())),
        "second upsert should update the title in place"
    );
}

/// Data-integrity guard: creating a node under a non-existent parent must be
/// rejected (no orphan nodes). This locks in the behavior of the parent-existence
/// check after it was optimized from a full RLS-filtered node load to an O(1)
/// path-index existence lookup — the integrity assertion must be unchanged.
#[tokio::test]
async fn test_create_under_missing_parent_rejected() {
    use raisin_models::auth::AuthContext;
    use raisin_models::permissions::ResolvedPermissions;
    use raisin_models::workspace::Workspace;
    use raisin_storage::{
        BranchRepository, RepoScope, RepositoryManagementRepository, WorkspaceRepository,
    };

    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(dir.path()).unwrap());
    storage
        .repository_management()
        .create_repository(
            "default",
            "default",
            raisin_context::RepositoryConfig::default(),
        )
        .await
        .unwrap();
    storage
        .workspaces()
        .put(
            RepoScope::new("default", "default"),
            Workspace::new("default".to_string()),
        )
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(
            "default", "default", "main", "test", None, None, false, false,
        )
        .await
        .unwrap();
    create_test_node_type(&*storage, "test:Doc", vec![], false, true).await;

    let admin =
        AuthContext::for_user("admin").with_permissions(ResolvedPermissions::system_admin());
    let svc = NodeService::new(storage.clone()).with_auth(admin);

    // Negative: parent does not exist → must be rejected (no orphan created).
    let err = svc
        .add_node(
            "/does-not-exist",
            create_test_node("child", "test:Doc", HashMap::new()),
        )
        .await;
    assert!(
        err.is_err(),
        "creating under a non-existent parent must be rejected"
    );
    // And nothing was persisted under the bogus path.
    assert!(
        svc.get_by_path("/does-not-exist/child")
            .await
            .unwrap()
            .is_none(),
        "no orphan node should be created"
    );

    // Positive: create the parent, then the child succeeds.
    svc.add_node("/", create_test_node("folder", "test:Doc", HashMap::new()))
        .await
        .unwrap();
    svc.add_node(
        "/folder",
        create_test_node("child", "test:Doc", HashMap::new()),
    )
    .await
    .expect("creating under an existing parent must succeed");
    assert!(
        svc.get_by_path("/folder/child").await.unwrap().is_some(),
        "child under existing parent should be persisted"
    );
}
