#![cfg(feature = "deprecated-scoped-storage-tests")]
//! Integration tests for multi-tenant scenarios.
//!
//! These tests verify that tenant isolation works correctly and that
//! scoped storage properly separates data between tenants.
//!
//! NOTE: These tests are DEPRECATED and IGNORED because they test the ScopedStorage
//! pattern which has been removed. Multi-tenancy is now handled at the service layer
//! with context-aware services that pass (tenant_id, repo_id, branch) parameters.
//! See the repository-first architecture documentation for the new approach.

#![cfg(all(
    feature = "storage-rocksdb",
    feature = "deprecated-scoped-storage-tests"
))]
#![allow(dead_code)]
#![allow(unused_imports)]

use raisin_core::NodeService;
use raisin_models as models;
use raisin_storage::{NodeRepository, NodeTypeRepository, Storage, WorkspaceRepository};
use raisin_storage_rocks::RocksStorage;
use std::sync::Arc;

/// Test that nodes from different tenants are properly isolated.
///
/// DEPRECATED: This test uses the old ScopedStorage pattern which has been removed.
#[ignore = "Deprecated: uses removed ScopedStorage pattern"]
#[tokio::test]
#[ignore = "Deprecated: uses removed ScopedStorage pattern"]
async fn test_tenant_isolation() {
    let path = format!("/tmp/raisin-mt-test-{}", nanoid::nanoid!(8));
    let _ = std::fs::remove_dir_all(&path);
    let storage = Arc::new(RocksStorage::open(&path).unwrap());

    // Create contexts for two different tenants
    let tenant1_ctx = TenantContext::new("tenant1", "production");
    let tenant2_ctx = TenantContext::new("tenant2", "production");

    // Create scoped storage instances for each tenant
    let storage1 = Arc::new((*storage).clone().scope(tenant1_ctx));
    let storage2 = Arc::new((*storage).clone().scope(tenant2_ctx));

    // Create services for each tenant
    let service1 = NodeService::new(storage1);
    let service2 = NodeService::new(storage2);

    // Create a workspace in tenant1
    let ws1_name = "workspace1";
    let mut ws1 = models::workspace::Workspace::new(ws1_name.to_string());
    ws1.description = Some("Tenant 1 Workspace".into());
    service1
        .storage()
        .workspaces()
        .put(ws1.clone())
        .await
        .unwrap();

    // Create a workspace in tenant2 with the same name
    let mut ws2 = models::workspace::Workspace::new(ws1_name.to_string());
    ws2.description = Some("Tenant 2 Workspace".into());
    service2
        .storage()
        .workspaces()
        .put(ws2.clone())
        .await
        .unwrap();

    // Verify each tenant can only see their own workspace
    let tenant1_ws = service1
        .storage()
        .workspaces()
        .get(ws1_name)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        tenant1_ws.description.as_deref(),
        Some("Tenant 1 Workspace")
    );

    let tenant2_ws = service2
        .storage()
        .workspaces()
        .get(ws1_name)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        tenant2_ws.description.as_deref(),
        Some("Tenant 2 Workspace")
    );

    // Create a NodeType in tenant1
    let node_type = models::nodes::types::NodeType {
        id: Some("page".to_string()),
        strict: None,
        name: "page".to_string(),
        extends: None,
        mixins: None,
        overrides: None,
        description: Some("A page node".to_string()),
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: None,
        required_nodes: None,
        initial_structure: None,
        versionable: None,
        publishable: None,
        auditable: None,
        created_at: None,
        updated_at: None,
        published_at: Some(chrono::Utc::now()),
        published_by: None,
        previous_version: None,
    };
    service1
        .storage()
        .node_types()
        .put(node_type.clone())
        .await
        .unwrap();
    service1
        .storage()
        .node_types()
        .publish("page")
        .await
        .unwrap();

    // Create a node in tenant1
    let node1 = models::nodes::Node {
        id: "node1".to_string(),
        name: "Node 1".to_string(),
        path: "/node1".to_string(),
        node_type: "page".to_string(),
        workspace: Some(ws1_name.to_string()),
        ..Default::default()
    };
    service1.put(ws1_name, node1.clone()).await.unwrap();

    // Verify tenant2 cannot see tenant1's node
    let tenant2_node = service2.get(ws1_name, "node1").await.unwrap();
    assert!(
        tenant2_node.is_none(),
        "Tenant 2 should not see Tenant 1's node"
    );

    // Verify tenant1 can see its own node
    let tenant1_node = service1.get(ws1_name, "node1").await.unwrap();
    assert!(tenant1_node.is_some(), "Tenant 1 should see its own node");
}

