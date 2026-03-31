//! Phase 4: Tree-Based Query Integration Tests
//!
//! These tests verify that the tree-based query system works correctly
//! with RaisinConnection at different revisions, solving the critical bug where
//! deleted nodes were invisible in old revisions.
//!
//! Run with: cargo test -p raisin-core --test phase4_tree_queries --features store-rocks

use raisin_core::RaisinConnection;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{
    BranchRepository, RepositoryManagementRepository, Storage, WorkspaceRepository,
};
use std::sync::Arc;

#[cfg(feature = "storage-rocksdb")]
use raisin_storage_rocks::RocksStorage;

#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;

/// Helper to create a minimal test node with all required fields
fn create_test_node(id: &str, name: &str, node_type: &str) -> Node {
    use std::collections::HashMap;

    let mut properties = HashMap::new();
    properties.insert("title".to_string(), PropertyValue::String(name.to_string()));
    properties.insert(
        "content".to_string(),
        PropertyValue::String(format!("Content for {}", name)),
    );

    Node {
        id: id.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        parent: None,
        path: format!("/{}", name),
        children: vec![],
        order_key: String::new(),
        has_children: None,
        properties,
        archetype: None,
        version: 1,
        published_at: None,
        published_by: None,
        created_at: None,
        updated_at: None,
        created_by: None,
        updated_by: None,
        translations: None,
        tenant_id: None,
        workspace: None,
        owner_id: None,
        relations: Vec::new(),
    }
}

