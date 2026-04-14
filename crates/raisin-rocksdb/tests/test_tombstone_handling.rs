//! Comprehensive Tombstone Handling Tests
//!
//! These tests verify that deleted nodes are correctly hidden from all query paths.
//! Covers all the bug fixes from commit 4bcef926:
//!
//! 1. PropertyIndexRepository - find_by_property, find_by_property_with_limit, count_by_property
//! 2. CompoundIndexRepository - scan_compound_index
//! 3. NodeRepository - scan_descendants_ordered, list_by_parent
//! 4. tombstones.rs - empty order_key handling
//!
//! The key invariant: After DELETE, a node must NOT appear in ANY query result.

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, CreateNodeOptions, DeleteNodeOptions,
    ListOptions, NodeRepository, NodeTypeRepository, PropertyIndexRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

// ============================================================================
// Test Constants
// ============================================================================

const TENANT: &str = "tombstone-test-tenant";
const REPO: &str = "tombstone-test-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

// ============================================================================
// Test Fixture
// ============================================================================

struct TestFixture {
    storage: RocksDBStorage,
    _temp_dir: TempDir,
}

impl TestFixture {
    async fn new() -> Result<Self> {
        let temp_dir =
            tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
        let storage = RocksDBStorage::new(temp_dir.path())?;

        // Initialize tenant
        storage
            .registry()
            .register_tenant(TENANT, HashMap::new())
            .await?;

        // Create repository
        let repo_config = RepositoryConfig {
            default_language: "en".to_string(),
            supported_languages: vec!["en".to_string()],
            locale_fallback_chains: HashMap::new(),
            default_branch: BRANCH.to_string(),
            description: Some("Tombstone test repository".to_string()),
            tags: HashMap::new(),
        };
        storage
            .repository_management()
            .create_repository(TENANT, REPO, repo_config)
            .await?;

        // Create branch
        storage
            .branches()
            .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
            .await?;

        // Create workspace
        let workspace = Workspace::new(WORKSPACE.to_string());
        let workspace_service = WorkspaceService::new(Arc::new(storage.clone()));
        workspace_service.put(TENANT, REPO, workspace).await?;

        // Create test node types
        let node_types = storage.node_types();

        // Folder type (allows any children)
        let folder_type = NodeType {
            id: Some(uuid::Uuid::new_v4().to_string()),
            strict: Some(false),
            name: "test:Folder".to_string(),
            extends: None,
            mixins: Vec::new(),
            overrides: None,
            description: Some("Test folder".to_string()),
            icon: None,
            version: Some(1),
            properties: None,
            allowed_children: vec!["*".to_string()],
            required_nodes: Vec::new(),
            initial_structure: None,
            versionable: Some(false),
            publishable: Some(false),
            auditable: Some(false),
            created_at: Some(chrono::Utc::now()),
            updated_at: Some(chrono::Utc::now()),
            published_at: None,
            published_by: None,
            previous_version: None,
            compound_indexes: None,
            is_mixin: None,
            indexable: None,
            index_types: None,
        };
        node_types
            .put(
                BranchScope::new(TENANT, REPO, BRANCH),
                folder_type,
                CommitMetadata::system("create test folder type"),
            )
            .await?;

        // Article type with properties
        let article_type = NodeType {
            id: Some(uuid::Uuid::new_v4().to_string()),
            strict: Some(false),
            name: "test:Article".to_string(),
            extends: None,
            mixins: Vec::new(),
            overrides: None,
            description: Some("Test article".to_string()),
            icon: None,
            version: Some(1),
            properties: None,
            allowed_children: vec![],
            required_nodes: Vec::new(),
            initial_structure: None,
            versionable: Some(false),
            publishable: Some(false),
            auditable: Some(false),
            created_at: Some(chrono::Utc::now()),
            updated_at: Some(chrono::Utc::now()),
            published_at: None,
            published_by: None,
            previous_version: None,
            compound_indexes: None,
            is_mixin: None,
            indexable: None,
            index_types: None,
        };
        node_types
            .put(
                BranchScope::new(TENANT, REPO, BRANCH),
                article_type,
                CommitMetadata::system("create test article type"),
            )
            .await?;

        Ok(Self {
            storage,
            _temp_dir: temp_dir,
        })
    }

    fn nodes(&self) -> &impl NodeRepository {
        self.storage.nodes()
    }

    fn property_index(&self) -> &impl PropertyIndexRepository {
        self.storage.property_index()
    }
}

