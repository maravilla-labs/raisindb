//! Integration tests for snapshot branch functionality
//!
//! These tests verify that:
//! - Branches can be created from specific revisions
//! - Listing children at snapshot revisions works correctly
//! - Nested paths work on snapshot branches
//! - has_children is populated correctly at snapshot revisions

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use serde_json::json;
use tower::ServiceExt;

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
#[cfg(feature = "storage-rocksdb")]
use std::sync::Arc;

#[cfg(feature = "storage-rocksdb")]
use raisin_transport_http::router;

/// Helper to create a test router with RocksDB storage
#[cfg(feature = "storage-rocksdb")]
fn create_test_router() -> Router {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = format!("/tmp/raisin-test-snapshot-branches-{}", id);
    let _ = std::fs::remove_dir_all(&path);
    let storage = Arc::new(RocksDBStorage::new(&path).expect("Failed to create RocksDBStorage"));
    router(storage)
}

/// Helper to parse JSON response
async fn parse_json_response<T: serde::de::DeserializeOwned>(
    response: axum::response::Response,
) -> T {
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read response body");
    serde_json::from_slice(&body_bytes).expect("Failed to parse JSON response")
}

/// Helper to create a node
async fn create_node(
    app: Router,
    repo: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
    name: &str,
    node_type: &str,
) -> (Router, serde_json::Value) {
    let request_body = json!({
        "name": name,
        "node_type": node_type,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!(
                    "/api/repository/{}/{}/{}/{}",
                    repo, branch, workspace, parent_path
                ))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let node: serde_json::Value = parse_json_response(response).await;
    (app, node)
}

/// Helper to get branch info
async fn get_branch(app: Router, repo: &str, branch: &str) -> (Router, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!(
                    "/api/management/repositories/default/{}/branches/{}",
                    repo, branch
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let branch_info: serde_json::Value = parse_json_response(response).await;
    (app, branch_info)
}

/// Helper to create a branch from a revision
async fn create_branch_from_revision(
    app: Router,
    repo: &str,
    branch_name: &str,
    from_revision: u64,
) -> Router {
    let request_body = json!({
        "name": branch_name,
        "from_revision": from_revision,
        "created_by": "test-user",
        "protected": false
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!(
                    "/api/management/repositories/default/{}/branches",
                    repo
                ))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    app
}

#[tokio::test]
#[cfg(feature = "storage-rocksdb")]
async fn test_snapshot_branch_nested_listing() {
    // Create test environment
    let app = create_test_router();

    // 1. Create nodes on main branch
    let (app, folder) = create_node(
        app,
        "default",
        "main",
        "demo",
        "",
        "parent-folder",
        "raisin:Folder",
    )
    .await;

    let folder_path = folder["path"].as_str().unwrap();
    eprintln!("Created parent folder: {}", folder_path);

    // Create first child
    let (app, child1) = create_node(
        app,
        "default",
        "main",
        "demo",
        folder_path,
        "child1",
        "raisin:Page",
    )
    .await;
    eprintln!("Created child1: {}", child1["path"].as_str().unwrap());

    // Create second child
    let (app, child2) = create_node(
        app,
        "default",
        "main",
        "demo",
        folder_path,
        "child2",
        "raisin:Page",
    )
    .await;
    eprintln!("Created child2: {}", child2["path"].as_str().unwrap());

    // 2. Get current revision (should be after creating the 2 children)
    let (app, branch_info) = get_branch(app, "default", "main").await;
    let snapshot_revision = branch_info["head"].as_u64().unwrap();
    eprintln!("Snapshot revision: {}", snapshot_revision);

    assert!(
        snapshot_revision >= 3,
        "Expected at least 3 revisions (folder + 2 children), got {}",
        snapshot_revision
    );

    // 3. Create a third child AFTER the snapshot point
    let (app, child3) = create_node(
        app,
        "default",
        "main",
        "demo",
        folder_path,
        "child3",
        "raisin:Page",
    )
    .await;
    eprintln!(
        "Created child3 (after snapshot): {}",
        child3["path"].as_str().unwrap()
    );

    // 4. Verify main branch now has 3 children
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!(
                    "/api/repository/default/main/demo/{}/",
                    folder_path
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let main_children: Vec<serde_json::Value> = parse_json_response(response).await;
    eprintln!("Main branch children count: {}", main_children.len());
    assert_eq!(main_children.len(), 3, "Main branch should have 3 children");

    // 5. Create snapshot branch from the revision BEFORE child3 was added
    let app =
        create_branch_from_revision(app, "default", "feature-snapshot", snapshot_revision).await;
    eprintln!(
        "Created snapshot branch from revision {}",
        snapshot_revision
    );

    // 6. CRITICAL TEST: List children at the snapshot revision
    // This should return 2 children (child1 and child2), NOT 3
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!(
                    "/api/repository/default/feature-snapshot/rev/{}/demo/{}/",
                    snapshot_revision, folder_path
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let snapshot_children: Vec<serde_json::Value> = parse_json_response(response).await;

    eprintln!(
        "Snapshot branch children at rev {}: {}",
        snapshot_revision,
        snapshot_children.len()
    );
    eprintln!(
        "Children: {}",
        serde_json::to_string_pretty(&snapshot_children).unwrap()
    );

    assert_eq!(
        snapshot_children.len(),
        2,
        "Snapshot branch at revision {} should have 2 children (child1, child2), but got {}. Children: {:?}",
        snapshot_revision,
        snapshot_children.len(),
        snapshot_children.iter().map(|c| c["name"].as_str().unwrap_or("?")).collect::<Vec<_>>()
    );

    // 7. Verify the children have correct names
    let child_names: Vec<&str> = snapshot_children
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();

    assert!(
        child_names.contains(&"child1"),
        "Expected child1 in snapshot, got: {:?}",
        child_names
    );
    assert!(
        child_names.contains(&"child2"),
        "Expected child2 in snapshot, got: {:?}",
        child_names
    );

    // 8. Verify has_children is populated correctly
    for child in &snapshot_children {
        assert!(
            child["has_children"].is_boolean() || child["has_children"].is_null(),
            "has_children should be populated"
        );
    }

    // 9. Verify parent folder has has_children=true at the snapshot revision
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!(
                    "/api/repository/default/feature-snapshot/rev/{}/demo/{}",
                    snapshot_revision, folder_path
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let parent_at_snapshot: serde_json::Value = parse_json_response(response).await;

    assert_eq!(
        parent_at_snapshot["has_children"].as_bool(),
        Some(true),
        "Parent folder should have has_children=true at snapshot revision"
    );

    eprintln!("✅ All snapshot branch tests passed!");
}

#[tokio::test]
#[cfg(feature = "storage-rocksdb")]
async fn test_snapshot_branch_root_vs_nested() {
    // This test specifically verifies that BOTH root and nested paths work on snapshot branches

    let app = create_test_router();

    // 1. Create folder at root
    let (app, root_folder) = create_node(
        app,
        "default",
        "main",
        "demo",
        "",
        "root-folder",
        "raisin:Folder",
    )
    .await;

    let root_folder_path = root_folder["path"].as_str().unwrap();

    // 2. Create nested folder inside root folder
    let (app, nested_folder) = create_node(
        app,
        "default",
        "main",
        "demo",
        root_folder_path,
        "nested-folder",
        "raisin:Folder",
    )
    .await;

    let nested_folder_path = nested_folder["path"].as_str().unwrap();

    // 3. Add children to nested folder
    let (app, _) = create_node(
        app,
        "default",
        "main",
        "demo",
        nested_folder_path,
        "deep-child1",
        "raisin:Page",
    )
    .await;

    let (app, _) = create_node(
        app,
        "default",
        "main",
        "demo",
        nested_folder_path,
        "deep-child2",
        "raisin:Page",
    )
    .await;

    // 4. Get snapshot revision
    let (app, branch_info) = get_branch(app, "default", "main").await;
    let snapshot_revision = branch_info["head"].as_u64().unwrap();

    // 5. Create snapshot branch
    let app = create_branch_from_revision(app, "default", "test-snapshot", snapshot_revision).await;

    // 6. TEST ROOT LEVEL - should work
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!(
                    "/api/repository/default/test-snapshot/rev/{}/demo/",
                    snapshot_revision
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let root_children: Vec<serde_json::Value> = parse_json_response(response).await;
    assert_eq!(
        root_children.len(),
        1,
        "Root should have 1 child (root-folder)"
    );

    // 7. TEST NESTED LEVEL (one level deep) - should work
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!(
                    "/api/repository/default/test-snapshot/rev/{}/demo/{}/",
                    snapshot_revision, root_folder_path
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let nested_children: Vec<serde_json::Value> = parse_json_response(response).await;
    eprintln!(
        "Nested folder children: {}",
        serde_json::to_string_pretty(&nested_children).unwrap()
    );
    assert_eq!(
        nested_children.len(),
        1,
        "Nested folder should have 1 child (nested-folder), got: {:?}",
        nested_children
            .iter()
            .map(|c| c["name"].as_str())
            .collect::<Vec<_>>()
    );

    // 8. TEST DEEP NESTED LEVEL (two levels deep) - should work
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!(
                    "/api/repository/default/test-snapshot/rev/{}/demo/{}/",
                    snapshot_revision, nested_folder_path
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let deep_children: Vec<serde_json::Value> = parse_json_response(response).await;
    eprintln!(
        "Deep nested children: {}",
        serde_json::to_string_pretty(&deep_children).unwrap()
    );
    assert_eq!(
        deep_children.len(),
        2,
        "Deep nested folder should have 2 children (deep-child1, deep-child2), got: {:?}",
        deep_children
            .iter()
            .map(|c| c["name"].as_str())
            .collect::<Vec<_>>()
    );

    eprintln!("✅ Root vs nested snapshot tests passed!");
}
