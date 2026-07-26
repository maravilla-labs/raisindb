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

use raisin_sql_execution::QueryEngine;
use raisin_storage_memory::InMemoryStorage;
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

    // Create in-memory storage for testing
    let storage = Arc::new(InMemoryStorage::default());

    // Create engine
    let engine = QueryEngine::new(storage.clone(), "tenant1", "repo1", "main");

    // Setup test data
    tracing::info!("Setting up test data...");
    setup_test_data(&engine).await;

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

/// Setup test data with JSON properties
async fn setup_test_data(engine: &QueryEngine<InMemoryStorage>) {
    // Insert test users with JSON properties
    for i in 1..=100 {
        let insert_sql = format!(
            r#"
            INSERT INTO nodes (id, name, node_type, properties) VALUES (
                'user_{}',
                'User {}',
                'user',
                '{{
                    "username": "user{}",
                    "email": "user{}@example.com",
                    "displayName": "User {} Display",
                    "avatar": "https://example.com/avatars/{}.jpg",
                    "bio": "This is user {} bio",
                    "location": "Location {}",
                    "website": "https://user{}.example.com",
                    "twitter": "@user{}",
                    "github": "user{}",
                    "status": "active",
                    "role": "member",
                    "department": "Engineering",
                    "title": "Software Engineer",
                    "phone": "+1-555-0{:03}",
                    "mobile": "+1-555-1{:03}",
                    "address": "{} Main St",
                    "city": "San Francisco",
                    "country": "USA",
                    "timezone": "America/Los_Angeles",
                    "language": "en",
                    "theme": "dark",
                    "notifications": "enabled",
                    "privacy": "public",
                    "subscription": "premium",
                    "verified": "true",
                    "last_login": "2024-01-{:02}T10:00:00Z"
                }}'
            )
            "#,
            i,
            i,
            i,
            i,
            i,
            i,
            i,
            i,
            i,
            i,
            i,
            i,
            i,
            i,
            i % 30 + 1
        );

        let _ = engine.execute(&insert_sql).await;
    }

    tracing::info!("Inserted 100 test user nodes with JSON properties");
}
