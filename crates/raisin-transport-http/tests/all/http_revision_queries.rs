//! HTTP integration tests for revision-based queries (time-travel reads)
//!
//! Tests the `/api/repository/{repo}/{branch}/rev/{revision}/{ws}/` routes
//! to ensure deleted nodes are visible in old revisions and hierarchical
//! queries work correctly across different points in time.
//!
//! Run with: cargo test --test http_revision_queries --features store-rocks

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use raisin_models::nodes::{ChildrenField, Node, NodeWithChildren};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;

#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;

use raisin_models::StorageTimestamp;
use raisin_storage::{
    BranchRepository, RepositoryManagementRepository, Storage, WorkspaceRepository,
};

#[cfg(feature = "storage-rocksdb")]
type Store = RocksDBStorage;
#[cfg(not(feature = "storage-rocksdb"))]
type Store = InMemoryStorage;

async fn setup_test_environment() -> (tempfile::TempDir, Arc<Store>, axum::Router) {
    #[cfg(feature = "storage-rocksdb")]
    let temp_dir = tempfile::tempdir().unwrap();

    #[cfg(not(feature = "storage-rocksdb"))]
    let temp_dir = tempfile::tempdir().unwrap();

    #[cfg(feature = "storage-rocksdb")]
    let storage = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());

    #[cfg(not(feature = "storage-rocksdb"))]
    let storage = Arc::new(InMemoryStorage::default());

    // Create repository
    use raisin_context::RepositoryConfig;
    storage
        .repository_management()
        .create_repository("default", "test_repo", RepositoryConfig::default())
        .await
        .unwrap();

    // Create workspace
    use raisin_models::workspace::Workspace;
    let workspace_model = Workspace {
        name: "default".to_string(),
        description: Some("Test workspace".to_string()),
        allowed_node_types: vec![],
        allowed_root_node_types: vec![],
        depends_on: vec![],
        initial_structure: None,
        created_at: StorageTimestamp::now(),
        updated_at: Some(StorageTimestamp::now()),
        config: raisin_models::workspace::WorkspaceConfig::default(),
    };
    storage
        .workspaces()
        .put(
            raisin_storage::RepoScope::new("default", "test_repo"),
            workspace_model,
        )
        .await
        .unwrap();

    // Create main branch
    storage
        .branches()
        .create_branch(
            "default",
            "test_repo",
            "main",
            "system",
            None,
            None,
            false,
            false,
        )
        .await
        .unwrap();

    // Create router
    let router = raisin_transport_http::router(storage.clone());

    (temp_dir, storage, router)
}

/// Helper to create a node via POST /head/ endpoint with commit
async fn create_node_via_http(
    router: &axum::Router,
    path: &str,
    node_name: &str,
    node_type: &str,
    parent: Option<String>,
) -> (u64, String) {
    let node_id = nanoid::nanoid!();
    let node_path = if let Some(ref parent_path) = parent {
        format!("{}/{}", parent_path.trim_end_matches('/'), node_name)
    } else {
        format!("/{}", node_name)
    };

    let mut node_data = json!({
        "id": node_id,
        "name": node_name,
        "node_type": node_type,
        "path": node_path,
        "properties": {
            "title": node_name,
            "content": format!("Content for {}", node_name)
        }
    });

    if let Some(parent_path) = parent {
        node_data["parent"] = json!(parent_path);
    }

    let request_body = json!({
        "node": node_data,
        "commit": {
            "message": format!("Create {}", node_name),
            "actor": "test_user"
        }
    });

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let revision = result["revision"].as_u64().unwrap();
    (revision, node_id)
}

/// Helper to delete a node via DELETE /head/ endpoint with commit
async fn delete_node_via_http(router: &axum::Router, path: &str, node_id: &str) -> u64 {
    let request_body = json!({
        "commit": {
            "message": format!("Delete node {}", node_id),
            "actor": "test_user"
        }
    });

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

    result["revision"].as_u64().unwrap()
}

#[tokio::test]
#[cfg(feature = "storage-rocksdb")]
async fn test_get_root_at_revision() {
    let (_temp_dir, _storage, router) = setup_test_environment().await;

    // Create three nodes at different revisions
    let (rev1, node_a_id) = create_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/",
        "node_a",
        "Article",
        None,
    )
    .await;

    let (rev2, node_b_id) = create_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/",
        "node_b",
        "Article",
        None,
    )
    .await;

    let (rev3, node_c_id) = create_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/",
        "node_c",
        "Article",
        None,
    )
    .await;

    println!("Created nodes: rev1={}, rev2={}, rev3={}", rev1, rev2, rev3);

    // Query at revision 1 - should see only node_a
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/",
                    rev1
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let nodes: Vec<Node> = serde_json::from_slice(&body).unwrap();

    assert_eq!(nodes.len(), 1, "Revision 1 should have 1 node");
    assert_eq!(nodes[0].id, node_a_id);

    // Query at revision 2 - should see node_a and node_b
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/",
                    rev2
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let nodes: Vec<Node> = serde_json::from_slice(&body).unwrap();

    assert_eq!(nodes.len(), 2, "Revision 2 should have 2 nodes");
    let node_ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(node_ids.contains(&node_a_id.as_str()));
    assert!(node_ids.contains(&node_b_id.as_str()));

    // Query at revision 3 - should see all three nodes
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/",
                    rev3
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let nodes: Vec<Node> = serde_json::from_slice(&body).unwrap();

    assert_eq!(nodes.len(), 3, "Revision 3 should have 3 nodes");
    let node_ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(node_ids.contains(&node_a_id.as_str()));
    assert!(node_ids.contains(&node_b_id.as_str()));
    assert!(node_ids.contains(&node_c_id.as_str()));
}