/// Test that scoped services operate independently.
#[ignore = "Deprecated: uses removed ScopedStorage pattern"]
#[tokio::test]
async fn test_scoped_node_operations() {
    let path = format!("/tmp/raisin-mt-test-{}", nanoid::nanoid!(8));
    let _ = std::fs::remove_dir_all(&path);
    let storage = Arc::new(RocksStorage::open(&path).unwrap());

    // Create two tenant contexts
    let tenant1_ctx = TenantContext::new("company-a", "production");
    let tenant2_ctx = TenantContext::new("company-b", "production");

    // Create scoped storage instances for each tenant
    let storage1 = Arc::new((*storage).clone().scope(tenant1_ctx));
    let storage2 = Arc::new((*storage).clone().scope(tenant2_ctx));

    let service1 = NodeService::new(storage1);
    let service2 = NodeService::new(storage2);

    let workspace = "docs";

    // Setup workspace and NodeType for tenant1
    let mut ws = models::workspace::Workspace::new(workspace.to_string());
    ws.description = Some("Documentation".into());
    service1
        .storage()
        .workspaces()
        .put(ws.clone())
        .await
        .unwrap();

    let node_type = models::nodes::types::NodeType {
        id: Some("article".to_string()),
        strict: None,
        name: "article".to_string(),
        extends: None,
        mixins: None,
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: None,
        required_nodes: None,
        initial_structure: None,
        versionable: None,
        publishable: None,
        auditable: None,
        created_at: None,
        updated_at: None,
        published_at: Some(chrono::Utc::now()),
        published_by: None,
        previous_version: None,
    };
    service1
        .storage()
        .node_types()
        .put(node_type.clone())
        .await
        .unwrap();
    service1
        .storage()
        .node_types()
        .publish("article")
        .await
        .unwrap();

    // Setup workspace and NodeType for tenant2
    service2
        .storage()
        .workspaces()
        .put(ws.clone())
        .await
        .unwrap();
    service2
        .storage()
        .node_types()
        .put(node_type.clone())
        .await
        .unwrap();
    service2
        .storage()
        .node_types()
        .publish("article")
        .await
        .unwrap();

    // Create nodes for tenant1
    let node1 = models::nodes::Node {
        id: "article1".to_string(),
        name: "company-a-article".to_string(),
        path: "/company-a-article".to_string(),
        node_type: "article".to_string(),
        workspace: Some(workspace.to_string()),
        ..Default::default()
    };
    service1.put(workspace, node1.clone()).await.unwrap();

    // Create nodes for tenant2
    let node2 = models::nodes::Node {
        id: "article1".to_string(), // Same ID as tenant1's node
        name: "company-b-article".to_string(),
        path: "/company-b-article".to_string(),
        node_type: "article".to_string(),
        workspace: Some(workspace.to_string()),
        ..Default::default()
    };
    service2.put(workspace, node2.clone()).await.unwrap();

    // Verify both tenants can access their own nodes independently
    let tenant1_article = service1.get(workspace, "article1").await.unwrap().unwrap();
    assert_eq!(tenant1_article.name, "company-a-article");

    let tenant2_article = service2.get(workspace, "article1").await.unwrap().unwrap();
    assert_eq!(tenant2_article.name, "company-b-article");

    // Verify list operations are isolated
    let tenant1_nodes = service1
        .storage()
        .nodes()
        .list_all(workspace)
        .await
        .unwrap();
    assert_eq!(tenant1_nodes.len(), 1);
    assert_eq!(tenant1_nodes[0].name, "company-a-article");

    let tenant2_nodes = service2
        .storage()
        .nodes()
        .list_all(workspace)
        .await
        .unwrap();
    assert_eq!(tenant2_nodes.len(), 1);
    assert_eq!(tenant2_nodes[0].name, "company-b-article");
}

