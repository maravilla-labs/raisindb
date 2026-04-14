//! RocksDB Integration Tests for SQL Query Engine
//!
//! Tests the complete SQL pipeline with actual RocksDB storage,
//! including revision-aware and branch-aware queries.

use futures::StreamExt;
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    CreateNodeOptions, NodeRepository, RelationRepository, Storage, StorageScope,
};
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to create a test RocksDB storage instance with branches initialized
async fn create_test_storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    use raisin_storage::BranchRepository;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path())
        .expect("Failed to create RocksDB storage");

    // Create default branches used by tests
    for branch in &["main", "staging", "dev"] {
        let _ = storage
            .branches()
            .create_branch(
                "test_tenant",
                "test_repo",
                branch,
                "test-user",
                None,
                None,
                false,
                false,
            )
            .await;
    }

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
    use raisin_storage::BranchRepository;

    // Ensure branch exists (ignore error if already created)
    let _ = storage
        .branches()
        .create_branch(
            tenant_id,
            repo_id,
            branch,
            "test-user",
            None,
            None,
            false,
            false,
        )
        .await;
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
            StorageScope::new(tenant_id, repo_id, branch, workspace),
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
                StorageScope::new(tenant_id, repo_id, branch, workspace),
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
            StorageScope::new(tenant_id, repo_id, "main", workspace),
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
            StorageScope::new(tenant_id, repo_id, "staging", workspace),
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
            StorageScope::new(tenant_id, repo_id, branch, workspace),
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

// ==================== GRAPH_TABLE Integration Tests ====================

