//! Complete Query Execution Example
//!
//! This example demonstrates the full query pipeline from SQL to execution.
//!
//! To run this example (once storage is available):
//! ```bash
//! cargo run --example query_execution
//! ```

use raisin_sql_execution::{
    execute_plan, ExecutionContext, Optimizer, PhysicalPlanner, PlanBuilder, QueryPlan,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== RaisinSQL Query Execution Example ===\n");

    // Example SQL query
    let sql = r#"
        SELECT
            id,
            name,
            path,
            DEPTH(path) as depth,
            properties->>'status' as status
        FROM nodes
        WHERE PATH_STARTS_WITH(path, '/content/')
          AND properties->>'status' = 'published'
        ORDER BY name
        LIMIT 10
    "#;

    println!("SQL Query:\n{}\n", sql);

    // ========================================
    // Phase 1-4: Parse, Analyze, Plan, Optimize
    // ========================================

    println!("=== Compilation Pipeline ===\n");

    // Create complete query plan (includes all phases 1-4)
    let query_plan = QueryPlan::from_sql(sql)?;

    // Show the compilation pipeline
    println!("{}", query_plan.explain());

    // ========================================
    // Phase 5: Physical Planning
    // ========================================

    println!("\n=== Physical Planning ===\n");

    // Create physical planner with workspace context
    let physical_planner = PhysicalPlanner::with_context(
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        "workspace1".to_string(),
    );

    // Convert optimized logical plan to physical plan
    let physical_plan = physical_planner.plan(&query_plan.optimized)?;

    // Show physical plan
    println!("Physical Plan:\n{}", physical_plan.explain());

    // ========================================
    // Phase 5: Execution (requires storage)
    // ========================================

    println!("\n=== Execution ===\n");
    println!("Note: Execution requires RocksDB storage instance.");
    println!("This example shows the API - actual execution needs:");
    println!("  1. RocksDBStorage instance");
    println!("  2. TantivyIndexingEngine (for full-text search)");
    println!("  3. Populated data\n");

    // Example execution code (commented out until storage is available):
    /*
    // Create storage instance
    let storage = Arc::new(RocksDBStorage::new("./data")?);

    // Create Tantivy engine for full-text search with cache config
    let cache_config = raisin_indexer::IndexCacheConfig::development();
    let tantivy_engine = Arc::new(TantivyIndexingEngine::new(
        "./indexes".into(),
        cache_config.fulltext_cache_size
    )?);

    // Create execution context
    let ctx = ExecutionContext::new(
        storage,
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        "workspace1".to_string(),
    )
    .with_indexing_engine(tantivy_engine);

    // Execute query
    use futures::stream::StreamExt;
    let mut stream = execute_plan(&physical_plan, &ctx).await?;

    println!("Results:");
    println!("{:-<80}", "");

    let mut count = 0;
    while let Some(row_result) = stream.next().await {
        let row = row_result?;
        count += 1;

        println!(
            "{:3}. {} | {} | {} | depth={} | status={}",
            count,
            row.get("id").map(|v| format!("{:?}", v)).unwrap_or_else(|| "NULL".to_string()),
            row.get("name").map(|v| format!("{:?}", v)).unwrap_or_else(|| "NULL".to_string()),
            row.get("path").map(|v| format!("{:?}", v)).unwrap_or_else(|| "NULL".to_string()),
            row.get("depth").map(|v| format!("{:?}", v)).unwrap_or_else(|| "NULL".to_string()),
            row.get("status").map(|v| format!("{:?}", v)).unwrap_or_else(|| "NULL".to_string()),
        );
    }

    println!("{:-<80}", "");
    println!("Total rows: {}", count);
    */

    println!("Example API usage:");
    println!(
        r#"
    // Create storage and indexing engine
    let storage = Arc::new(RocksDBStorage::new("./data")?);
    let cache_config = raisin_indexer::IndexCacheConfig::development();
    let tantivy = Arc::new(TantivyIndexingEngine::new(
        "./indexes".into(),
        cache_config.fulltext_cache_size
    )?);

    // Create execution context
    let ctx = ExecutionContext::new(
        storage,
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        "workspace1".to_string(),
    )
    .with_indexing_engine(tantivy);

    // Execute query
    let mut stream = execute_plan(&physical_plan, &ctx).await?;

    // Process results
    while let Some(row) = stream.next().await {{
        let row = row?;
        println!("Row: {{:?}}", row);
    }}
    "#
    );

    Ok(())
}

// ========================================
// Helper: Alternative Query Examples
// ========================================

#[allow(dead_code)]
fn example_queries() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "Simple SELECT",
            "SELECT id, name FROM nodes WHERE depth = 2 LIMIT 10",
        ),
        (
            "Path Hierarchy",
            "SELECT * FROM nodes WHERE PATH_STARTS_WITH(path, '/content/blog/')",
        ),
        (
            "JSON Property",
            "SELECT id, properties->>'title' as title FROM nodes WHERE properties->>'status' = 'draft'",
        ),
        (
            "Full-Text Search",
            "SELECT id, name FROM nodes WHERE to_tsvector('english', name) @@ to_tsquery('english', 'database')",
        ),
        (
            "Complex Filter",
            "SELECT id, name, DEPTH(path) as depth FROM nodes WHERE DEPTH(path) BETWEEN 2 AND 4 AND name LIKE '%doc%'",
        ),
        (
            "Sorting",
            "SELECT id, name, created_at FROM nodes ORDER BY created_at DESC, name ASC LIMIT 20",
        ),
        (
            "Pagination",
            "SELECT id, name FROM nodes ORDER BY name LIMIT 10 OFFSET 20",
        ),
    ]
}
