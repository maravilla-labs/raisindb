//! Logical Planner Demo
//!
//! Demonstrates how to use the logical planner to convert SQL queries into logical plans.

use raisin_sql::analyzer::StaticCatalog;
use raisin_sql::{Analyzer, PlanBuilder};

fn main() {
    // Create the catalog and analyzer
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    // Example 1: Simple SELECT
    println!("=== Example 1: Simple SELECT ===");
    let sql1 = "SELECT id, name FROM nodes";
    demonstrate_query(&analyzer, &planner, sql1);

    // Example 2: SELECT with WHERE
    println!("\n=== Example 2: SELECT with WHERE ===");
    let sql2 = "SELECT id, name FROM nodes WHERE workspace = 'default'";
    demonstrate_query(&analyzer, &planner, sql2);

    // Example 3: SELECT with ORDER BY and LIMIT
    println!("\n=== Example 3: SELECT with ORDER BY and LIMIT ===");
    let sql3 = "SELECT id, name FROM nodes ORDER BY created_at DESC LIMIT 10";
    demonstrate_query(&analyzer, &planner, sql3);

    // Example 4: Complex hierarchical query
    println!("\n=== Example 4: Hierarchical query with functions ===");
    let sql4 = "SELECT id, DEPTH(path) as depth, PARENT(path) as parent_path FROM nodes WHERE PATH_STARTS_WITH(path, '/content/') ORDER BY depth ASC LIMIT 20";
    demonstrate_query(&analyzer, &planner, sql4);

    // Example 5: JSON operations
    println!("\n=== Example 5: JSON operations ===");
    let sql5 = "SELECT id, properties ->> 'title' AS title FROM nodes WHERE properties @> '{\"status\": \"published\"}'";
    demonstrate_query(&analyzer, &planner, sql5);

    // Example 6: Complex nested query
    println!("\n=== Example 6: Complex nested query ===");
    let sql6 = "SELECT id, name, DEPTH(path) as level FROM nodes WHERE workspace = 'default' AND node_type = 'document' AND version > 1 ORDER BY name ASC, created_at DESC LIMIT 100 OFFSET 20";
    demonstrate_query(&analyzer, &planner, sql6);
}

fn demonstrate_query(analyzer: &Analyzer, planner: &PlanBuilder, sql: &str) {
    println!("SQL: {}", sql);
    println!();

    // Step 1: Analyze (semantic analysis)
    match analyzer.analyze(sql) {
        Ok(analyzed) => {
            println!("✓ Analysis successful");

            // Step 2: Build logical plan
            match planner.build(&analyzed) {
                Ok(plan) => {
                    println!("✓ Logical plan created");
                    println!();
                    println!("Plan:");
                    println!("{}", plan.explain());
                    println!();

                    // Show output schema
                    let schema = plan.schema();
                    println!("Output Schema:");
                    for (i, col) in schema.iter().enumerate() {
                        println!("  {}. {} ({})", i + 1, col.name, col.data_type);
                    }
                }
                Err(e) => {
                    println!("✗ Planning failed: {}", e);
                }
            }
        }
        Err(e) => {
            println!("✗ Analysis failed: {}", e);
        }
    }
}
