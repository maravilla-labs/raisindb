//! Tests for publish/unpublish and delete protection workflows

use std::collections::HashMap;
use std::sync::Arc;

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::NodeType;
use raisin_storage::{NodeTypeRepository, Storage, VersioningRepository};
use raisin_storage_memory::InMemoryStorage;

use super::NodeService;

/// Setup test service (versioning is now built into storage)
async fn setup_test_service_with_versioning() -> NodeService<InMemoryStorage> {
    let storage = Arc::new(InMemoryStorage::default());
    NodeService::new(storage).with_auth(AuthContext::system())
}

/// Create a simple test NodeType
async fn create_simple_node_type(storage: &InMemoryStorage, name: &str) {
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
        properties: None,
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
        .put(
            raisin_storage::scope::BranchScope::new("default", "default", "main"),
            node_type,
            raisin_storage::CommitMetadata::system("seed node type"),
        )
        .await
        .unwrap();
}

/// Create a test node
fn create_test_node(name: &str, node_type: &str) -> raisin_models::nodes::Node {
    raisin_models::nodes::Node {
        id: String::new(),
        name: name.to_string(),
        path: String::new(),
        node_type: node_type.to_string(),
        archetype: None,
        properties: HashMap::new(),
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
async fn test_publish_creates_version_first() {
    let service = setup_test_service_with_versioning().await;
    create_simple_node_type(&service.storage, "test:Page").await;

    // Create a draft node
    let node = create_test_node("my-page", "test:Page");
    let created = service.add_node("/", node).await.unwrap();

    // Modify the node to add some data
    let mut updated = created.clone();
    updated.properties.insert(
        "title".to_string(),
        PropertyValue::String("Draft Title".to_string()),
    );
    service.put(updated).await.unwrap();

    // Publish the node
    service.publish(&created.path).await.unwrap();

    // Get the published node
    let published = service.get_by_path(&created.path).await.unwrap().unwrap();
    assert!(published.published_at.is_some());
    assert_eq!(published.published_by, Some("system".to_string()));

    // Get the version created during publish
    // The version should have been created BEFORE publishing (draft state)
    let versions = service
        .storage
        .versioning()
        .list_versions(&created.id)
        .await
        .unwrap();

    // Should have at least one version from publish()
    assert!(!versions.is_empty(), "Publish should create a version");

    // Find the version created by publish (should be the latest one)
    let latest_version = versions.last().unwrap();

    // CRITICAL: The version should capture the DRAFT state (published_at = None)
    assert!(
        latest_version.node_data.published_at.is_none(),
        "Version should snapshot DRAFT state before publishing, not the published state"
    );
}

#[tokio::test]
async fn test_publish_tree_snapshots_all_nodes() {
    let service = setup_test_service_with_versioning().await;
    create_simple_node_type(&service.storage, "test:Folder").await;
    create_simple_node_type(&service.storage, "test:File").await;

    // Create a tree: root -> child1, child2
    let root = create_test_node("root", "test:Folder");
    let created_root = service.add_node("/", root).await.unwrap();

    let child1 = create_test_node("child1", "test:File");
    let created_child1 = service.add_node(&created_root.path, child1).await.unwrap();

    let child2 = create_test_node("child2", "test:File");
    let created_child2 = service.add_node(&created_root.path, child2).await.unwrap();

    // Publish the entire tree
    service.publish_tree(&created_root.path).await.unwrap();

    // Verify all nodes are published
    let root_published = service
        .get_by_path(&created_root.path)
        .await
        .unwrap()
        .unwrap();
    let child1_published = service
        .get_by_path(&created_child1.path)
        .await
        .unwrap()
        .unwrap();
    let child2_published = service
        .get_by_path(&created_child2.path)
        .await
        .unwrap()
        .unwrap();

    assert!(root_published.published_at.is_some());
    assert!(child1_published.published_at.is_some());
    assert!(child2_published.published_at.is_some());

    // Verify versions were created for all nodes
    let root_versions = service
        .storage
        .versioning()
        .list_versions(&created_root.id)
        .await
        .unwrap();
    let child1_versions = service
        .storage
        .versioning()
        .list_versions(&created_child1.id)
        .await
        .unwrap();
    let child2_versions = service
        .storage
        .versioning()
        .list_versions(&created_child2.id)
        .await
        .unwrap();

    assert!(!root_versions.is_empty());
    assert!(!child1_versions.is_empty());
    assert!(!child2_versions.is_empty());

    // All versions should have captured DRAFT state (published_at = None)
    assert!(root_versions
        .last()
        .unwrap()
        .node_data
        .published_at
        .is_none());
    assert!(child1_versions
        .last()
        .unwrap()
        .node_data
        .published_at
        .is_none());
    assert!(child2_versions
        .last()
        .unwrap()
        .node_data
        .published_at
        .is_none());
}

#[tokio::test]
async fn test_unpublish_does_not_create_version() {
    let service = setup_test_service_with_versioning().await;
    create_simple_node_type(&service.storage, "test:Page").await;

    // Create and publish a node
    let node = create_test_node("my-page", "test:Page");
    let created = service.add_node("/", node).await.unwrap();
    service.publish(&created.path).await.unwrap();

    // Get version count after publishing
    let versions_after_publish = service
        .storage
        .versioning()
        .list_versions(&created.id)
        .await
        .unwrap();
    let count_after_publish = versions_after_publish.len();

    // Unpublish the node
    service.unpublish(&created.path).await.unwrap();

    // Get version count after unpublishing
    let versions_after_unpublish = service
        .storage
        .versioning()
        .list_versions(&created.id)
        .await
        .unwrap();
    let count_after_unpublish = versions_after_unpublish.len();

    // Version count should NOT increase on unpublish
    assert_eq!(
        count_after_publish, count_after_unpublish,
        "Unpublish should NOT create a version"
    );

    // Verify the node is unpublished
    let unpublished = service.get_by_path(&created.path).await.unwrap().unwrap();
    assert!(unpublished.published_at.is_none());
}

#[tokio::test]
async fn test_delete_published_node_fails() {
    let service = setup_test_service_with_versioning().await;
    create_simple_node_type(&service.storage, "test:Page").await;

    // Create and publish a node
    let node = create_test_node("my-page", "test:Page");
    let created = service.add_node("/", node).await.unwrap();
    service.publish(&created.path).await.unwrap();

    // Try to delete the published node - should fail
    let result = service.delete_by_path(&created.path).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Cannot delete published node"));

    // Verify the node still exists
    let still_exists = service.get_by_path(&created.path).await.unwrap();
    assert!(still_exists.is_some());
}

#[tokio::test]
async fn test_delete_node_with_published_child_fails() {
    let service = setup_test_service_with_versioning().await;
    create_simple_node_type(&service.storage, "test:Folder").await;
    create_simple_node_type(&service.storage, "test:File").await;

    // Create a tree: root -> child
    let root = create_test_node("root", "test:Folder");
    let created_root = service.add_node("/", root).await.unwrap();

    let child = create_test_node("child", "test:File");
    let created_child = service.add_node(&created_root.path, child).await.unwrap();

    // Publish only the child
    service.publish(&created_child.path).await.unwrap();

    // Try to delete the root - should fail because child is published
    let result = service.delete_by_path(&created_root.path).await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("child"));
    assert!(err_msg.contains("is published"));
    assert!(err_msg.contains(&created_child.path));

    // Verify the root still exists
    let still_exists = service.get_by_path(&created_root.path).await.unwrap();
    assert!(still_exists.is_some());
}

#[tokio::test]
async fn test_delete_node_with_published_grandchild_fails() {
    let service = setup_test_service_with_versioning().await;
    create_simple_node_type(&service.storage, "test:Folder").await;
    create_simple_node_type(&service.storage, "test:File").await;

    // Create a tree: root -> child -> grandchild
    let root = create_test_node("root", "test:Folder");
    let created_root = service.add_node("/", root).await.unwrap();

    let child = create_test_node("child", "test:Folder");
    let created_child = service.add_node(&created_root.path, child).await.unwrap();

    let grandchild = create_test_node("grandchild", "test:File");
    let created_grandchild = service
        .add_node(&created_child.path, grandchild)
        .await
        .unwrap();

    // Publish only the grandchild
    service.publish(&created_grandchild.path).await.unwrap();

    // Try to delete the root - should fail because grandchild is published
    let result = service.delete_by_path(&created_root.path).await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("child"));
    assert!(err_msg.contains("is published"));
    assert!(err_msg.contains(&created_grandchild.path));

    // Verify the root still exists
    let still_exists = service.get_by_path(&created_root.path).await.unwrap();
    assert!(still_exists.is_some());
}