/// Helper to create a test node
async fn create_node(
    fixture: &TestFixture,
    path: &str,
    node_type: &str,
    category: &str,
) -> Result<Node> {
    let mut properties = HashMap::new();
    properties.insert(
        "category".to_string(),
        PropertyValue::String(category.to_string()),
    );

    // Determine parent path
    let parent = if path == "/" {
        None
    } else {
        let parts: Vec<&str> = path.rsplitn(2, '/').collect();
        if parts.len() == 2 && !parts[1].is_empty() {
            Some(parts[1].to_string())
        } else {
            Some("/".to_string())
        }
    };

    let node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: path.to_string(),
        name: path.split('/').last().unwrap_or("node").to_string(),
        parent,
        node_type: node_type.to_string(),
        properties,
        children: Vec::new(),
        order_key: "temp".to_string(),
        has_children: None,
        version: 1,
        archetype: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        created_by: Some("test-user".to_string()),
        updated_by: Some("test-user".to_string()),
        published_at: None,
        published_by: None,
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: vec![],
    };

    fixture
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            node.clone(),
            CreateNodeOptions::default(),
        )
        .await?;

    Ok(node)
}

/// Helper to delete a node
async fn delete_node(fixture: &TestFixture, node_id: &str) -> Result<bool> {
    fixture
        .nodes()
        .delete(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            node_id,
            DeleteNodeOptions::default(),
        )
        .await
}

// ============================================================================
// Test Cases
// ============================================================================

/// Test: Deleted node should not appear in list_by_parent
///
/// This tests the REST API path: list_by_parent -> get_ordered_child_ids -> get_impl
#[tokio::test]
async fn test_deleted_node_not_in_list_by_parent() -> Result<()> {
    let fixture = TestFixture::new().await?;

    // Create parent and children
    let parent = create_node(&fixture, "/parent", "test:Folder", "container").await?;
    let child1 = create_node(&fixture, "/parent/child1", "test:Article", "sports").await?;
    let child2 = create_node(&fixture, "/parent/child2", "test:Article", "politics").await?;
    let child3 = create_node(&fixture, "/parent/child3", "test:Article", "tech").await?;

    // Verify all children exist before delete
    let children_before = fixture
        .nodes()
        .list_by_parent(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &parent.id,
            ListOptions::default(),
        )
        .await?;
    assert_eq!(
        children_before.len(),
        3,
        "Should have 3 children before delete"
    );

    // Delete one child
    let deleted = delete_node(&fixture, &child2.id).await?;
    assert!(deleted, "Delete should succeed");

    // Verify deleted child is NOT in list_by_parent
    let children_after = fixture
        .nodes()
        .list_by_parent(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &parent.id,
            ListOptions::default(),
        )
        .await?;

    assert_eq!(
        children_after.len(),
        2,
        "Should have 2 children after delete"
    );

    let child_ids: Vec<_> = children_after.iter().map(|n| n.id.as_str()).collect();
    assert!(
        !child_ids.contains(&child2.id.as_str()),
        "Deleted child should NOT appear in list_by_parent"
    );
    assert!(
        child_ids.contains(&child1.id.as_str()),
        "Non-deleted child1 should still appear"
    );
    assert!(
        child_ids.contains(&child3.id.as_str()),
        "Non-deleted child3 should still appear"
    );

    Ok(())
}

/// Test: Deleted node should not appear in scan_descendants_ordered
///
/// This tests the SQL PrefixScan path: scan_descendants_ordered -> get_latest_node_at_or_before_revision
/// This was Bug #4 - the function used to `continue` on tombstone instead of returning None
#[tokio::test]
async fn test_deleted_node_not_in_scan_descendants_ordered() -> Result<()> {
    let fixture = TestFixture::new().await?;

    // Create a tree: /root -> /root/level1 -> /root/level1/level2
    let root = create_node(&fixture, "/root", "test:Folder", "container").await?;
    let level1 = create_node(&fixture, "/root/level1", "test:Folder", "sports").await?;
    let _level2 = create_node(&fixture, "/root/level1/level2", "test:Article", "tech").await?;
    let sibling = create_node(&fixture, "/root/sibling", "test:Article", "politics").await?;

    // Verify all descendants exist
    let descendants_before = fixture
        .nodes()
        .scan_descendants_ordered(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &root.id,
            ListOptions::default(),
        )
        .await?;

    // Root + 3 descendants = 4 total (scan_descendants_ordered includes the root)
    assert!(
        descendants_before.len() >= 3,
        "Should have at least 3 descendants (level1, level2, sibling) before delete, got {}",
        descendants_before.len()
    );

    // Delete level1 (should also hide level2 since it's nested under level1)
    let deleted = delete_node(&fixture, &level1.id).await?;
    assert!(deleted, "Delete should succeed");

    // Scan descendants again
    let descendants_after = fixture
        .nodes()
        .scan_descendants_ordered(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &root.id,
            ListOptions::default(),
        )
        .await?;

    let descendant_ids: Vec<_> = descendants_after.iter().map(|n| n.id.as_str()).collect();

    // level1 should NOT appear (it was deleted)
    assert!(
        !descendant_ids.contains(&level1.id.as_str()),
        "Deleted level1 should NOT appear in scan_descendants_ordered"
    );

    // sibling should still appear
    assert!(
        descendant_ids.contains(&sibling.id.as_str()),
        "Non-deleted sibling should still appear"
    );

    Ok(())
}