/// Helper to create a graph with nodes and relations for GRAPH_TABLE testing.
///
/// Creates a social graph:
///   Alice --FOLLOWS--> Bob --FOLLOWS--> Charlie --FOLLOWS--> Alice (cycle)
///   Alice --FOLLOWS--> Charlie (shortcut)
///   Dave --FOLLOWS--> Bob (separate component entry)
async fn setup_graph_data(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) {
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::nodes::RelationRef;
    use std::collections::HashMap;

    // Ensure branch exists
    {
        use raisin_storage::BranchRepository;
        let _ = storage
            .branches()
            .create_branch(
                tenant_id,
                repo_id,
                branch,
                "test-user",
                None,
                None,
                false,
                false,
            )
            .await;
    }

    let scope = StorageScope::new(tenant_id, repo_id, branch, workspace);

    // Create parent folder
    let users_folder = Node {
        id: "users".to_string(),
        path: "/users".to_string(),
        name: "users".to_string(),
        parent: Some("/".to_string()),
        node_type: "raisin:Folder".to_string(),
        properties: HashMap::new(),
        ..Default::default()
    };
    storage
        .nodes()
        .create(
            scope.clone(),
            users_folder,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to create users folder");

    // Create user nodes
    let users = vec![
        ("alice", "Alice"),
        ("bob", "Bob"),
        ("charlie", "Charlie"),
        ("dave", "Dave"),
    ];

    for (id, name) in &users {
        let mut props = HashMap::new();
        props.insert("name".to_string(), PropertyValue::String(name.to_string()));

        let node = Node {
            id: id.to_string(),
            path: format!("/users/{}", id),
            name: id.to_string(),
            parent: Some("users".to_string()),
            node_type: "raisin:User".to_string(),
            properties: props,
            ..Default::default()
        };

        storage
            .nodes()
            .create(
                scope.clone(),
                node,
                CreateNodeOptions {
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .unwrap_or_else(|e| panic!("Failed to create user {}: {}", id, e));
    }

    // Create FOLLOWS relations: Alice->Bob, Bob->Charlie, Charlie->Alice, Alice->Charlie, Dave->Bob
    let relations = vec![
        ("alice", "bob"),
        ("bob", "charlie"),
        ("charlie", "alice"),
        ("alice", "charlie"),
        ("dave", "bob"),
    ];

    for (from, to) in relations {
        let rel = RelationRef::new(
            to.to_string(),
            workspace.to_string(),
            "raisin:User".to_string(),
            "FOLLOWS".to_string(),
            None,
        );
        storage
            .relations()
            .add_relation(scope.clone(), from, "raisin:User", rel)
            .await
            .unwrap_or_else(|e| panic!("Failed to create relation {}->{}:  {}", from, to, e));
    }
}

#[tokio::test]
async fn test_graph_table_basic_match() {
    let (storage, _temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let branch = "main";
    let workspace = "default";

    setup_graph_data(&storage, tenant_id, repo_id, branch, workspace).await;

    let catalog = create_test_catalog(&[workspace]);
    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    )
    .with_catalog(catalog);

    // Basic GRAPH_TABLE: match all nodes with outgoing FOLLOWS edges
    let sql = r#"SELECT * FROM GRAPH_TABLE(MATCH (a:User)-[r:FOLLOWS]->(b:User) COLUMNS (a.id AS source, b.id AS target))"#;

    let result = engine.execute(sql).await;
    match result {
        Ok(mut stream) => {
            let mut rows = Vec::new();
            while let Some(row_result) = stream.next().await {
                match row_result {
                    Ok(row) => rows.push(row),
                    Err(e) => panic!("Row error: {:?}", e),
                }
            }
            // We created 5 FOLLOWS relations
            assert_eq!(
                rows.len(),
                5,
                "Should have 5 FOLLOWS edges, got {}",
                rows.len()
            );
            println!("GRAPH_TABLE basic match: {} rows returned", rows.len());
        }
        Err(e) => {
            panic!("GRAPH_TABLE query failed: {:?}", e);
        }
    }
}

/// Helper: create engine with graph data ready
async fn create_graph_engine() -> (QueryEngine<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let (storage, temp_dir) = create_test_storage().await;
    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let branch = "main";
    let workspace = "default";

    setup_graph_data(&storage, tenant_id, repo_id, branch, workspace).await;

    let catalog = create_test_catalog(&[workspace]);
    let engine = QueryEngine::new(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    )
    .with_catalog(catalog);

    (engine, temp_dir)
}

/// Helper: execute a SQL query and collect all rows
async fn execute_and_collect(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> Vec<raisin_sql_execution::Row> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("Query failed: {:?}\nSQL: {}", e, sql));

    let mut rows = Vec::new();
    while let Some(row_result) = stream.next().await {
        rows.push(row_result.unwrap_or_else(|e| panic!("Row error: {:?}\nSQL: {}", e, sql)));
    }
    rows
}

/// Helper: find a row by node_id and extract a column value
fn get_value_for_node<'a>(
    rows: &'a [raisin_sql_execution::Row],
    node_id: &str,
    col: &str,
) -> Option<&'a raisin_models::nodes::properties::PropertyValue> {
    use raisin_models::nodes::properties::PropertyValue;
    for row in rows {
        if let Some(PropertyValue::String(id)) = row.get("node_id") {
            if id == node_id {
                return row.get(col);
            }
        }
    }
    None
}

fn as_integer(pv: &raisin_models::nodes::properties::PropertyValue) -> Option<i64> {
    match pv {
        raisin_models::nodes::properties::PropertyValue::Integer(i) => Some(*i),
        _ => None,
    }
}

