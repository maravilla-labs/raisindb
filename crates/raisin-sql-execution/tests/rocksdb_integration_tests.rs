//! RocksDB Integration Tests for SQL Query Engine
//!
//! Tests the complete SQL pipeline with actual RocksDB storage,
//! including revision-aware and branch-aware queries.

use futures::StreamExt;
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage};
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to create a test RocksDB storage instance
async fn create_test_storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path())
        .expect("Failed to create RocksDB storage");
    (Arc::new(storage), temp_dir)
}

/// Helper to create a catalog with workspace tables registered
fn create_test_catalog(workspaces: &[&str]) -> Arc<StaticCatalog> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    for workspace in workspaces {
        catalog.register_workspace(workspace.to_string());
    }
    Arc::new(catalog)
}

/// Helper to create test nodes in a specific branch
async fn setup_test_data(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) {
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashMap;

    // Create some test nodes with simple properties
    let mut props1 = HashMap::new();
    props1.insert(
        "title".to_string(),
        PropertyValue::String(format!("Page 1 on {}", branch)),
    );
    props1.insert(
        "status".to_string(),
        PropertyValue::String("published".to_string()),
    );

    let mut props2 = HashMap::new();
    props2.insert(
        "title".to_string(),
        PropertyValue::String(format!("Page 2 on {}", branch)),
    );
    props2.insert(
        "status".to_string(),
        PropertyValue::String("draft".to_string()),
    );

    // First create the parent folder
    let content_folder = Node {
        id: "content".to_string(),
        path: "/content".to_string(),
        name: "content".to_string(),
        parent: Some("/".to_string()),
        node_type: "raisin:Folder".to_string(),
        properties: HashMap::new(),
        ..Default::default()
    };

    storage
        .nodes()
        .create(
            tenant_id,
            repo_id,
            branch,
            workspace,
            content_folder,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to create content folder");

    let nodes = vec![
        Node {
            id: "node1".to_string(),
            path: "/content/page1".to_string(),
            name: "page1".to_string(),
            parent: Some("content".to_string()),
            node_type: "raisin:Page".to_string(),
            properties: props1,
            ..Default::default()
        },
        Node {
            id: "node2".to_string(),
            path: "/content/page2".to_string(),
            name: "page2".to_string(),
            parent: Some("content".to_string()),
            node_type: "raisin:Page".to_string(),
            properties: props2,
            ..Default::default()
        },
    ];

    for node in nodes {
        storage
            .nodes()
            .create(
                tenant_id,
                repo_id,
                branch,
                workspace,
                node,
                CreateNodeOptions {
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .expect("Failed to insert test node");
    }
}

#[tokio::test]
async fn test_query_default_branch() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let default_branch = "main";
    let workspace = "default";

    // Setup test data in main branch
    setup_test_data(&storage, tenant_id, repo_id, default_branch, workspace).await;

    // Create catalog with workspace registered
    let catalog = create_test_catalog(&[workspace]);

    // Create QueryEngine with default branch = "main"
    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        default_branch.to_string(),
    )
    .with_catalog(catalog);

    // Query without specifying branch - should use default "main"
    let sql = "SELECT id, name FROM default ORDER BY name";
    let mut stream = engine.execute(sql).await.expect("Query failed");

    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        let row = row_result.expect("Failed to read row");
        results.push(row);
    }

    // Should get 3 nodes from main branch (including content folder)
    assert_eq!(results.len(), 3);
    assert!(results.iter().any(|r| {
        matches!(
            r.get("id"),
            Some(raisin_models::nodes::properties::PropertyValue::String(s)) if s == "node1"
        )
    }));
}