#[tokio::test]
#[cfg(feature = "storage-rocksdb")]
async fn test_deleted_node_visible_in_old_revision() -> Result<()> {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksStorage::open(temp_dir.path())?);
    let connection = RaisinConnection::with_storage(storage.clone());

    // STEP 1: Create repository
    use raisin_context::RepositoryConfig;
    storage
        .repository_management()
        .create_repository("test_tenant", "test_repo", RepositoryConfig::default())
        .await?;

    // STEP 2: Create workspace
    use raisin_models::workspace::Workspace;
    let workspace_model = Workspace {
        name: "default".to_string(),
        description: Some("Test workspace".to_string()),
        allowed_node_types: vec![],
        allowed_root_node_types: vec![],
        depends_on: vec![],
        root_children: None,
        created_at: chrono::Utc::now(),
        updated_at: Some(chrono::Utc::now()),
        config: raisin_models::workspace::WorkspaceConfig::default(),
    };
    storage
        .workspaces()
        .put("test_tenant", "test_repo", workspace_model)
        .await?;

    // STEP 3: Create main branch
    storage
        .branches()
        .create_branch(
            "test_tenant",
            "test_repo",
            "main",
            "system",
            None,
            false,
            false,
        )
        .await?;

    // Now we can use the connection API
    let tenant = connection.tenant("test_tenant");
    let repo = tenant.repository("test_repo");
    let workspace = repo.workspace("default");

    // Revision 1: Create nodeA using transactional API
    let node_a = create_test_node("node_a", "node_a", "Article");
    let mut tx1 = workspace.nodes().transaction();
    tx1.create(node_a.clone());
    let rev1 = tx1.commit("Created node_a", "test_user").await?;

    println!("Revision 1: {} (created nodeA)", rev1);

    // Debug: Check if snapshot was created
    use raisin_storage::RevisionRepository;
    if let Some((actual_rev, snapshot)) = storage
        .revisions()
        .get_node_snapshot_at_or_before("test_tenant", "test_repo", &node_a.id, rev1)
        .await?
    {
        println!(
            "Found snapshot for node_a at revision {}, size: {} bytes",
            actual_rev,
            snapshot.len()
        );
    } else {
        println!(
            "NO SNAPSHOT found for node_a at or before revision {}!",
            rev1
        );
    }

    // Revision 2: Create nodeB
    let node_b = create_test_node("node_b", "node_b", "Article");
    let mut tx2 = workspace.nodes().transaction();
    tx2.create(node_b.clone());
    let rev2 = tx2.commit("Created node_b", "test_user").await?;

    println!("Revision 2: {} (created nodeB)", rev2);

    // Revision 3: Create nodeC
    let node_c = create_test_node("node_c", "node_c", "Article");
    let mut tx3 = workspace.nodes().transaction();
    tx3.create(node_c.clone());
    let rev3 = tx3.commit("Created node_c", "test_user").await?;

    println!("Revision 3: {} (created nodeC)", rev3);

    // Revision 4: Delete nodeA
    let mut tx4 = workspace.nodes().transaction();
    tx4.delete(node_a.id.clone());
    let rev4 = tx4.commit("Deleted node_a", "test_user").await?;

    println!("Revision 4: {} (deleted nodeA)", rev4);

    // CRITICAL TEST: Query at revision 1 - nodeA MUST be present
    println!("\n=== Testing revision queries ===");

    // Debug: Check what tree exists at revision 1
    use raisin_storage::TreeRepository;
    if let Some(root_tree_id) = storage
        .trees()
        .get_root_tree_id("test_tenant", "test_repo", rev1)
        .await?
    {
        println!("Root tree ID at revision {}: found", rev1);
        let entries = storage
            .trees()
            .iter_tree("test_tenant", "test_repo", &root_tree_id, None, 100)
            .await?;
        println!(
            "Tree entries at revision {}: {} entries",
            rev1,
            entries.len()
        );
        for entry in &entries {
            println!(
                "  - node_id: {}, node_type: {:?}",
                entry.node_id, entry.node_type
            );
        }
    } else {
        println!("NO ROOT TREE at revision {}!", rev1);
    }

    println!("Querying at revision {}", rev1);
    let nodes_at_rev1 = workspace.nodes().revision(rev1).list_root().await?;
    println!(
        "Nodes at revision {}: {:?}",
        rev1,
        nodes_at_rev1.iter().map(|n| &n.id).collect::<Vec<_>>()
    );

    assert_eq!(
        nodes_at_rev1.len(),
        1,
        "Revision 1 should have exactly 1 node"
    );
    assert!(
        nodes_at_rev1.iter().any(|n| n.id == node_a.id),
        "nodeA must be present at revision 1 (even though deleted at HEAD)"
    );

    // Verify we can get nodeA by ID at revision 1
    let retrieved_node_a = workspace
        .nodes()
        .revision(rev1)
        .get(&node_a.id)
        .await?
        .expect("nodeA should be retrievable at revision 1");
    assert_eq!(retrieved_node_a.id, node_a.id);
    assert_eq!(retrieved_node_a.name, "node_a");

    // Query at revision 2 - should have nodeA and nodeB
    let nodes_at_rev2 = workspace.nodes().revision(rev2).list_root().await?;
    println!(
        "Nodes at revision 2: {:?}",
        nodes_at_rev2.iter().map(|n| &n.id).collect::<Vec<_>>()
    );

    assert_eq!(
        nodes_at_rev2.len(),
        2,
        "Revision 2 should have exactly 2 nodes"
    );
    assert!(
        nodes_at_rev2.iter().any(|n| n.id == node_a.id),
        "nodeA must be present at revision 2"
    );
    assert!(
        nodes_at_rev2.iter().any(|n| n.id == node_b.id),
        "nodeB must be present at revision 2"
    );

    // Query at revision 3 - should have all three nodes
    let nodes_at_rev3 = workspace.nodes().revision(rev3).list_root().await?;
    println!(
        "Nodes at revision 3: {:?}",
        nodes_at_rev3.iter().map(|n| &n.id).collect::<Vec<_>>()
    );

    assert_eq!(
        nodes_at_rev3.len(),
        3,
        "Revision 3 should have exactly 3 nodes"
    );
    assert!(
        nodes_at_rev3.iter().any(|n| n.id == node_a.id),
        "nodeA must be present at revision 3"
    );
    assert!(
        nodes_at_rev3.iter().any(|n| n.id == node_b.id),
        "nodeB must be present at revision 3"
    );
    assert!(
        nodes_at_rev3.iter().any(|n| n.id == node_c.id),
        "nodeC must be present at revision 3"
    );

    // Query at revision 4 (after deletion) - nodeA should be gone
    let nodes_at_rev4 = workspace.nodes().revision(rev4).list_root().await?;
    println!(
        "Nodes at revision 4: {:?}",
        nodes_at_rev4.iter().map(|n| &n.id).collect::<Vec<_>>()
    );

    assert_eq!(
        nodes_at_rev4.len(),
        2,
        "Revision 4 should have exactly 2 nodes (nodeA deleted)"
    );
    assert!(
        !nodes_at_rev4.iter().any(|n| n.id == node_a.id),
        "nodeA must NOT be present at revision 4"
    );
    assert!(
        nodes_at_rev4.iter().any(|n| n.id == node_b.id),
        "nodeB must be present at revision 4"
    );
    assert!(
        nodes_at_rev4.iter().any(|n| n.id == node_c.id),
        "nodeC must be present at revision 4"
    );

    // Verify nodeA is not retrievable by ID at revision 4
    let missing_node = workspace.nodes().revision(rev4).get(&node_a.id).await?;
    assert!(
        missing_node.is_none(),
        "nodeA should not be retrievable at revision 4 (deleted)"
    );

    // Query at HEAD (current state) - only nodeB and nodeC should exist
    let nodes_at_head = workspace.nodes().list_root().await?;
    println!(
        "Nodes at HEAD: {:?}",
        nodes_at_head.iter().map(|n| &n.id).collect::<Vec<_>>()
    );

    assert_eq!(nodes_at_head.len(), 2, "HEAD should have exactly 2 nodes");
    assert!(
        !nodes_at_head.iter().any(|n| n.id == node_a.id),
        "nodeA must NOT be present at HEAD"
    );

    Ok(())
}