#[tokio::test]
#[cfg(feature = "storage-rocksdb")]
async fn test_deleted_node_visible_in_old_revision_via_http() {
    let (_temp_dir, _storage, router) = setup_test_environment().await;

    // Create node
    let (rev1, node_id) = create_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/",
        "test_node",
        "Article",
        None,
    )
    .await;

    // Delete node
    let rev2 = delete_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/test_node",
        &node_id,
    )
    .await;

    println!("Created at rev {}, deleted at rev {}", rev1, rev2);

    // Query at revision 1 (before deletion) - node should be visible
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/$ref/{}",
                    rev1, node_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Node should be visible at revision {} (before deletion)",
        rev1
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let node: Node = serde_json::from_slice(&body).unwrap();
    assert_eq!(node.id, node_id);
    assert_eq!(node.name, "test_node");

    // Query at revision 2 (after deletion) - node should NOT be visible
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/$ref/{}",
                    rev2, node_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Node should NOT be visible at revision {} (after deletion)",
        rev2
    );

    // Verify root listing at rev1 includes the node
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/",
                    rev1
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let nodes: Vec<Node> = serde_json::from_slice(&body).unwrap();

    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].id, node_id);

    // Verify root listing at rev2 does NOT include the node
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/",
                    rev2
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let nodes: Vec<Node> = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        nodes.len(),
        0,
        "Node should not appear in root listing after deletion"
    );
}

#[tokio::test]
#[cfg(feature = "storage-rocksdb")]
async fn test_hierarchical_queries_at_revision() {
    let (_temp_dir, _storage, router) = setup_test_environment().await;

    // Revision 1: Create folder
    let (rev1, _folder_id) = create_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/",
        "folder",
        "Folder",
        None,
    )
    .await;

    // Revision 2: Add child1
    let (rev2, child1_id) = create_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/folder",
        "child1",
        "Article",
        Some("/folder".to_string()),
    )
    .await;

    // Revision 3: Add child2
    let (rev3, child2_id) = create_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/folder",
        "child2",
        "Article",
        Some("/folder".to_string()),
    )
    .await;

    // Revision 4: Delete child1
    let rev4 = delete_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/folder/child1",
        &child1_id,
    )
    .await;

    println!(
        "Revisions: folder={}, +child1={}, +child2={}, -child1={}",
        rev1, rev2, rev3, rev4
    );

    // At revision 1: folder should have no children
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/folder/",
                    rev1
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let children: Vec<Node> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        children.len(),
        0,
        "Folder should have no children at revision 1"
    );

    // At revision 2: folder should have 1 child
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/folder/",
                    rev2
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let children: Vec<Node> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        children.len(),
        1,
        "Folder should have 1 child at revision 2"
    );
    assert_eq!(children[0].id, child1_id);

    // At revision 3: folder should have 2 children
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/folder/",
                    rev3
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let children: Vec<Node> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        children.len(),
        2,
        "Folder should have 2 children at revision 3"
    );
    let child_ids: Vec<&str> = children.iter().map(|n| n.id.as_str()).collect();
    assert!(child_ids.contains(&child1_id.as_str()));
    assert!(child_ids.contains(&child2_id.as_str()));

    // At revision 4: folder should have 1 child (child1 deleted)
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/folder/",
                    rev4
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let children: Vec<Node> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        children.len(),
        1,
        "Folder should have 1 child at revision 4"
    );
    assert_eq!(children[0].id, child2_id);
}

// TODO: This test requires deep_children_* methods to support revision queries
// Currently deep_children_array/flat/nested don't check self.revision
// See task #3 in Phase 5 todo list
#[tokio::test]
#[cfg(feature = "storage-rocksdb")]
async fn test_deep_query_at_revision() {
    let (_temp_dir, _storage, router) = setup_test_environment().await;

    // Create nested structure: folder -> subfolder -> item
    let (rev1, _) = create_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/",
        "folder",
        "Folder",
        None,
    )
    .await;

    let (rev2, _) = create_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/folder",
        "subfolder",
        "Folder",
        Some("/folder".to_string()),
    )
    .await;

    let (rev3, item_id) = create_node_via_http(
        &router,
        "/api/repository/test_repo/main/head/default/folder/subfolder",
        "item",
        "Article",
        Some("/folder/subfolder".to_string()),
    )
    .await;

    println!(
        "Created: folder={}, subfolder={}, item={}",
        rev1, rev2, rev3
    );

    // Deep query at revision 2 (before item was created) - level 2
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/folder/?level=2",
                    rev2
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    println!(
        "Response body at rev {}: {}",
        rev2,
        String::from_utf8_lossy(&body)
    );
    let nodes: Vec<NodeWithChildren> = serde_json::from_slice(&body).unwrap();

    // Should have subfolder but not item
    assert_eq!(nodes.len(), 1, "Should have only subfolder at revision 2");
    assert_eq!(nodes[0].node.name, "subfolder");

    // Deep query at revision 3 (after item was created) - level 2
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/repository/test_repo/main/rev/{}/default/folder/?level=2",
                    rev3
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let nodes: Vec<NodeWithChildren> = serde_json::from_slice(&body).unwrap();

    // Should have subfolder with nested item
    assert_eq!(nodes.len(), 1, "Should have subfolder at revision 3");
    assert_eq!(nodes[0].node.name, "subfolder");

    // subfolder should have item as child
    if let ChildrenField::Nodes(ref children) = nodes[0].children {
        assert_eq!(children.len(), 1, "subfolder should have 1 child");
        assert_eq!(children[0].node.name, "item");
        assert_eq!(children[0].node.id, item_id);
    } else {
        panic!("Expected Nodes variant of ChildrenField");
    }
}