#[tokio::test]
#[ignore = "publish workflow will be deprecated"]
async fn test_delete_tree_after_unpublish_tree_succeeds() {
    let service = setup_test_service_with_versioning().await;
    create_simple_node_type(&service.storage, "test:Folder").await;
    create_simple_node_type(&service.storage, "test:File").await;

    // Create a tree: root -> child1, child2
    let root = create_test_node("root", "test:Folder");
    let created_root = service.add_node("/", root).await.unwrap();

    let child1 = create_test_node("child1", "test:File");
    let created_child1 = service.add_node(&created_root.path, child1).await.unwrap();

    let child2 = create_test_node("child2", "test:File");
    let _created_child2 = service.add_node(&created_root.path, child2).await.unwrap();

    // Publish the entire tree
    service.publish_tree(&created_root.path).await.unwrap();

    // Try to delete - should fail
    let result = service.delete_by_path(&created_root.path).await;
    assert!(result.is_err());

    // Unpublish the entire tree
    service.unpublish_tree(&created_root.path).await.unwrap();

    // Now deletion should succeed
    let result = service.delete_by_path(&created_root.path).await;
    assert!(result.is_ok());
    assert!(result.unwrap());

    // Verify all nodes are deleted
    assert!(service
        .get_by_path(&created_root.path)
        .await
        .unwrap()
        .is_none());
    assert!(service
        .get_by_path(&created_child1.path)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_delete_by_id_published_node_fails() {
    let service = setup_test_service_with_versioning().await;
    create_simple_node_type(&service.storage, "test:Page").await;

    // Create and publish a node
    let node = create_test_node("my-page", "test:Page");
    let created = service.add_node("/", node).await.unwrap();
    service.publish(&created.path).await.unwrap();

    // Try to delete by ID - should also fail
    let result = service.delete(&created.id).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Cannot delete published node"));

    // Verify the node still exists
    let still_exists = service.get(&created.id).await.unwrap();
    assert!(still_exists.is_some());
}

#[tokio::test]
async fn test_unpublish_tree_does_not_create_versions() {
    let service = setup_test_service_with_versioning().await;
    create_simple_node_type(&service.storage, "test:Folder").await;
    create_simple_node_type(&service.storage, "test:File").await;

    // Create a tree: root -> child1, child2
    let root = create_test_node("root", "test:Folder");
    let created_root = service.add_node("/", root).await.unwrap();

    let child1 = create_test_node("child1", "test:File");
    let created_child1 = service.add_node(&created_root.path, child1).await.unwrap();

    let child2 = create_test_node("child2", "test:File");
    let created_child2 = service.add_node(&created_root.path, child2).await.unwrap();

    // Publish the entire tree
    service.publish_tree(&created_root.path).await.unwrap();

    // Get version counts after publishing
    let root_versions_after_publish = service
        .storage
        .versioning()
        .list_versions(&created_root.id)
        .await
        .unwrap();
    let child1_versions_after_publish = service
        .storage
        .versioning()
        .list_versions(&created_child1.id)
        .await
        .unwrap();
    let child2_versions_after_publish = service
        .storage
        .versioning()
        .list_versions(&created_child2.id)
        .await
        .unwrap();

    // Unpublish the entire tree
    service.unpublish_tree(&created_root.path).await.unwrap();

    // Get version counts after unpublishing
    let root_versions_after_unpublish = service
        .storage
        .versioning()
        .list_versions(&created_root.id)
        .await
        .unwrap();
    let child1_versions_after_unpublish = service
        .storage
        .versioning()
        .list_versions(&created_child1.id)
        .await
        .unwrap();
    let child2_versions_after_unpublish = service
        .storage
        .versioning()
        .list_versions(&created_child2.id)
        .await
        .unwrap();

    // Version counts should NOT increase on unpublish_tree
    assert_eq!(
        root_versions_after_publish.len(),
        root_versions_after_unpublish.len()
    );
    assert_eq!(
        child1_versions_after_publish.len(),
        child1_versions_after_unpublish.len()
    );
    assert_eq!(
        child2_versions_after_publish.len(),
        child2_versions_after_unpublish.len()
    );

    // Verify all nodes are unpublished
    let root_unpublished = service
        .get_by_path(&created_root.path)
        .await
        .unwrap()
        .unwrap();
    let child1_unpublished = service
        .get_by_path(&created_child1.path)
        .await
        .unwrap()
        .unwrap();
    let child2_unpublished = service
        .get_by_path(&created_child2.path)
        .await
        .unwrap()
        .unwrap();

    assert!(root_unpublished.published_at.is_none());
    assert!(child1_unpublished.published_at.is_none());
    assert!(child2_unpublished.published_at.is_none());
}