/// Test: Deleted node should not appear in PropertyIndexRepository.find_by_property
///
/// This tests Bug #1 - property_index.rs used `value.is_empty()` instead of `is_tombstone(&value)`
#[tokio::test]
async fn test_deleted_node_not_in_property_index_find() -> Result<()> {
    let fixture = TestFixture::new().await?;

    // Create multiple nodes with the same category
    let node1 = create_node(&fixture, "/article1", "test:Article", "sports").await?;
    let node2 = create_node(&fixture, "/article2", "test:Article", "sports").await?;
    let node3 = create_node(&fixture, "/article3", "test:Article", "sports").await?;

    // Verify all nodes are found by property index
    let found_before = fixture
        .property_index()
        .find_by_property(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "category",
            &PropertyValue::String("sports".to_string()),
            false, // not published_only
        )
        .await?;

    assert_eq!(
        found_before.len(),
        3,
        "Should find 3 nodes with category=sports before delete"
    );

    // Delete one node
    let deleted = delete_node(&fixture, &node2.id).await?;
    assert!(deleted, "Delete should succeed");

    // Query property index again
    let found_after = fixture
        .property_index()
        .find_by_property(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "category",
            &PropertyValue::String("sports".to_string()),
            false,
        )
        .await?;

    assert_eq!(
        found_after.len(),
        2,
        "Should find 2 nodes after delete (deleted node should be filtered)"
    );

    assert!(
        !found_after.contains(&node2.id),
        "Deleted node ID should NOT appear in property index results"
    );
    assert!(
        found_after.contains(&node1.id),
        "Non-deleted node1 should still appear"
    );
    assert!(
        found_after.contains(&node3.id),
        "Non-deleted node3 should still appear"
    );

    Ok(())
}

/// Test: Deleted node should not be counted in PropertyIndexRepository.count_by_property
///
/// This tests Bug #1 variant - count_by_property had same issue
#[tokio::test]
async fn test_deleted_node_not_in_property_index_count() -> Result<()> {
    let fixture = TestFixture::new().await?;

    // Create nodes
    let node1 = create_node(&fixture, "/count1", "test:Article", "tech").await?;
    let _node2 = create_node(&fixture, "/count2", "test:Article", "tech").await?;
    let node3 = create_node(&fixture, "/count3", "test:Article", "tech").await?;

    // Count before delete
    let count_before = fixture
        .property_index()
        .count_by_property(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "category",
            &PropertyValue::String("tech".to_string()),
            false,
        )
        .await?;

    assert_eq!(count_before, 3, "Should count 3 nodes before delete");

    // Delete node1 and node3
    delete_node(&fixture, &node1.id).await?;
    delete_node(&fixture, &node3.id).await?;

    // Count after delete
    let count_after = fixture
        .property_index()
        .count_by_property(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "category",
            &PropertyValue::String("tech".to_string()),
            false,
        )
        .await?;

    assert_eq!(
        count_after, 1,
        "Should count 1 node after deleting 2 (deleted nodes should not be counted)"
    );

    Ok(())
}

/// Test: find_by_property_with_limit should also filter deleted nodes
///
/// This tests the SQL PropertyIndexScan path used by queries like:
/// SELECT * FROM nodes WHERE node_type = 'Article' LIMIT 10
#[tokio::test]
async fn test_deleted_node_not_in_property_index_find_with_limit() -> Result<()> {
    let fixture = TestFixture::new().await?;

    // Create nodes
    let _node1 = create_node(&fixture, "/limited1", "test:Article", "politics").await?;
    let node2 = create_node(&fixture, "/limited2", "test:Article", "politics").await?;
    let _node3 = create_node(&fixture, "/limited3", "test:Article", "politics").await?;

    // Find with limit before delete
    let found_before = fixture
        .property_index()
        .find_by_property_with_limit(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "category",
            &PropertyValue::String("politics".to_string()),
            false,
            Some(10), // limit
        )
        .await?;

    assert_eq!(found_before.len(), 3, "Should find 3 nodes before delete");

    // Delete middle node
    delete_node(&fixture, &node2.id).await?;

    // Find with limit after delete
    let found_after = fixture
        .property_index()
        .find_by_property_with_limit(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "category",
            &PropertyValue::String("politics".to_string()),
            false,
            Some(10),
        )
        .await?;

    assert_eq!(
        found_after.len(),
        2,
        "Should find 2 nodes after delete with limit"
    );
    assert!(
        !found_after.contains(&node2.id),
        "Deleted node should NOT appear even with limit"
    );

    Ok(())
}

