#![cfg(not(feature = "s3"))]
//! Comprehensive end-to-end integration test
//!
//! This test validates the complete workflow using the legacy POST API for node creation.
//! It covers: workspace creation, node CRUD with POST, listing, hierarchy navigation,
//! deep creation, rename, move, copy, delete, and validates the final state.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use raisin_models::nodes::types::NodeType;
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};
#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;
use raisin_transport_http as http;

async fn create_test_node_type<S: Storage>(storage: &S, name: &str) {
    let test_node_type = NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends: None,
        mixins: vec![],
        overrides: None,
        description: Some(format!("Test NodeType: {}", name)),
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec![],
        required_nodes: vec![],
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(true),
        auditable: Some(false),
        indexable: None,
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        compound_indexes: None,
        is_mixin: None,
        previous_version: None,
    };
    storage
        .node_types()
        .put(
            "test",
            "test",
            "main",
            test_node_type,
            CommitMetadata::system("test setup"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn complete_workflow_with_legacy_api() {
    // Setup test app
    let app = {
        #[cfg(feature = "storage-rocksdb")]
        {
            let path = "/tmp/raisin-rocks-test-e2e";
            let _ = std::fs::remove_dir_all(path);
            let store = Arc::new(RocksDBStorage::new(path).unwrap());
            create_test_node_type(&*store, "page").await;
            create_test_node_type(&*store, "raisin:Folder").await;
            raisin_transport_http::router(store)
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            let store = Arc::new(InMemoryStorage::default());
            create_test_node_type(&*store, "page").await;
            create_test_node_type(&*store, "raisin:Folder").await;
            raisin_transport_http::router(store)
        }
    };

    // ============================================================================
    // 1. Create workspace
    // ============================================================================
    eprintln!("\n=== Step 1: Create Workspace ===");
    let ws_body = serde_json::json!({
        "name": "demo",
        "description": "Demo workspace for E2E test",
        "allowed_node_types": ["page", "article", "folder"],
        "allowed_root_node_types": ["page", "article", "folder"],
        "depends_on": []
    });
    let req = Request::builder()
        .method("PUT")
        .uri("/workspaces/demo")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&ws_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "Failed to create workspace"
    );

    // ============================================================================
    // 2. POST to create root node "about"
    // ============================================================================
    eprintln!("=== Step 2: Create Root Node 'about' ===");
    let node_body = serde_json::json!({
        "name": "about",
        "node_type": "page",
        "properties": {"title": "About Us"}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/demo/")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Failed to create root node");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let about_node: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(about_node.name, "about");
    assert_eq!(about_node.path, "/about");

    // ============================================================================
    // 3. GET node from root
    // ============================================================================
    eprintln!("=== Step 3: GET Root Node ===");
    let req = Request::builder()
        .uri("/api/repository/demo/about")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Failed to get root node");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let node: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(node.path, "/about");

    // ============================================================================
    // 4. Create siblings at root: "services", "products", "contact"
    // ============================================================================
    eprintln!("=== Step 4: Create Siblings ===");
    for name in ["services", "products", "contact"] {
        let node_body = serde_json::json!({
            "name": name,
            "node_type": "page",
            "properties": {}
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/repository/demo/")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Failed to create sibling: {}",
            name
        );
    }

    // ============================================================================
    // 5. GET list root nodes (should have 4 nodes)
    // ============================================================================
    eprintln!("=== Step 5: List Root Nodes ===");
    let req = Request::builder()
        .uri("/api/repository/demo/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Failed to list root nodes");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let root_nodes: Vec<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(root_nodes.len(), 4, "Expected 4 root nodes");
    let root_names: Vec<&str> = root_nodes.iter().map(|n| n.name.as_str()).collect();
    assert!(root_names.contains(&"about"));
    assert!(root_names.contains(&"services"));
    assert!(root_names.contains(&"products"));
    assert!(root_names.contains(&"contact"));

    // ============================================================================
    // 6. Create children under "services"
    // ============================================================================
    eprintln!("=== Step 6: Create Children Under /services ===");
    for name in ["consulting", "development", "support"] {
        let node_body = serde_json::json!({
            "name": name,
            "node_type": "page",
            "properties": {}
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/repository/demo/services")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Failed to create child: {}",
            name
        );
    }

    // ============================================================================
    // 7. GET list children of /services (should have 3 children)
    // ============================================================================
    eprintln!("=== Step 7: List Children of /services ===");
    let req = Request::builder()
        .uri("/api/repository/demo/services/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Failed to list services children"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let service_children: Vec<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        service_children.len(),
        3,
        "Expected 3 children under services"
    );

    // ============================================================================
    // 8. Deep creation with auto-folder creation
    // ============================================================================
    eprintln!("=== Step 8: Deep Creation ===");
    let node_body = serde_json::json!({
        "name": "iphone",
        "node_type": "page",
        "properties": {}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/demo/products/electronics/phones?deep=true")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&node_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Failed deep creation");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let deep_node: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(deep_node.path, "/products/electronics/phones/iphone");

    // Verify auto-created folders
    for path in ["/products/electronics", "/products/electronics/phones"] {
        let req = Request::builder()
            .uri(format!("/api/repository/demo{}", path))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Auto-created folder {} not found",
            path
        );
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let folder: raisin_models::nodes::Node = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            folder.node_type, "raisin:Folder",
            "Expected folder type for {}",
            path
        );
    }

    // ============================================================================
    // 9. GET deep children with level parameter
    // ============================================================================
    eprintln!("=== Step 9: GET Deep Children with ?level=3 ===");
    let req = Request::builder()
        .uri("/api/repository/demo/products/?level=3")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Failed to get deep children");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let deep_map: std::collections::HashMap<String, raisin_models::nodes::DeepNode> =
        serde_json::from_slice(&bytes).unwrap();
    eprintln!("Deep map contains {} top-level nodes", deep_map.len());
    // The map contains immediate children with nested structure
    assert!(
        deep_map.len() >= 1,
        "Expected at least 1 node in deep hierarchy"
    );
    // Verify electronics folder exists and has nested children
    assert!(
        deep_map.contains_key("/products/electronics"),
        "Should contain electronics folder"
    );

    // ============================================================================
    // 10. GET deep children flattened
    // ============================================================================
    eprintln!("=== Step 10: GET Deep Children Flattened ===");
    let req = Request::builder()
        .uri("/api/repository/demo/products/?level=2&flatten=true")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Failed to get flattened deep children"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let flat_map: std::collections::HashMap<String, raisin_models::nodes::Node> =
        serde_json::from_slice(&bytes).unwrap();
    // Should contain electronics and phones (level 2)
    assert!(
        flat_map.len() >= 2,
        "Expected at least 2 nodes in flattened list"
    );

    // ============================================================================
    // 11. Rename "about" → "about-us"
    // ============================================================================
    eprintln!("=== Step 11: Rename /about → /about-us ===");
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/demo/about?command=rename")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"newName": "about-us"})).unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Failed to rename node");

    // Verify rename
    let req = Request::builder()
        .uri("/api/repository/demo/about-us")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Renamed node not found");

    // ============================================================================
    // 12. Move "contact" → "/services/contact"
    // ============================================================================
    eprintln!("=== Step 12: Move /contact → /services/contact ===");
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/demo/contact?command=move")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"targetPath": "/services/contact"})).unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Failed to move node");

    // Verify move
    let req = Request::builder()
        .uri("/api/repository/demo/services/contact")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Moved node not found at new location"
    );

    // ============================================================================
    // 13. Copy "consulting" → "/consulting-copy"
    // ============================================================================
    eprintln!("=== Step 13: Copy /services/consulting → /consulting-copy ===");
    let req = Request::builder()
        .method("POST")
        .uri("/api/repository/demo/services/consulting?command=copy")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "targetPath": "/",
                "newName": "consulting-copy"
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Failed to copy node");

    // Verify copy
    let req = Request::builder()
        .uri("/api/repository/demo/consulting-copy")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Copied node not found at /consulting-copy"
    );

    // ============================================================================
    // 14. Delete "products" tree
    // ============================================================================
    eprintln!("=== Step 14: Delete /products Tree ===");
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/repository/demo/products")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Failed to delete products tree"
    );

    // Verify deletion
    let req = Request::builder()
        .uri("/api/repository/demo/products")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Deleted node still exists"
    );

    // ============================================================================
    // 15. Verify final state
    // ============================================================================
    eprintln!("=== Step 15: Verify Final State ===");

    // List root - should have: about-us, services, consulting-copy
    let req = Request::builder()
        .uri("/api/repository/demo/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let final_root: Vec<raisin_models::nodes::Node> = serde_json::from_slice(&bytes).unwrap();
    let final_names: Vec<&str> = final_root.iter().map(|n| n.name.as_str()).collect();

    eprintln!("Final root nodes: {:?}", final_names);
    assert!(final_names.contains(&"about-us"), "about-us should exist");
    assert!(final_names.contains(&"services"), "services should exist");
    assert!(
        final_names.contains(&"consulting-copy"),
        "consulting-copy should exist"
    );
    assert!(
        !final_names.contains(&"products"),
        "products should be deleted"
    );
    assert!(!final_names.contains(&"contact"), "contact should be moved");

    // List services children - should have: consulting, development, support, contact
    let req = Request::builder()
        .uri("/api/repository/demo/services/")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let services_children: Vec<raisin_models::nodes::Node> =
        serde_json::from_slice(&bytes).unwrap();
    let services_names: Vec<&str> = services_children.iter().map(|n| n.name.as_str()).collect();

    eprintln!("Services children: {:?}", services_names);
    assert_eq!(
        services_children.len(),
        4,
        "Expected 4 children under services"
    );
    assert!(
        services_names.contains(&"consulting"),
        "consulting should exist"
    );
    assert!(
        services_names.contains(&"development"),
        "development should exist"
    );
    assert!(services_names.contains(&"support"), "support should exist");
    assert!(
        services_names.contains(&"contact"),
        "contact should be moved here"
    );

    eprintln!("\n✅ All E2E tests passed!");
}