#[tokio::test]
#[cfg(feature = "storage-rocksdb")]
async fn test_tree_structure_changes_across_revisions() -> Result<()> {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksStorage::open(temp_dir.path())?);
    let connection = RaisinConnection::with_storage(storage.clone());

    // STEP 1: Create repository
    use raisin_context::RepositoryConfig;
    storage
        .repository_management()
        .create_repository("test_tenant", "test_repo", RepositoryConfig::default())
        .await?;

    // STEP 2: Create workspace
    use raisin_models::workspace::Workspace;
    let workspace_model = Workspace {
        name: "default".to_string(),
        description: Some("Test workspace".to_string()),
        allowed_node_types: vec![],
        allowed_root_node_types: vec![],
        depends_on: vec![],
        root_children: None,
        created_at: chrono::Utc::now(),
        updated_at: Some(chrono::Utc::now()),
        config: raisin_models::workspace::WorkspaceConfig::default(),
    };
    storage
        .workspaces()
        .put("test_tenant", "test_repo", workspace_model)
        .await?;

    // STEP 3: Create main branch
    storage
        .branches()
        .create_branch(
            "test_tenant",
            "test_repo",
            "main",
            "system",
            None,
            false,
            false,
        )
        .await?;

    // Now we can use the connection API
    let tenant = connection.tenant("test_tenant");
    let repo = tenant.repository("test_repo");
    let workspace = repo.workspace("default");

    // Revision 1: Create parent folder
    let parent = create_test_node("folder", "folder", "Folder");
    let mut tx1 = workspace.nodes().transaction();
    tx1.create(parent.clone());
    let rev1 = tx1.commit("Created folder", "test_user").await?;

    println!("Revision 1: {} (created folder)", rev1);

    // Revision 2: Add child1 to folder
    let mut child1 = create_test_node("child1", "child1", "Article");
    child1.parent = Some(parent.id.clone());
    child1.path = format!("{}/{}", parent.path, child1.name);
    let mut tx2 = workspace.nodes().transaction();
    tx2.create(child1.clone());
    let rev2 = tx2.commit("Added child1 to folder", "test_user").await?;

    println!("Revision 2: {} (added child1)", rev2);

    // Revision 3: Add child2 to folder
    let mut child2 = create_test_node("child2", "child2", "Article");
    child2.parent = Some(parent.id.clone());
    child2.path = format!("{}/{}", parent.path, child2.name);
    let mut tx3 = workspace.nodes().transaction();
    tx3.create(child2.clone());
    let rev3 = tx3.commit("Added child2 to folder", "test_user").await?;

    println!("Revision 3: {} (added child2)", rev3);

    // Revision 4: Delete child1
    let mut tx4 = workspace.nodes().transaction();
    tx4.delete(child1.id.clone());
    let rev4 = tx4.commit("Deleted child1", "test_user").await?;

    println!("Revision 4: {} (deleted child1)", rev4);

    // At revision 1: folder should have no children
    let children_at_rev1 = workspace
        .nodes()
        .revision(rev1)
        .list_by_parent(&parent.id)
        .await?;
    assert_eq!(
        children_at_rev1.len(),
        0,
        "Folder should have no children at revision 1"
    );

    // At revision 2: folder should have 1 child
    let children_at_rev2 = workspace
        .nodes()
        .revision(rev2)
        .list_by_parent(&parent.id)
        .await?;
    assert_eq!(
        children_at_rev2.len(),
        1,
        "Folder should have 1 child at revision 2"
    );
    assert!(
        children_at_rev2.iter().any(|n| n.id == child1.id),
        "Should have child1"
    );

    // At revision 3: folder should have 2 children
    let children_at_rev3 = workspace
        .nodes()
        .revision(rev3)
        .list_by_parent(&parent.id)
        .await?;
    assert_eq!(
        children_at_rev3.len(),
        2,
        "Folder should have 2 children at revision 3"
    );
    assert!(
        children_at_rev3.iter().any(|n| n.id == child1.id),
        "Should have child1"
    );
    assert!(
        children_at_rev3.iter().any(|n| n.id == child2.id),
        "Should have child2"
    );

    // At revision 4: folder should have 1 child (child1 deleted)
    let children_at_rev4 = workspace
        .nodes()
        .revision(rev4)
        .list_by_parent(&parent.id)
        .await?;
    assert_eq!(
        children_at_rev4.len(),
        1,
        "Folder should have 1 child at revision 4"
    );
    assert!(
        !children_at_rev4.iter().any(|n| n.id == child1.id),
        "Should NOT have child1"
    );
    assert!(
        children_at_rev4.iter().any(|n| n.id == child2.id),
        "Should have child2"
    );

    Ok(())
}