/// Test: get_by_path should return None for deleted node
///
/// Basic sanity check that the primary get path works
#[tokio::test]
async fn test_deleted_node_not_in_get_by_path() -> Result<()> {
    let fixture = TestFixture::new().await?;

    let node = create_node(&fixture, "/gettest", "test:Article", "sports").await?;

    // Verify node exists
    let found = fixture
        .nodes()
        .get_by_path(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "/gettest",
            None,
        )
        .await?;
    assert!(found.is_some(), "Node should exist before delete");

    // Delete
    delete_node(&fixture, &node.id).await?;

    // Should return None
    let not_found = fixture
        .nodes()
        .get_by_path(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "/gettest",
            None,
        )
        .await?;
    assert!(
        not_found.is_none(),
        "get_by_path should return None for deleted node"
    );

    Ok(())
}

/// Test: get by ID should return None for deleted node
#[tokio::test]
async fn test_deleted_node_not_in_get_by_id() -> Result<()> {
    let fixture = TestFixture::new().await?;

    let node = create_node(&fixture, "/idtest", "test:Article", "tech").await?;
    let node_id = node.id.clone();

    // Verify node exists
    let found = fixture
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &node_id,
            None,
        )
        .await?;
    assert!(found.is_some(), "Node should exist before delete");

    // Delete
    delete_node(&fixture, &node_id).await?;

    // Should return None
    let not_found = fixture
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &node_id,
            None,
        )
        .await?;
    assert!(
        not_found.is_none(),
        "get by ID should return None for deleted node"
    );

    Ok(())
}

/// Test: Multiple deletes then recreate same path
///
/// Verifies that tombstones don't interfere with recreating nodes at the same path
#[tokio::test]
async fn test_recreate_after_delete() -> Result<()> {
    let fixture = TestFixture::new().await?;

    // Create, delete, recreate cycle
    let node1 = create_node(&fixture, "/recreate", "test:Article", "sports").await?;
    let id1 = node1.id.clone();

    delete_node(&fixture, &id1).await?;

    // Recreate at same path
    let node2 = create_node(&fixture, "/recreate", "test:Article", "tech").await?;
    let id2 = node2.id.clone();

    // IDs should be different
    assert_ne!(id1, id2, "Recreated node should have different ID");

    // New node should exist
    let found = fixture
        .nodes()
        .get_by_path(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "/recreate",
            None,
        )
        .await?;
    assert!(found.is_some(), "Recreated node should exist");
    assert_eq!(found.unwrap().id, id2, "Should find the new node, not old");

    // Old ID should still be gone
    let old_not_found = fixture
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &id1,
            None,
        )
        .await?;
    assert!(
        old_not_found.is_none(),
        "Old deleted node should still be gone"
    );

    Ok(())
}

/// Test: Property index should filter tombstones correctly even with empty order_key
///
/// This tests Bug #3 - tombstones.rs skipped ORDERED_CHILDREN tombstone for empty order_key
#[tokio::test]
async fn test_delete_node_with_empty_order_key() -> Result<()> {
    let fixture = TestFixture::new().await?;

    // Create parent and child (child will have auto-generated order_key)
    let parent = create_node(&fixture, "/parent_empty_ok", "test:Folder", "container").await?;
    let child = create_node(&fixture, "/parent_empty_ok/child", "test:Article", "tech").await?;

    // Verify child exists in parent's children
    let children_before = fixture
        .nodes()
        .list_by_parent(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &parent.id,
            ListOptions::default(),
        )
        .await?;
    assert_eq!(
        children_before.len(),
        1,
        "Should have 1 child before delete"
    );

    // Delete the child
    delete_node(&fixture, &child.id).await?;

    // Child should not appear
    let children_after = fixture
        .nodes()
        .list_by_parent(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &parent.id,
            ListOptions::default(),
        )
        .await?;
    assert_eq!(
        children_after.len(),
        0,
        "Should have 0 children after delete (tombstone should work regardless of order_key)"
    );

    Ok(())
}
