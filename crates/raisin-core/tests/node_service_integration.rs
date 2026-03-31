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
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};

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
            "default",
            "default",
            "main",
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
            "default",
            "default",
            "main",
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
            "default",
            "default",
            "main",
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
            "default",
            "default",
            "main",
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
            "default",
            "default",
            "main",
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
