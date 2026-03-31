#![cfg(feature = "store-memory")]
//! Integration tests for audit log functionality
//!
//! Tests that audit logs are properly created and retrieved for node operations
//!
//! NOTE: Temporarily disabled (ignored) while we migrate audit logging to the
//! new git-like revision/event architecture. These tests compile but won't run
//! by default. Once the migration is complete, update routes/services and re-enable.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use raisin_audit::{AuditRepository, InMemoryAuditRepo};
use raisin_core::{init::init_global_nodetypes, NodeService, RepoAuditAdapter, WorkspaceService};
use raisin_models::{
    nodes::{
        audit_log::{AuditLog, AuditLogAction},
        Node,
    },
    workspace::Workspace,
};
use raisin_storage::{Storage, WorkspaceRepository};
use raisin_storage_memory::InMemoryStorage;
use raisin_transport_http as http;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use tower::ServiceExt;

async fn setup_test_service_with_audit() -> (
    axum::Router,
    Arc<NodeService<InMemoryStorage>>,
    Arc<InMemoryAuditRepo>,
) {
    let storage = Arc::new(InMemoryStorage::default());

    // Initialize global NodeTypes
    init_global_nodetypes(storage.clone())
        .await
        .expect("Failed to initialize global NodeTypes");

    // Create audit repository and adapter
    let audit_repo = Arc::new(InMemoryAuditRepo::default());
    let audit_adapter = Arc::new(RepoAuditAdapter::new(audit_repo.clone()));

    // Create NodeService with audit logging enabled
    let node_svc = Arc::new(NodeService::new(storage.clone()).with_audit(audit_adapter.clone()));
    let ws_svc = Arc::new(WorkspaceService::new(storage.clone()));

    // Create binary storage
    let bin = Arc::new(raisin_binary::FilesystemBinaryStorage::new(
        "./.test_uploads",
        Some("/files".into()),
    ));

    // Create the router
    let (app, _state) = raisin_transport_http::router_with_bin_and_audit(
        storage.clone(),
        ws_svc,
        bin,
        audit_repo.clone(),
        audit_adapter,
        false,
        false, // dev_mode
        &[],
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
    );

    (app, node_svc, audit_repo)
}