#[tokio::test]
async fn test_query_with_branch_override() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let workspace = "default";

    // Setup test data in multiple branches
    setup_test_data(&storage, tenant_id, repo_id, "main", workspace).await;
    setup_test_data(&storage, tenant_id, repo_id, "staging", workspace).await;
    setup_test_data(&storage, tenant_id, repo_id, "dev", workspace).await;

    // Create catalog with workspace registered
    let catalog = create_test_catalog(&[workspace]);

    // Create QueryEngine with default branch = "main"
    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Query staging branch using __branch override
    let sql = "SELECT id, name FROM default WHERE __branch = 'staging' ORDER BY name";
    let mut stream = engine.execute(sql).await.expect("Query failed");

    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        let row = row_result.expect("Failed to read row");
        results.push(row);
    }

    // Should get nodes from staging branch (including content folder)
    assert_eq!(results.len(), 3);
}

#[tokio::test]
async fn test_query_combined_branch_and_revision() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let workspace = "default";

    // Setup test data in dev branch
    setup_test_data(&storage, tenant_id, repo_id, "dev", workspace).await;

    // Create catalog with workspace registered
    let catalog = create_test_catalog(&[workspace]);

    // Create QueryEngine with default branch = "main"
    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Query specific branch with revision filter
    // Note: This tests the SQL parsing and pipeline, actual revision filtering
    // depends on RocksDB's revision implementation
    let sql =
        "SELECT id, name FROM default WHERE __branch = 'dev' AND __revision IS NULL ORDER BY name";
    let result = engine.execute(sql).await;

    // Should succeed in parsing and planning
    assert!(result.is_ok(), "Query should parse and plan successfully");
}

#[tokio::test]
async fn test_query_branch_with_other_predicates() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let workspace = "default";

    // Setup test data in feature branch
    setup_test_data(&storage, tenant_id, repo_id, "feature-x", workspace).await;

    // Create catalog with workspace registered
    let catalog = create_test_catalog(&[workspace]);

    // Create QueryEngine
    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Query with branch override AND other predicates
    let sql = "SELECT id, name FROM default WHERE __branch = 'feature-x' AND node_type = 'raisin:Page' ORDER BY name";
    let mut stream = engine.execute(sql).await.expect("Query failed");

    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        let row = row_result.expect("Failed to read row");
        results.push(row);
    }

    // Should filter by both branch and node_type predicate
    assert_eq!(results.len(), 2); // Both test nodes are raisin:Page type
}

#[tokio::test]
async fn test_query_nonexistent_branch() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";

    // Setup test data in main branch only
    setup_test_data(&storage, tenant_id, repo_id, "main", "default").await;

    // Create catalog with workspace registered
    let catalog = create_test_catalog(&["default"]);

    // Create QueryEngine
    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Query non-existent branch - should return empty results (not error)
    let sql = "SELECT id, name FROM default WHERE __branch = 'nonexistent'";
    let mut stream = engine.execute(sql).await.expect("Query should succeed");

    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        let row = row_result.expect("Failed to read row");
        results.push(row);
    }

    // Should return no results for non-existent branch
    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn test_branch_isolation() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let workspace = "default";

    // Create different data in different branches
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashMap;

    // Main branch
    let mut main_props = HashMap::new();
    main_props.insert(
        "title".to_string(),
        PropertyValue::String("Main Branch Version".to_string()),
    );
    main_props.insert(
        "branch_data".to_string(),
        PropertyValue::String("main".to_string()),
    );

    let main_node = Node {
        id: "shared_id".to_string(),
        path: "/content/page".to_string(),
        name: "page".to_string(),
        node_type: "raisin:Page".to_string(),
        properties: main_props,
        ..Default::default()
    };

    storage
        .nodes()
        .create(
            tenant_id,
            repo_id,
            "main",
            workspace,
            main_node,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to insert main node");

    // Staging branch (different content for same ID)
    let mut staging_props = HashMap::new();
    staging_props.insert(
        "title".to_string(),
        PropertyValue::String("Staging Branch Version".to_string()),
    );
    staging_props.insert(
        "branch_data".to_string(),
        PropertyValue::String("staging".to_string()),
    );

    let staging_node = Node {
        id: "shared_id".to_string(),
        path: "/content/page".to_string(),
        name: "page".to_string(),
        node_type: "raisin:Page".to_string(),
        properties: staging_props,
        ..Default::default()
    };

    storage
        .nodes()
        .create(
            tenant_id,
            repo_id,
            "staging",
            workspace,
            staging_node,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to insert staging node");

    // Create catalog with workspace registered
    let catalog = create_test_catalog(&[workspace]);

    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Query main branch
    let sql_main = "SELECT id FROM default WHERE id = 'shared_id'";
    let mut stream = engine.execute(sql_main).await.expect("Query failed");
    let mut main_results = Vec::new();
    while let Some(row_result) = stream.next().await {
        main_results.push(row_result.expect("Failed to read row"));
    }

    // Query staging branch
    let sql_staging = "SELECT id FROM default WHERE __branch = 'staging' AND id = 'shared_id'";
    let mut stream = engine.execute(sql_staging).await.expect("Query failed");
    let mut staging_results = Vec::new();
    while let Some(row_result) = stream.next().await {
        staging_results.push(row_result.expect("Failed to read row"));
    }

    // Both should find the node (same ID but different branches)
    assert_eq!(main_results.len(), 1);
    assert_eq!(staging_results.len(), 1);

    // The results should be from different branches
    // (In a real scenario, properties would differ - this test validates branch isolation)
}

#[tokio::test]
async fn test_branch_in_workspace_as_table() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";

    // Setup data in "content" workspace on "dev" branch
    setup_test_data(&storage, tenant_id, repo_id, "dev", "content").await;

    // Create catalog with workspace registered
    let catalog = create_test_catalog(&["content"]);

    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Query "content" workspace with branch override
    let sql = "SELECT id, name FROM content WHERE __branch = 'dev' ORDER BY name";
    let mut stream = engine.execute(sql).await.expect("Query failed");

    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        let row = row_result.expect("Failed to read row");
        results.push(row);
    }

    // Should get nodes from content workspace on dev branch (including content folder)
    assert_eq!(results.len(), 3);
}