fn as_float(pv: &raisin_models::nodes::properties::PropertyValue) -> Option<f64> {
    match pv {
        raisin_models::nodes::properties::PropertyValue::Float(f) => Some(*f),
        raisin_models::nodes::properties::PropertyValue::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

fn is_null(pv: &raisin_models::nodes::properties::PropertyValue) -> bool {
    matches!(pv, raisin_models::nodes::properties::PropertyValue::Null)
}

// ==================== Graph Setup Helpers ====================

/// Create nodes + directed relations for a graph.
/// `nodes`: list of (id, name) pairs
/// `edges`: list of (from_id, to_id, weight) triples
async fn setup_graph(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    nodes: &[(&str, &str)],
    edges: &[(&str, &str, Option<f32>)],
) {
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::nodes::RelationRef;
    use std::collections::HashMap;

    {
        use raisin_storage::BranchRepository;
        let _ = storage
            .branches()
            .create_branch(
                tenant_id,
                repo_id,
                branch,
                "test-user",
                None,
                None,
                false,
                false,
            )
            .await;
    }

    let scope = StorageScope::new(tenant_id, repo_id, branch, workspace);

    // Create parent folder
    let folder = Node {
        id: "graph_nodes".to_string(),
        path: "/graph_nodes".to_string(),
        name: "graph_nodes".to_string(),
        parent: Some("/".to_string()),
        node_type: "raisin:Folder".to_string(),
        properties: HashMap::new(),
        ..Default::default()
    };
    let _ = storage
        .nodes()
        .create(
            scope.clone(),
            folder,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await;

    // Create nodes
    for (id, name) in nodes {
        let mut props = HashMap::new();
        props.insert("name".to_string(), PropertyValue::String(name.to_string()));
        let node = Node {
            id: id.to_string(),
            path: format!("/graph_nodes/{}", id),
            name: id.to_string(),
            parent: Some("graph_nodes".to_string()),
            node_type: "raisin:User".to_string(),
            properties: props,
            ..Default::default()
        };
        let _ = storage
            .nodes()
            .create(
                scope.clone(),
                node,
                CreateNodeOptions {
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await;
    }

    // Create directed edges
    for (from, to, weight) in edges {
        let rel = RelationRef::new(
            to.to_string(),
            workspace.to_string(),
            "raisin:User".to_string(),
            "FOLLOWS".to_string(),
            *weight,
        );
        storage
            .relations()
            .add_relation(scope.clone(), from, "raisin:User", rel)
            .await
            .unwrap_or_else(|e| panic!("Failed relation {}→{}: {}", from, to, e));
    }
}

/// Create engine for a graph, returning (engine, TempDir)
async fn create_engine_with_graph(
    nodes: &[(&str, &str)],
    edges: &[(&str, &str, Option<f32>)],
) -> (QueryEngine<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let (storage, temp_dir) = create_test_storage().await;
    let tenant = "test_tenant";
    let repo = "test_repo";
    let branch = "main";
    let workspace = "default";

    setup_graph(&storage, tenant, repo, branch, workspace, nodes, edges).await;

    let catalog = create_test_catalog(&[workspace]);
    let engine = QueryEngine::new(
        storage.clone(),
        tenant.to_string(),
        repo.to_string(),
        branch.to_string(),
    )
    .with_catalog(catalog);

    (engine, temp_dir)
}

// ==================== Graph Definitions ====================

/// Graph 1: Two communities with bridge
///
/// Community A (triangle): A1↔A2, A1↔A3, A2↔A3
/// Community B (triangle): B1↔B2, B1↔B3, B2↔B3
/// Bridge: A3→B1 (single directed edge — weak link)
///
/// Note: GRAPH_TABLE MATCH (n:User) only returns nodes that participate in
/// relations, so isolated nodes would not appear. All 6 nodes here have edges.
fn two_community_graph() -> (
    Vec<(&'static str, &'static str)>,
    Vec<(&'static str, &'static str, Option<f32>)>,
) {
    let nodes = vec![
        ("a1", "Alice1"),
        ("a2", "Alice2"),
        ("a3", "Alice3"),
        ("b1", "Bob1"),
        ("b2", "Bob2"),
        ("b3", "Bob3"),
    ];
    let edges = vec![
        // Community A triangle (bidirectional)
        ("a1", "a2", None),
        ("a2", "a1", None),
        ("a1", "a3", None),
        ("a3", "a1", None),
        ("a2", "a3", None),
        ("a3", "a2", None),
        // Community B triangle (bidirectional)
        ("b1", "b2", None),
        ("b2", "b1", None),
        ("b1", "b3", None),
        ("b3", "b1", None),
        ("b2", "b3", None),
        ("b3", "b2", None),
        // Bridge: A3 → B1 (weak link)
        ("a3", "b1", None),
    ];
    (nodes, edges)
}

/// Graph 2: Weighted path graph for SSSP
///
/// S →(1.0)→ A →(2.0)→ B →(1.0)→ T
/// S →(10.0)→ T  (direct but expensive)
fn weighted_path_graph() -> (
    Vec<(&'static str, &'static str)>,
    Vec<(&'static str, &'static str, Option<f32>)>,
) {
    let nodes = vec![
        ("s", "Source"),
        ("a", "NodeA"),
        ("b", "NodeB"),
        ("t", "Target"),
    ];
    let edges = vec![
        ("s", "a", Some(1.0)),
        ("a", "b", Some(2.0)),
        ("b", "t", Some(1.0)),
        ("s", "t", Some(10.0)),
    ];
    (nodes, edges)
}

/// Graph 3: Disconnected graph
///
/// X → Y   (component 1)
/// Z        (isolated, component 2)
fn disconnected_graph() -> (
    Vec<(&'static str, &'static str)>,
    Vec<(&'static str, &'static str, Option<f32>)>,
) {
    let nodes = vec![("x", "NodeX"), ("y", "NodeY"), ("z", "NodeZ")];
    let edges = vec![("x", "y", None)];
    (nodes, edges)
}

// ==================== WCC Tests ====================

#[tokio::test]
async fn test_graph_table_wcc_two_communities() {
    let (nodes, edges) = two_community_graph();
    let (engine, _dir) = create_engine_with_graph(&nodes, &edges).await;

    let sql = r#"SELECT * FROM GRAPH_TABLE(MATCH (n:User) COLUMNS (n.id AS node_id, wcc(n) AS component))"#;
    let rows = execute_and_collect(&engine, sql).await;

    assert_eq!(
        rows.len(),
        6,
        "Should have 6 nodes (all participate in relations)"
    );

    // All 6 nodes are connected (bridge A3→B1 makes it undirected-connected) → 1 component
    let a1 = as_integer(get_value_for_node(&rows, "a1", "component").expect("a1")).expect("int");
    let a2 = as_integer(get_value_for_node(&rows, "a2", "component").expect("a2")).expect("int");
    let a3 = as_integer(get_value_for_node(&rows, "a3", "component").expect("a3")).expect("int");
    let b1 = as_integer(get_value_for_node(&rows, "b1", "component").expect("b1")).expect("int");
    let b2 = as_integer(get_value_for_node(&rows, "b2", "component").expect("b2")).expect("int");
    let b3 = as_integer(get_value_for_node(&rows, "b3", "component").expect("b3")).expect("int");

    assert_eq!(a1, a2, "A1 and A2 same component");
    assert_eq!(a2, a3, "A2 and A3 same component");
    assert_eq!(a3, b1, "A3 and B1 same component (bridge)");
    assert_eq!(b1, b2, "B1 and B2 same component");
    assert_eq!(b2, b3, "B2 and B3 same component");

    // All in 1 component
    let unique: std::collections::HashSet<i64> = [a1, a2, a3, b1, b2, b3].into();
    assert_eq!(
        unique.len(),
        1,
        "Should have exactly 1 component (all connected via bridge), got {}",
        unique.len()
    );
}

#[tokio::test]
async fn test_graph_table_wcc_disconnected() {
    // X→Y, Z isolated. But MATCH (n:User) only returns nodes in relations.
    // Z has no edges → Z won't appear. X and Y will appear, same component.
    // To test actual disconnection, we need two separate edge groups.
    // Use: X→Y and P→Q (two disconnected pairs)
    let nodes = vec![
        ("x", "NodeX"),
        ("y", "NodeY"),
        ("p", "NodeP"),
        ("q", "NodeQ"),
    ];
    let edges = vec![("x", "y", None), ("p", "q", None)];
    let (engine, _dir) = create_engine_with_graph(&nodes, &edges).await;

    let sql = r#"SELECT * FROM GRAPH_TABLE(MATCH (n:User) COLUMNS (n.id AS node_id, wcc(n) AS component))"#;
    let rows = execute_and_collect(&engine, sql).await;

    assert_eq!(rows.len(), 4, "Should have 4 nodes (X,Y,P,Q)");

    let x = as_integer(get_value_for_node(&rows, "x", "component").expect("x")).expect("int");
    let y = as_integer(get_value_for_node(&rows, "y", "component").expect("y")).expect("int");
    let p = as_integer(get_value_for_node(&rows, "p", "component").expect("p")).expect("int");
    let q = as_integer(get_value_for_node(&rows, "q", "component").expect("q")).expect("int");

    // With per-query caching, component IDs are now consistent across all nodes
    assert_eq!(
        x, y,
        "X and Y should be in same component (x={}, y={})",
        x, y
    );
    assert_eq!(
        p, q,
        "P and Q should be in same component (p={}, q={})",
        p, q
    );
    assert_ne!(
        x, p,
        "X-Y component should differ from P-Q component (x={}, p={})",
        x, p
    );
}

// ==================== Triangle Count Tests ====================

#[tokio::test]
async fn test_graph_table_triangle_count_exact() {
    let (nodes, edges) = two_community_graph();
    let (engine, _dir) = create_engine_with_graph(&nodes, &edges).await;

    let sql = r#"SELECT * FROM GRAPH_TABLE(MATCH (n:User) COLUMNS (n.id AS node_id, triangle_count(n) AS triangles))"#;
    let rows = execute_and_collect(&engine, sql).await;

    assert_eq!(rows.len(), 6);

    // Each community forms exactly 1 triangle
    let a1 = as_integer(get_value_for_node(&rows, "a1", "triangles").expect("a1")).expect("int");
    let a2 = as_integer(get_value_for_node(&rows, "a2", "triangles").expect("a2")).expect("int");
    let a3 = as_integer(get_value_for_node(&rows, "a3", "triangles").expect("a3")).expect("int");
    let b1 = as_integer(get_value_for_node(&rows, "b1", "triangles").expect("b1")).expect("int");
    let b2 = as_integer(get_value_for_node(&rows, "b2", "triangles").expect("b2")).expect("int");
    let b3 = as_integer(get_value_for_node(&rows, "b3", "triangles").expect("b3")).expect("int");

    assert_eq!(a1, 1, "A1 is in exactly 1 triangle, got {}", a1);
    assert_eq!(a2, 1, "A2 is in exactly 1 triangle, got {}", a2);
    assert_eq!(a3, 1, "A3 is in exactly 1 triangle, got {}", a3);
    assert_eq!(b1, 1, "B1 is in exactly 1 triangle, got {}", b1);
    assert_eq!(b2, 1, "B2 is in exactly 1 triangle, got {}", b2);
    assert_eq!(b3, 1, "B3 is in exactly 1 triangle, got {}", b3);
}

// ==================== LCC Tests ====================

#[tokio::test]
async fn test_graph_table_lcc_exact() {
    let (nodes, edges) = two_community_graph();
    let (engine, _dir) = create_engine_with_graph(&nodes, &edges).await;

    let sql = r#"SELECT * FROM GRAPH_TABLE(MATCH (n:User) COLUMNS (n.id AS node_id, lcc(n) AS clustering))"#;
    let rows = execute_and_collect(&engine, sql).await;

    assert_eq!(rows.len(), 6);

    let a1 = as_float(get_value_for_node(&rows, "a1", "clustering").expect("a1")).expect("float");
    let a2 = as_float(get_value_for_node(&rows, "a2", "clustering").expect("a2")).expect("float");
    let a3 = as_float(get_value_for_node(&rows, "a3", "clustering").expect("a3")).expect("float");

    // A1: undirected deg=2 (A2,A3), triangles=1 → LCC = 2*1/(2*1) = 1.0
    assert!((a1 - 1.0).abs() < 0.01, "A1 LCC should be 1.0, got {}", a1);

    // A2: undirected deg=2 (A1,A3), triangles=1 → LCC = 1.0
    assert!((a2 - 1.0).abs() < 0.01, "A2 LCC should be 1.0, got {}", a2);

    // A3: undirected deg=3 (A1,A2,B1), triangles=1 → LCC = 2*1/(3*2) = 0.333
    assert!(
        (a3 - 0.333).abs() < 0.02,
        "A3 LCC should be ~0.333, got {}",
        a3
    );
}

// ==================== PageRank Tests ====================

#[tokio::test]
async fn test_graph_table_pagerank_properties() {
    let (nodes, edges) = two_community_graph();
    let (engine, _dir) = create_engine_with_graph(&nodes, &edges).await;

    let sql = r#"SELECT * FROM GRAPH_TABLE(MATCH (n:User) COLUMNS (n.id AS node_id, pageRank(n) AS rank))"#;
    let rows = execute_and_collect(&engine, sql).await;

    assert_eq!(rows.len(), 6);

    let mut scores: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for node_id in &["a1", "a2", "a3", "b1", "b2", "b3"] {
        let score =
            as_float(get_value_for_node(&rows, node_id, "rank").expect(node_id)).expect("float");
        assert!(
            score > 0.0,
            "{} PageRank should be > 0, got {}",
            node_id,
            score
        );
        scores.insert(node_id.to_string(), score);
    }

    // Sum ≈ 1.0
    let sum: f64 = scores.values().sum();
    assert!(
        (sum - 1.0).abs() < 0.05,
        "PageRank sum should be ~1.0, got {}",
        sum
    );

    // B1 has 3 in-edges (B2, B3, A3) vs A1 has 2 in-edges (A2, A3)
    // B1 should have highest or near-highest PageRank
    let b1_score = scores["b1"];
    assert!(
        b1_score >= scores["a1"] - 0.01,
        "B1 ({:.4}) should have >= PageRank as A1 ({:.4}) — B1 has more in-edges",
        b1_score,
        scores["a1"]
    );
}

// ==================== CDLP Tests ====================

#[tokio::test]
async fn test_graph_table_cdlp_separation() {
    let (nodes, edges) = two_community_graph();
    let (engine, _dir) = create_engine_with_graph(&nodes, &edges).await;

    let sql = r#"SELECT * FROM GRAPH_TABLE(MATCH (n:User) COLUMNS (n.id AS node_id, cdlp(n) AS community))"#;
    let rows = execute_and_collect(&engine, sql).await;

    assert_eq!(rows.len(), 6);

    let a1 = as_integer(get_value_for_node(&rows, "a1", "community").expect("a1")).expect("int");
    let a2 = as_integer(get_value_for_node(&rows, "a2", "community").expect("a2")).expect("int");
    let a3 = as_integer(get_value_for_node(&rows, "a3", "community").expect("a3")).expect("int");
    let b1 = as_integer(get_value_for_node(&rows, "b1", "community").expect("b1")).expect("int");
    let b2 = as_integer(get_value_for_node(&rows, "b2", "community").expect("b2")).expect("int");
    let b3 = as_integer(get_value_for_node(&rows, "b3", "community").expect("b3")).expect("int");

    // Within each community, nodes should share same label
    assert_eq!(a1, a2, "A1 and A2 should be in same community");
    assert_eq!(a2, a3, "A2 and A3 should be in same community");
    assert_eq!(b1, b2, "B1 and B2 should be in same community");
    assert_eq!(b2, b3, "B2 and B3 should be in same community");

    // At least 2 distinct community labels (A-group vs B-group)
    let unique: std::collections::HashSet<i64> = [a1, a2, a3, b1, b2, b3].into();
    assert!(
        unique.len() >= 2,
        "Should have at least 2 communities, got {}",
        unique.len()
    );
}

// ==================== BFS Tests ====================

#[tokio::test]
async fn test_graph_table_bfs_directed() {
    let (nodes, edges) = weighted_path_graph();
    let (engine, _dir) = create_engine_with_graph(&nodes, &edges).await;

    // BFS from s (ignoring weights — counts hops)
    // s=0, a=1 (s→a), t=1 (s→t direct), b=2 (s→a→b)
    let sql = r#"SELECT * FROM GRAPH_TABLE(MATCH (n:User) COLUMNS (n.id AS node_id, bfs(n, 's') AS distance))"#;
    let rows = execute_and_collect(&engine, sql).await;

    assert_eq!(rows.len(), 4);

    let s = as_integer(get_value_for_node(&rows, "s", "distance").expect("s")).expect("int");
    let a = as_integer(get_value_for_node(&rows, "a", "distance").expect("a")).expect("int");
    let b = as_integer(get_value_for_node(&rows, "b", "distance").expect("b")).expect("int");
    let t = as_integer(get_value_for_node(&rows, "t", "distance").expect("t")).expect("int");

    assert_eq!(s, 0, "BFS s→s should be 0, got {}", s);
    assert_eq!(a, 1, "BFS s→a should be 1, got {}", a);
    assert_eq!(t, 1, "BFS s→t should be 1 (direct hop), got {}", t);
    assert_eq!(b, 2, "BFS s→b should be 2, got {}", b);
}

#[tokio::test]
async fn test_graph_table_bfs_unreachable() {
    // X→Y and P→Q (disconnected). BFS from X: X=0, Y=1, P=Null, Q=Null
    let nodes = vec![
        ("x", "NodeX"),
        ("y", "NodeY"),
        ("p", "NodeP"),
        ("q", "NodeQ"),
    ];
    let edges = vec![("x", "y", None), ("p", "q", None)];
    let (engine, _dir) = create_engine_with_graph(&nodes, &edges).await;

    let sql = r#"SELECT * FROM GRAPH_TABLE(MATCH (n:User) COLUMNS (n.id AS node_id, bfs(n, 'x') AS distance))"#;
    let rows = execute_and_collect(&engine, sql).await;

    assert_eq!(rows.len(), 4);

    let x = as_integer(get_value_for_node(&rows, "x", "distance").expect("x")).expect("int");
    let y = as_integer(get_value_for_node(&rows, "y", "distance").expect("y")).expect("int");
    let p_val = get_value_for_node(&rows, "p", "distance").expect("p");
    let q_val = get_value_for_node(&rows, "q", "distance").expect("q");

    assert_eq!(x, 0, "BFS x→x should be 0, got {}", x);
    assert_eq!(y, 1, "BFS x→y should be 1, got {}", y);
    assert!(
        is_null(p_val),
        "P should be unreachable from X (Null), got {:?}",
        p_val
    );
    assert!(
        is_null(q_val),
        "Q should be unreachable from X (Null), got {:?}",
        q_val
    );
}

// ==================== SSSP Tests ====================

#[tokio::test]
async fn test_graph_table_sssp_weighted() {
    let (nodes, edges) = weighted_path_graph();
    let (engine, _dir) = create_engine_with_graph(&nodes, &edges).await;

    // SSSP from s with weights:
    //   s→a = 1.0
    //   s→a→b = 1.0 + 2.0 = 3.0
    //   s→a→b→t = 1.0 + 2.0 + 1.0 = 4.0
    //   s→t direct = 10.0
    //   Shortest to t: via a→b→t = 4.0 (not direct 10.0)
    let sql = r#"SELECT * FROM GRAPH_TABLE(MATCH (n:User) COLUMNS (n.id AS node_id, sssp(n, 's') AS distance))"#;
    let rows = execute_and_collect(&engine, sql).await;

    assert_eq!(rows.len(), 4);

    let s = as_float(get_value_for_node(&rows, "s", "distance").expect("s")).expect("float");
    let a = as_float(get_value_for_node(&rows, "a", "distance").expect("a")).expect("float");
    let b = as_float(get_value_for_node(&rows, "b", "distance").expect("b")).expect("float");
    let t = as_float(get_value_for_node(&rows, "t", "distance").expect("t")).expect("float");

    assert!((s - 0.0).abs() < 0.001, "SSSP s→s should be 0.0, got {}", s);
    assert!((a - 1.0).abs() < 0.001, "SSSP s→a should be 1.0, got {}", a);
    assert!((b - 3.0).abs() < 0.001, "SSSP s→b should be 3.0, got {}", b);
    assert!(
        (t - 4.0).abs() < 0.001,
        "SSSP s→t should be 4.0 (via a→b→t, not direct 10.0), got {}",
        t
    );
}
