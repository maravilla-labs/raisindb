//! Integration test for indexed property validation
//!
//! This test demonstrates the performance improvement from using IndexManager
//! with PropertyIndexPlugin for unique property validation.

use raisin_core::services::node_validation::NodeValidator;
use raisin_events::{Event, EventBus, EventHandler, InMemoryEventBus, NodeEvent, NodeEventKind};
use raisin_indexer::{IndexManager, PropertyIndexPlugin};
use raisin_models::nodes::properties::schema::{PropertyType, PropertyValueSchema};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::NodeType;
use raisin_models::nodes::Node;
use raisin_storage::{
    BranchScope, CommitMetadata, NodeRepository, NodeTypeRepository, Storage, StorageScope,
};
use raisin_storage_memory::InMemoryStorage;
use std::collections::HashMap;
use std::sync::Arc;

async fn setup_test_storage() -> Arc<InMemoryStorage> {
    Arc::new(InMemoryStorage::default())
}

async fn create_node_type_with_unique_property(storage: &InMemoryStorage, name: &str) {
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
        properties: Some(vec![PropertyValueSchema {
            name: Some("email".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            unique: Some(true), // Unique property
            default: None,
            constraints: None,
            structure: None,
            items: None,
            value: None,
            meta: None,
            is_translatable: None,
            allow_additional_properties: None,
            index: None,
        }]),
        allowed_children: Vec::new(),
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
            BranchScope::new("default", "default", "main"),
            node_type,
            CommitMetadata::system("create unique node type"),
        )
        .await
        .unwrap();
}