#[tokio::test]
#[ignore = "Pending migration to git-like revision/audit pipeline"]
async fn test_audit_log_created_on_node_creation() {
    let (app, _, audit_repo) = setup_test_service_with_audit().await;

    // Create a workspace
    let ws_body = json!({"name": "test-ws", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .uri("/workspaces/test-ws")
        .method("PUT")
        .header("content-type", "application/json")
        .body(Body::from(ws_body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Create a node
    let node_data = json!({
        "name": "test-folder",
        "node_type": "raisin:Folder",
        "properties": {}
    });

    let req = Request::builder()
        .uri("/api/repository/test-ws/")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(node_data.to_string()))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_node: Node = serde_json::from_slice(&body).unwrap();

    // Verify audit log was created
    let logs = audit_repo
        .get_logs_by_node_id(&created_node.id)
        .await
        .unwrap();

    assert_eq!(logs.len(), 1, "Expected 1 audit log for node creation");
    assert_eq!(logs[0].node_id, created_node.id);
    assert_eq!(logs[0].action, AuditLogAction::Update); // add_node calls put() which logs Update
    assert_eq!(logs[0].workspace, "test-ws");
}

#[tokio::test]
#[ignore = "Pending migration to git-like revision/audit pipeline"]
async fn test_audit_log_created_on_node_update() {
    let (app, _, audit_repo) = setup_test_service_with_audit().await;

    // Setup workspace
    let ws_body = json!({"name": "test-ws", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .uri("/workspaces/test-ws")
        .method("PUT")
        .header("content-type", "application/json")
        .body(Body::from(ws_body.to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    // Create a node
    let node_data = json!({
        "name": "test-page",
        "node_type": "raisin:Page",
        "properties": {"title": "Original Title"}
    });

    let req = Request::builder()
        .uri("/api/repository/test-ws/")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(node_data.to_string()))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_node: Node = serde_json::from_slice(&body).unwrap();

    // Update the node
    let update_data = json!({
        "properties": {"title": "Updated Title"}
    });

    let req = Request::builder()
        .uri(&format!("/api/repository/test-ws{}", created_node.path))
        .method("PUT")
        .header("content-type", "application/json")
        .body(Body::from(update_data.to_string()))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify audit logs (create + update)
    let logs = audit_repo
        .get_logs_by_node_id(&created_node.id)
        .await
        .unwrap();

    assert!(
        logs.len() >= 2,
        "Expected at least 2 audit logs (create + update)"
    );
    assert!(logs.iter().any(|log| log.action == AuditLogAction::Update));
}

#[tokio::test]
#[ignore = "Pending migration to git-like revision/audit pipeline"]
async fn test_audit_log_created_on_node_deletion() {
    let (app, _, audit_repo) = setup_test_service_with_audit().await;

    // Setup workspace
    let ws_body = json!({"name": "test-ws", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .uri("/workspaces/test-ws")
        .method("PUT")
        .header("content-type", "application/json")
        .body(Body::from(ws_body.to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    // Create a node
    let node_data = json!({
        "name": "to-delete",
        "node_type": "raisin:Folder",
        "properties": {}
    });

    let req = Request::builder()
        .uri("/api/repository/test-ws/")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(node_data.to_string()))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_node: Node = serde_json::from_slice(&body).unwrap();

    // Delete the node
    let req = Request::builder()
        .uri(&format!("/api/repository/test-ws{}", created_node.path))
        .method("DELETE")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify audit logs (create + delete)
    let logs = audit_repo
        .get_logs_by_node_id(&created_node.id)
        .await
        .unwrap();

    assert!(
        logs.len() >= 2,
        "Expected at least 2 audit logs (create + delete)"
    );
    assert!(logs.iter().any(|log| log.action == AuditLogAction::Delete));
}

#[tokio::test]
#[ignore = "Pending migration to git-like revision/audit pipeline"]
async fn test_audit_log_created_on_publish() {
    let (app, _, audit_repo) = setup_test_service_with_audit().await;

    // Setup workspace
    let ws_body = json!({"name": "test-ws", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .uri("/workspaces/test-ws")
        .method("PUT")
        .header("content-type", "application/json")
        .body(Body::from(ws_body.to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    // Create a node
    let node_data = json!({
        "name": "to-publish",
        "node_type": "raisin:Page",
        "properties": {"title": "Test Page"}
    });

    let req = Request::builder()
        .uri("/api/repository/test-ws/")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(node_data.to_string()))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    if status != StatusCode::OK {
        eprintln!(
            "Node creation failed with status {}: {}",
            status,
            String::from_utf8_lossy(&body)
        );
        panic!("Failed to create node");
    }
    let created_node: Node = serde_json::from_slice(&body).unwrap();

    // Publish the node
    let req = Request::builder()
        .uri(&format!(
            "/api/repository/test-ws{}/raisin:cmd/publish",
            created_node.path
        ))
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify audit logs (create + publish)
    let logs = audit_repo
        .get_logs_by_node_id(&created_node.id)
        .await
        .unwrap();

    assert!(
        logs.len() >= 2,
        "Expected at least 2 audit logs (create + publish)"
    );
    assert!(logs.iter().any(|log| log.action == AuditLogAction::Publish));
}

#[tokio::test]
#[ignore = "Pending migration to git-like revision/audit pipeline"]
async fn test_audit_log_http_endpoint() {
    let (app, node_svc, _) = setup_test_service_with_audit().await;

    // Setup workspace
    let ws_body = json!({"name": "test-ws", "allowed_node_types": [], "allowed_root_node_types": [], "depends_on": []});
    let req = Request::builder()
        .uri("/workspaces/test-ws")
        .method("PUT")
        .header("content-type", "application/json")
        .body(Body::from(ws_body.to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    // Create a node directly via service
    let mut node = Node {
        id: nanoid::nanoid!(),
        name: "audit-test".to_string(),
        path: "/audit-test".to_string(),
        node_type: "raisin:Folder".to_string(),
        archetype: None,
        properties: HashMap::new(),
        children: vec![],
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
        workspace: Some("test-ws".to_string()),
        owner_id: None,
        has_children: None,
        order_key: "a0".to_string(),
        relations: vec![],
    };

    node_svc.put(node.clone()).await.unwrap();

    // Retrieve audit logs via HTTP API
    let req = Request::builder()
        .uri(&format!("/api/audit/test-ws/by-id/{}", node.id))
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let logs: Vec<AuditLog> = serde_json::from_slice(&body).unwrap();

    assert!(!logs.is_empty(), "Expected audit logs via HTTP endpoint");
    assert_eq!(logs[0].node_id, node.id);
}

#[tokio::test]
#[ignore = "Pending migration to git-like revision/audit pipeline"]
async fn test_audit_log_for_move_operation() {
    let (_, node_svc, audit_repo) = setup_test_service_with_audit().await;

    // Create workspace through service
    let workspace = Workspace::new("test-ws".to_string());
    node_svc
        .storage()
        .workspaces()
        .put("default", "default", workspace)
        .await
        .unwrap();

    // Create a node
    let node = Node {
        id: nanoid::nanoid!(),
        name: "movable".to_string(),
        path: "/movable".to_string(),
        node_type: "raisin:Page".to_string(),
        archetype: None,
        properties: {
            let mut props = HashMap::new();
            props.insert(
                "title".to_string(),
                raisin_models::nodes::properties::PropertyValue::String("Movable Page".to_string()),
            );
            props
        },
        children: vec![],
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
        workspace: Some("test-ws".to_string()),
        owner_id: None,
        has_children: None,
        order_key: "a0".to_string(),
        relations: vec![],
    };

    node_svc.put(node.clone()).await.unwrap();

    // Move the node using service layer
    node_svc.move_node(&node.id, "/moved").await.unwrap();

    // Verify audit logs include move operation
    let logs = audit_repo.get_logs_by_node_id(&node.id).await.unwrap();

    assert!(
        logs.iter().any(|log| log.action == AuditLogAction::Move),
        "Expected Move action in audit logs"
    );
}

#[tokio::test]
#[ignore = "Pending migration to git-like revision/audit pipeline"]
async fn test_audit_log_for_rename_operation() {
    let (_, node_svc, audit_repo) = setup_test_service_with_audit().await;

    // Create workspace through service
    let workspace = Workspace::new("test-ws".to_string());
    node_svc
        .storage()
        .workspaces()
        .put("default", "default", workspace)
        .await
        .unwrap();

    // Create a node
    let node = Node {
        id: nanoid::nanoid!(),
        name: "old-name".to_string(),
        path: "/old-name".to_string(),
        node_type: "raisin:Page".to_string(),
        archetype: None,
        properties: {
            let mut props = HashMap::new();
            props.insert(
                "title".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(
                    "Renamable Page".to_string(),
                ),
            );
            props
        },
        children: vec![],
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
        workspace: Some("test-ws".to_string()),
        owner_id: None,
        has_children: None,
        order_key: "a0".to_string(),
        relations: vec![],
    };

    node_svc.put(node.clone()).await.unwrap();

    // Rename the node using service layer
    node_svc.rename_node("/old-name", "new-name").await.unwrap();

    // Verify audit logs include rename operation
    let logs = audit_repo.get_logs_by_node_id(&node.id).await.unwrap();

    assert!(
        logs.iter().any(|log| log.action == AuditLogAction::Rename),
        "Expected Rename action in audit logs"
    );
}
