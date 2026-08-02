//! Profiling Test for Query Execution
//!
//! This test is designed to help identify performance bottlenecks in SQL query execution.
//! Run with profiling enabled:
//!
//! ```bash
//! RUST_LOG=raisin_sql=debug cargo test --features profiling profiling_test -- --nocapture
//! ```
//!
//! The output will show timing breakdown for each operator and expression.

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_sql_execution::QueryEngine;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
use std::sync::Arc;
use tracing_subscriber::{fmt, EnvFilter};

/// Initialize tracing subscriber with timing information
fn init_tracing() {
    let _ = fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_timer(fmt::time::uptime())
        .with_target(false)
        .with_level(true)
        .try_init();
}

#[tokio::test]
async fn profile_complex_json_projection() {
    init_tracing();

    tracing::info!("========================================");
    tracing::info!("Starting profiling test: Complex JSON projection query");
    tracing::info!("========================================");

    // Storage for the fixture. RocksDB (a temp dir) rather than the in-memory
    // backend: the engine bounds reads at the branch HEAD and the in-memory
    // backend does not advance it, so every seeded node reads back invisible.
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let storage = Arc::new(
        raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("open rocksdb storage"),
    );

    // Create engine
    let engine = QueryEngine::new(storage.clone(), "tenant1", "repo1", "main");

    // Setup test data
    tracing::info!("Setting up test data...");
    setup_test_data(&storage).await;

    tracing::info!("========================================");
    tracing::info!("Running query with complex JSON projections");
    tracing::info!("========================================");

    // Query with complex JSON projections (similar to user's slow query)
    let sql = r#"
        SELECT
            id,
            name,
            properties ->> 'username' as username,
            properties ->> 'email' as email,
            properties ->> 'displayName' as displayName,
            properties ->> 'avatar' as avatar,
            properties ->> 'bio' as bio,
            properties ->> 'location' as location,
            properties ->> 'website' as website,
            properties ->> 'twitter' as twitter,
            properties ->> 'github' as github,
            properties ->> 'status' as status,
            properties ->> 'role' as role,
            properties ->> 'department' as department,
            properties ->> 'title' as title,
            properties ->> 'phone' as phone,
            properties ->> 'mobile' as mobile,
            properties ->> 'address' as address,
            properties ->> 'city' as city,
            properties ->> 'country' as country,
            properties ->> 'timezone' as timezone,
            properties ->> 'language' as language,
            properties ->> 'theme' as theme,
            properties ->> 'notifications' as notifications,
            properties ->> 'privacy' as privacy,
            properties ->> 'subscription' as subscription,
            properties ->> 'verified' as verified,
            properties ->> 'last_login' as last_login
        FROM nodes
        WHERE node_type = 'user'
        LIMIT 100
    "#;

    let query_start = std::time::Instant::now();

    let result = engine.execute(sql).await;

    let query_elapsed = query_start.elapsed();

    match result {
        Ok(mut stream) => {
            use futures::StreamExt;
            let mut count = 0;
            while let Some(_row) = stream.next().await {
                count += 1;
            }

            tracing::info!("========================================");
            tracing::info!("Query completed successfully");
            tracing::info!("Total rows: {}", count);
            tracing::info!(
                "Total time: {:?} ({} ms)",
                query_elapsed,
                query_elapsed.as_millis()
            );
            tracing::info!("========================================");

            // Assert some basic expectations
            assert!(count > 0, "Should return some rows");

            // Log performance expectation
            if query_elapsed.as_millis() > 20 {
                tracing::warn!(
                    "Query took {}ms, which is above the 17ms current baseline. \
                     Check trace output above to identify bottlenecks.",
                    query_elapsed.as_millis()
                );
            }
        }
        Err(e) => {
            tracing::error!("Query failed: {:?}", e);
            panic!("Query execution failed: {:?}", e);
        }
    }
}

/// Setup test data with JSON properties.
///
/// Seeds through the storage layer rather than `INSERT INTO nodes` — DML is not
/// supported on the `nodes` pseudo-table, so the SQL form silently failed and
/// left the fixture empty. Every write is asserted so this cannot rot silently
/// again.
async fn setup_test_data(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    use raisin_storage::BranchRepository;

    let scope = StorageScope::new("tenant1", "repo1", "main", "default");

    // The branch must exist before any write, or its HEAD never advances.
    let _ = storage
        .branches()
        .create_branch(
            "tenant1",
            "repo1",
            "main",
            "test-user",
            None,
            None,
            false,
            false,
        )
        .await;

    for i in 1..=100 {
        let mut props = std::collections::HashMap::new();
        let mut put = |k: &str, v: String| {
            props.insert(k.to_string(), PropertyValue::String(v));
        };
        put("username", format!("user{}", i));
        put("email", format!("user{}@example.com", i));
        put("displayName", format!("User {} Display", i));
        put("avatar", format!("https://example.com/avatars/{}.jpg", i));
        put("bio", format!("This is user {} bio", i));
        put("location", format!("Location {}", i));
        put("website", format!("https://user{}.example.com", i));
        put("twitter", format!("@user{}", i));
        put("github", format!("user{}", i));
        put("status", "active".to_string());
        put("role", "member".to_string());
        put("department", "Engineering".to_string());
        put("title", "Software Engineer".to_string());
        put("phone", format!("+1-555-0{:03}", i));
        put("mobile", format!("+1-555-1{:03}", i));
        put("address", format!("{} Main St", i));
        put("city", "San Francisco".to_string());
        put("country", "USA".to_string());
        put("timezone", "America/Los_Angeles".to_string());
        put("language", "en".to_string());
        put("theme", "dark".to_string());
        put("notifications", "enabled".to_string());
        put("privacy", "public".to_string());
        put("subscription", "premium".to_string());
        put("verified", "true".to_string());
        put("last_login", format!("2024-01-{:02}T10:00:00Z", i % 30 + 1));

        let node = Node {
            id: format!("user_{}", i),
            path: format!("/user_{}", i),
            name: format!("User {}", i),
            node_type: "user".to_string(),
            archetype: Some("user".to_string()),
            properties: props,
            children: Vec::new(),
            order_key: String::new(),
            has_children: None,
            parent: None,
            version: 1,
            created_at: Some(chrono::Utc::now()),
            updated_at: Some(chrono::Utc::now()),
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: None,
            tenant_id: None,
            workspace: None,
            owner_id: None,
            relations: Vec::new(),
        };

        storage
            .nodes()
            .create(
                scope,
                node,
                CreateNodeOptions {
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .unwrap_or_else(|e| panic!("seeding user_{} failed: {:?}", i, e));
    }

    tracing::info!("Inserted 100 test user nodes with JSON properties");
}