#[tokio::test]
async fn test_camelcase_table_name_raisin_access_control() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";

    // Create catalog with workspace registered (original name with colon)
    let catalog = create_test_catalog(&["raisin:access_control"]);

    // Verify catalog recognizes CamelCase table name
    assert!(
        catalog.is_workspace("RaisinAccessControl"),
        "Catalog should recognize CamelCase table name"
    );

    // Verify we can get table definition using CamelCase name
    let table_def = catalog
        .get_workspace_table("RaisinAccessControl")
        .expect("Should get table definition for CamelCase name");

    // Verify the table definition uses the CamelCase table name for the schema
    assert_eq!(
        table_def.name, "RaisinAccessControl",
        "Table definition should use the CamelCase table name"
    );

    // Verify we can resolve back to the original workspace name
    assert_eq!(
        catalog.resolve_workspace_name("RaisinAccessControl"),
        Some("raisin:access_control".to_string()),
        "Should resolve CamelCase name to original workspace name"
    );

    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Query using CamelCase table name (no quoting needed!)
    // This should NOT error even if there's no data
    let sql = "SELECT id, name FROM RaisinAccessControl ORDER BY name";
    let result = engine.execute(sql).await;

    // The query should succeed (parse and execute without error)
    assert!(
        result.is_ok(),
        "Query with CamelCase table name should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_camelcase_table_name_raisin_user() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";

    // Create catalog with workspace registered
    let catalog = create_test_catalog(&["raisin:user"]);

    // Verify catalog recognizes CamelCase table name
    assert!(
        catalog.is_workspace("RaisinUser"),
        "Catalog should recognize CamelCase table name"
    );

    // Verify we can get table definition using CamelCase name
    let table_def = catalog
        .get_workspace_table("RaisinUser")
        .expect("Should get table definition for CamelCase name");

    // Verify the table definition uses the CamelCase table name for the schema
    assert_eq!(
        table_def.name, "RaisinUser",
        "Table definition should use the CamelCase table name"
    );

    // Verify we can resolve back to the original workspace name
    assert_eq!(
        catalog.resolve_workspace_name("RaisinUser"),
        Some("raisin:user".to_string()),
        "Should resolve CamelCase name to original workspace name"
    );

    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Query using CamelCase table name: raisin:user -> RaisinUser
    let sql = "SELECT id, name FROM RaisinUser ORDER BY name";
    let result = engine.execute(sql).await;

    // The query should succeed (parse and execute without error)
    assert!(
        result.is_ok(),
        "Query with CamelCase table name should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_backward_compatibility_with_original_workspace_name() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";

    // Create catalog with workspace registered
    let catalog = create_test_catalog(&["raisin:access_control"]);

    // Verify catalog still recognizes original workspace name
    assert!(
        catalog.is_workspace("raisin:access_control"),
        "Catalog should still recognize original workspace name"
    );

    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Query using original workspace name with quotes (backward compatibility)
    let sql = r#"SELECT id FROM "raisin:access_control" ORDER BY id"#;
    let result = engine.execute(sql).await;

    // Should still work with original workspace name
    assert!(
        result.is_ok(),
        "Query with original workspace name should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_multiple_workspaces_with_camelcase() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";

    // Create catalog with both workspaces registered
    let catalog = create_test_catalog(&["raisin:user", "raisin:group"]);

    // Verify both CamelCase names are recognized
    assert!(
        catalog.is_workspace("RaisinUser"),
        "Catalog should recognize RaisinUser"
    );
    assert!(
        catalog.is_workspace("RaisinGroup"),
        "Catalog should recognize RaisinGroup"
    );

    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Query first workspace using CamelCase
    let sql1 = "SELECT id FROM RaisinUser";
    let result1 = engine.execute(sql1).await;
    assert!(
        result1.is_ok(),
        "Query on RaisinUser should succeed: {:?}",
        result1.err()
    );

    // Query second workspace using CamelCase
    let sql2 = "SELECT id FROM RaisinGroup";
    let result2 = engine.execute(sql2).await;
    assert!(
        result2.is_ok(),
        "Query on RaisinGroup should succeed: {:?}",
        result2.err()
    );
}

#[tokio::test]
async fn test_camelcase_with_joins() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";

    // Create catalog with workspace registered
    let catalog = create_test_catalog(&["raisin:user", "default"]);

    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
    )
    .with_catalog(catalog);

    // Self-join using CamelCase table name
    let sql = r#"
        SELECT u1.id as id1, u2.id as id2
        FROM RaisinUser u1
        CROSS JOIN RaisinUser u2
        LIMIT 1
    "#;
    let result = engine.execute(sql).await;

    // The query should succeed (parse and execute without error)
    assert!(
        result.is_ok(),
        "Join query with CamelCase table name should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_json_functions() {
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashMap;

    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let branch = "main";
    let workspace = "default";

    // Setup initial data (this creates the branch implicitly)
    setup_test_data(&storage, tenant_id, repo_id, branch, workspace).await;

    // Create catalog
    let catalog = create_test_catalog(&["default"]);

    // Now create a node with complex JSON properties for testing
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String("Test Product".to_string()),
    );
    properties.insert("price".to_string(), PropertyValue::Float(99.99));
    properties.insert("quantity".to_string(), PropertyValue::Integer(10));
    properties.insert("in_stock".to_string(), PropertyValue::Boolean(true));
    properties.insert(
        "description".to_string(),
        PropertyValue::String("A test product".to_string()),
    );

    // Add nested SEO object
    let mut seo_map = HashMap::new();
    seo_map.insert(
        "title".to_string(),
        PropertyValue::String("SEO Title".to_string()),
    );
    seo_map.insert(
        "description".to_string(),
        PropertyValue::String("SEO Description".to_string()),
    );
    properties.insert("seo".to_string(), PropertyValue::Object(seo_map));

    // Add nested metadata object
    let mut metadata_map = HashMap::new();
    metadata_map.insert(
        "category".to_string(),
        PropertyValue::String("electronics".to_string()),
    );
    properties.insert("metadata".to_string(), PropertyValue::Object(metadata_map));

    // Create test node
    let test_node = Node {
        id: "product1".to_string(),
        path: "/product1".to_string(),
        name: "product1".to_string(),
        node_type: "shop:Product".to_string(),
        parent: Some("/".to_string()),
        properties,
        ..Default::default()
    };

    storage
        .nodes()
        .create(
            tenant_id,
            repo_id,
            branch,
            workspace,
            test_node,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to create test node");

    // Create query engine
    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    )
    .with_catalog(catalog);

    // Test 1: JSON_VALUE with simple path
    let sql =
        "SELECT id, JSON_VALUE(properties, '$.title') AS title FROM default WHERE id = 'product1'";
    let mut stream = engine.execute(sql).await.expect("Query should succeed");
    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        results.push(row_result.expect("Failed to read row"));
    }
    assert_eq!(
        results.len(),
        1,
        "Should return one result for JSON_VALUE test"
    );

    // Test 2: JSON_VALUE with nested path
    let sql = "SELECT id, JSON_VALUE(properties, '$.seo.title') AS seo_title FROM default WHERE id = 'product1'";
    let mut stream = engine
        .execute(sql)
        .await
        .expect("Nested JSON_VALUE should succeed");
    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        results.push(row_result.expect("Failed to read row"));
    }
    assert_eq!(
        results.len(),
        1,
        "Should return one result for nested JSON_VALUE"
    );

    // Test 3: JSON_EXISTS with existing path
    let sql =
        "SELECT id FROM default WHERE id = 'product1' AND JSON_EXISTS(properties, '$.seo.title')";
    let mut stream = engine
        .execute(sql)
        .await
        .expect("JSON_EXISTS should succeed");
    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        results.push(row_result.expect("Failed to read row"));
    }
    assert_eq!(results.len(), 1, "JSON_EXISTS should find existing path");

    // Test 4: JSON_EXISTS with non-existing path
    let sql =
        "SELECT id FROM default WHERE id = 'product1' AND JSON_EXISTS(properties, '$.nonexistent')";
    let mut stream = engine
        .execute(sql)
        .await
        .expect("JSON_EXISTS with non-existing path should succeed");
    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        results.push(row_result.expect("Failed to read row"));
    }
    assert_eq!(
        results.len(),
        0,
        "JSON_EXISTS should not find non-existing path"
    );

    // Test 5: JSON_GET_TEXT
    let sql = "SELECT id, JSON_GET_TEXT(properties, 'description') AS desc FROM default WHERE id = 'product1'";
    let mut stream = engine
        .execute(sql)
        .await
        .expect("JSON_GET_TEXT should succeed");
    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        results.push(row_result.expect("Failed to read row"));
    }
    assert_eq!(results.len(), 1, "JSON_GET_TEXT should return result");

    // Test 6: JSON_GET_DOUBLE
    let sql = "SELECT id, JSON_GET_DOUBLE(properties, 'price') AS price FROM default WHERE id = 'product1'";
    let mut stream = engine
        .execute(sql)
        .await
        .expect("JSON_GET_DOUBLE should succeed");
    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        results.push(row_result.expect("Failed to read row"));
    }
    assert_eq!(results.len(), 1, "JSON_GET_DOUBLE should return result");

    // Test 7: JSON_GET_INT
    let sql =
        "SELECT id, JSON_GET_INT(properties, 'quantity') AS qty FROM default WHERE id = 'product1'";
    let mut stream = engine
        .execute(sql)
        .await
        .expect("JSON_GET_INT should succeed");
    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        results.push(row_result.expect("Failed to read row"));
    }
    assert_eq!(results.len(), 1, "JSON_GET_INT should return result");

    // Test 8: JSON_GET_BOOL
    let sql = "SELECT id, JSON_GET_BOOL(properties, 'in_stock') AS stock FROM default WHERE id = 'product1'";
    let mut stream = engine
        .execute(sql)
        .await
        .expect("JSON_GET_BOOL should succeed");
    let mut results = Vec::new();
    while let Some(row_result) = stream.next().await {
        results.push(row_result.expect("Failed to read row"));
    }
    assert_eq!(results.len(), 1, "JSON_GET_BOOL should return result");
}