/// Test that tenant context switching works correctly.
#[ignore = "Deprecated: uses removed ScopedStorage pattern"]
#[tokio::test]
async fn test_tenant_context_switching() {
    let path = format!("/tmp/raisin-mt-test-{}", nanoid::nanoid!(8));
    let _ = std::fs::remove_dir_all(&path);
    let storage = Arc::new(RocksStorage::open(&path).unwrap());

    let ctx_prod = TenantContext::new("acme-corp", "production");
    let ctx_dev = TenantContext::new("acme-corp", "development");

    // Create scoped storage instances for each deployment
    let storage_prod = Arc::new((*storage).clone().scope(ctx_prod));
    let storage_dev = Arc::new((*storage).clone().scope(ctx_dev));

    let service_prod = NodeService::new(storage_prod);
    let service_dev = NodeService::new(storage_dev);

    let workspace = "content";

    // Setup workspace and NodeType
    let ws = models::workspace::Workspace::new(workspace.to_string());
    service_prod
        .storage()
        .workspaces()
        .put(ws.clone())
        .await
        .unwrap();
    service_dev
        .storage()
        .workspaces()
        .put(ws.clone())
        .await
        .unwrap();

    let node_type = models::nodes::types::NodeType {
        id: Some("post".to_string()),
        strict: None,
        name: "post".to_string(),
        extends: None,
        mixins: None,
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: None,
        required_nodes: None,
        initial_structure: None,
        versionable: None,
        publishable: None,
        auditable: None,
        created_at: None,
        updated_at: None,
        published_at: Some(chrono::Utc::now()),
        published_by: None,
        previous_version: None,
    };
    service_prod
        .storage()
        .node_types()
        .put(node_type.clone())
        .await
        .unwrap();
    service_prod
        .storage()
        .node_types()
        .publish("post")
        .await
        .unwrap();
    service_dev
        .storage()
        .node_types()
        .put(node_type.clone())
        .await
        .unwrap();
    service_dev
        .storage()
        .node_types()
        .publish("post")
        .await
        .unwrap();

    // Create a node in production
    let prod_node = models::nodes::Node {
        id: "post1".to_string(),
        name: "production-post".to_string(),
        path: "/production-post".to_string(),
        node_type: "post".to_string(),
        workspace: Some(workspace.to_string()),
        ..Default::default()
    };
    service_prod
        .put(workspace, prod_node.clone())
        .await
        .unwrap();

    // Create a node in development
    let dev_node = models::nodes::Node {
        id: "post1".to_string(),
        name: "development-post".to_string(),
        path: "/development-post".to_string(),
        node_type: "post".to_string(),
        workspace: Some(workspace.to_string()),
        ..Default::default()
    };
    service_dev.put(workspace, dev_node.clone()).await.unwrap();

    // Verify isolation between environments
    let prod_result = service_prod.get(workspace, "post1").await.unwrap().unwrap();
    assert_eq!(prod_result.name, "production-post");

    let dev_result = service_dev.get(workspace, "post1").await.unwrap().unwrap();
    assert_eq!(dev_result.name, "development-post");
}

/// Test that NodeType isolation works across tenants.
#[ignore = "Deprecated: uses removed ScopedStorage pattern"]
#[tokio::test]
async fn test_node_type_isolation() {
    let path = format!("/tmp/raisin-mt-test-{}", nanoid::nanoid!(8));
    let _ = std::fs::remove_dir_all(&path);
    let storage = Arc::new(RocksStorage::open(&path).unwrap());

    let tenant1_ctx = TenantContext::new("tenant1", "production");
    let tenant2_ctx = TenantContext::new("tenant2", "production");

    // Create scoped storage instances for each tenant
    let storage1 = Arc::new((*storage).clone().scope(tenant1_ctx));
    let storage2 = Arc::new((*storage).clone().scope(tenant2_ctx));

    let service1 = NodeService::new(storage1);
    let service2 = NodeService::new(storage2);

    // Create a NodeType only in tenant1
    let node_type1 = models::nodes::types::NodeType {
        id: Some("custom-type".to_string()),
        strict: None,
        name: "custom-type".to_string(),
        extends: None,
        mixins: None,
        overrides: None,
        description: Some("Tenant 1 specific type".to_string()),
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: None,
        required_nodes: None,
        initial_structure: None,
        versionable: None,
        publishable: None,
        auditable: None,
        created_at: None,
        updated_at: None,
        published_at: Some(chrono::Utc::now()),
        published_by: None,
        previous_version: None,
    };
    service1
        .storage()
        .node_types()
        .put(node_type1.clone())
        .await
        .unwrap();
    service1
        .storage()
        .node_types()
        .publish("custom-type")
        .await
        .unwrap();

    // Verify tenant1 can see the NodeType
    let tenant1_type = service1
        .storage()
        .node_types()
        .get("custom-type")
        .await
        .unwrap();
    assert!(tenant1_type.is_some());

    // Verify tenant2 cannot see tenant1's NodeType
    let tenant2_type = service2
        .storage()
        .node_types()
        .get("custom-type")
        .await
        .unwrap();
    assert!(
        tenant2_type.is_none(),
        "Tenant 2 should not see Tenant 1's NodeType"
    );
}