fn create_test_node(id: &str, email: &str) -> Node {
    let mut props = HashMap::new();
    props.insert(
        "email".to_string(),
        PropertyValue::String(email.to_string()),
    );

    Node {
        id: id.to_string(),
        name: format!("User {}", id),
        path: format!("/users/{}", id),
        node_type: "test:User".to_string(),
        archetype: None,
        properties: props,
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
        workspace: Some("ws1".to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

#[tokio::test]
async fn test_unique_validation_without_index() {
    let storage = setup_test_storage().await;
    create_node_type_with_unique_property(&storage, "test:User").await;

    // Create validator WITHOUT index manager
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    // Create first node
    let node1 = create_test_node("node1", "user@example.com");
    storage
        .nodes()
        .create(
            StorageScope::new("default", "default", "main", "ws1"),
            node1.clone(),
            raisin_storage::CreateNodeOptions::default(),
        )
        .await
        .unwrap();

    // First node should validate successfully
    assert!(validator.validate_node("ws1", &node1).await.is_ok());

    // Try to create second node with same email (should fail)
    let node2 = create_test_node("node2", "user@example.com");

    let result = validator.validate_node("ws1", &node2).await;
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("must be unique"));
}

#[tokio::test]
async fn test_unique_validation_with_index() {
    let storage = setup_test_storage().await;
    create_node_type_with_unique_property(&storage, "test:User").await;

    // Create index manager with property index plugin
    let index_manager = Arc::new(IndexManager::new());
    let property_index = Arc::new(PropertyIndexPlugin::new());
    index_manager.register_plugin(property_index.clone());

    // Create validator WITH index manager
    let validator = NodeValidator::with_index_manager(
        storage.clone(),
        index_manager.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    // Create first node and index it
    let node1 = create_test_node("node1", "user@example.com");
    storage
        .nodes()
        .create(
            StorageScope::new("default", "default", "main", "ws1"),
            node1.clone(),
            raisin_storage::CreateNodeOptions::default(),
        )
        .await
        .unwrap();

    // Manually trigger index update (simulating event bus)
    let mut node1_props = HashMap::new();
    node1_props.insert("email".to_string(), serde_json::json!("user@example.com"));
    let event1 = Event::Node(NodeEvent {
        tenant_id: "default".to_string(),
        repository_id: "default".to_string(),
        branch: "main".to_string(),
        workspace_id: "ws1".to_string(),
        node_id: "node1".to_string(),
        node_type: Some("test:User".to_string()),
        revision: raisin_hlc::HLC::new(1, 0),
        kind: NodeEventKind::Created,
        path: None,
        metadata: Some({
            let mut meta = HashMap::new();
            meta.insert("properties".to_string(), serde_json::json!(node1_props));
            meta
        }),
    });
    property_index.handle(&event1).await.unwrap();

    // First node should validate successfully
    assert!(validator.validate_node("ws1", &node1).await.is_ok());

    // Try to create second node with same email (should fail via index lookup)
    let node2 = create_test_node("node2", "user@example.com");

    let result = validator.validate_node("ws1", &node2).await;
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("must be unique"));
}

#[tokio::test]
async fn test_unique_validation_with_event_bus_integration() {
    let storage = setup_test_storage().await;
    create_node_type_with_unique_property(&storage, "test:User").await;

    // Create event bus and index manager
    let event_bus = Arc::new(InMemoryEventBus::new());
    let index_manager = Arc::new(IndexManager::new());
    let property_index = Arc::new(PropertyIndexPlugin::new());

    // Wire up: event bus -> property index plugin
    index_manager.register_plugin(property_index.clone());
    event_bus.subscribe(property_index);

    // Create validator with index
    let validator = NodeValidator::with_index_manager(
        storage.clone(),
        index_manager.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    // Create first node
    let node1 = create_test_node("node1", "user@example.com");
    storage
        .nodes()
        .create(
            StorageScope::new("default", "default", "main", "ws1"),
            node1.clone(),
            raisin_storage::CreateNodeOptions::default(),
        )
        .await
        .unwrap();

    // Publish node created event
    let mut props = HashMap::new();
    props.insert("email".to_string(), serde_json::json!("user@example.com"));
    event_bus.publish(Event::Node(NodeEvent {
        tenant_id: "default".to_string(),
        repository_id: "default".to_string(),
        branch: "main".to_string(),
        workspace_id: "ws1".to_string(),
        node_id: "node1".to_string(),
        node_type: Some("test:User".to_string()),
        revision: raisin_hlc::HLC::new(1, 0),
        kind: NodeEventKind::Created,
        path: None,
        metadata: Some({
            let mut meta = HashMap::new();
            meta.insert("properties".to_string(), serde_json::json!(props));
            meta
        }),
    }));

    // Give event handlers time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Validation should now use the index
    let node2 = create_test_node("node2", "user@example.com");
    let result = validator.validate_node("ws1", &node2).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_performance_comparison() {
    let storage = setup_test_storage().await;
    create_node_type_with_unique_property(&storage, "test:User").await;

    // Create 100 nodes to simulate a larger workspace
    for i in 0..100 {
        let node = create_test_node(&format!("node{}", i), &format!("user{}@example.com", i));
        storage
            .nodes()
            .create(
                StorageScope::new("default", "default", "main", "ws1"),
                node,
                raisin_storage::CreateNodeOptions::default(),
            )
            .await
            .unwrap();
    }

    // Test without index (O(n) scan)
    let validator_no_index = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );
    let test_node = create_test_node("test", "newuser@example.com");

    let start = std::time::Instant::now();
    let _ = validator_no_index.validate_node("ws1", &test_node).await;
    let duration_no_index = start.elapsed();

    // Test with index (O(1) lookup)
    let index_manager = Arc::new(IndexManager::new());
    let property_index = Arc::new(PropertyIndexPlugin::new());
    index_manager.register_plugin(property_index.clone());

    // Index all existing nodes
    for i in 0..100 {
        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            serde_json::json!(format!("user{}@example.com", i)),
        );
        let event = Event::Node(NodeEvent {
            tenant_id: "default".to_string(),
            repository_id: "default".to_string(),
            branch: "main".to_string(),
            workspace_id: "ws1".to_string(),
            node_id: format!("node{}", i),
            node_type: Some("test:User".to_string()),
            revision: raisin_hlc::HLC::new(1, 0),
            kind: NodeEventKind::Created,
            path: None,
            metadata: Some({
                let mut meta = HashMap::new();
                meta.insert("properties".to_string(), serde_json::json!(props));
                meta
            }),
        });
        property_index.handle(&event).await.unwrap();
    }

    let validator_with_index = NodeValidator::with_index_manager(
        storage.clone(),
        index_manager.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    let start = std::time::Instant::now();
    let _ = validator_with_index.validate_node("ws1", &test_node).await;
    let duration_with_index = start.elapsed();

    println!("Without index (O(n)): {:?}", duration_no_index);
    println!("With index (O(1)):    {:?}", duration_with_index);

    // Index should be faster (though with small datasets, overhead might dominate)
    // This is more about demonstrating the pattern than strict performance testing
}
