//! Optimizer Demo
//!
//! This example demonstrates the query optimizer capabilities in raisin-sql.
//! Run with: cargo run --example optimizer_demo

use raisin_sql::QueryPlan;

fn main() {
    println!("=== RaisinSQL Optimizer Demo ===\n");

    // Example 1: Projection Pruning
    println!("Example 1: Projection Pruning with ORDER BY");
    println!("{}", "=".repeat(80));

    let sql1 =
        "SELECT id, name FROM nodes WHERE workspace = 'default' ORDER BY created_at DESC LIMIT 10";
    match QueryPlan::from_sql(sql1) {
        Ok(plan) => {
            println!("{}", plan.explain());
        }
        Err(e) => eprintln!("Error: {}", e),
    }

    println!("\n\n");

    // Example 2: Hierarchy Function Rewriting
    println!("Example 2: Hierarchy Function Optimization");
    println!("{}", "=".repeat(80));

    let sql2 = "SELECT id, name, path FROM nodes WHERE PATH_STARTS_WITH(path, '/content/blog/') AND DEPTH(path) = 3";
    match QueryPlan::from_sql(sql2) {
        Ok(plan) => {
            println!("{}", plan.explain());
        }
        Err(e) => eprintln!("Error: {}", e),
    }

    println!("\n\n");

    // Example 3: Constant Folding
    println!("Example 3: Constant Folding");
    println!("{}", "=".repeat(80));

    let sql3 = "SELECT id, name FROM nodes WHERE DEPTH('/content/blog') = 2 AND node_type = 'page'";
    match QueryPlan::from_sql(sql3) {
        Ok(plan) => {
            println!("{}", plan.explain());
            println!("\nNote: DEPTH('/content/blog') was folded to constant 2 at compile time!");
        }
        Err(e) => eprintln!("Error: {}", e),
    }

    println!("\n\n");

    // Example 4: PARENT Function Expansion
    println!("Example 4: PARENT Function Expansion");
    println!("{}", "=".repeat(80));

    let sql4 = "SELECT id, name, path FROM nodes WHERE PARENT(path) = '/content/blog'";
    match QueryPlan::from_sql(sql4) {
        Ok(plan) => {
            println!("{}", plan.explain());
            println!("\nNote: PARENT(path) = '/content/blog' expanded to:");
            println!("  1. PrefixRange(path, '/content/blog/') - RocksDB prefix scan");
            println!("  2. DepthEq(path, 3) - Filter to direct children only");
        }
        Err(e) => eprintln!("Error: {}", e),
    }

    println!("\n\n");

    // Example 5: Complex Query with Multiple Optimizations
    println!("Example 5: Complex Query - All Optimizations");
    println!("{}", "=".repeat(80));

    let sql5 = r#"
        SELECT id, name, DEPTH(path) as depth
        FROM nodes
        WHERE PATH_STARTS_WITH(path, '/content/')
          AND node_type = 'page'
          AND DEPTH(path) <= 3
        ORDER BY created_at DESC
        LIMIT 20
    "#;
    match QueryPlan::from_sql(sql5) {
        Ok(plan) => {
            println!("{}", plan.explain());
            println!("\nOptimizations applied:");
            println!("  - Projection pruning: Only reads [id, name, path, node_type, created_at]");
            println!("  - Hierarchy rewriting: PATH_STARTS_WITH → PrefixRange (RocksDB scan)");
            println!("  - Constant folding: DEPTH in filter (if literal path)");
        }
        Err(e) => eprintln!("Error: {}", e),
    }

    println!("\n\n");

    // Example 6: Disabled Optimization
    println!("Example 6: Optimization Disabled");
    println!("{}", "=".repeat(80));

    use raisin_sql::OptimizerConfig;

    let config = OptimizerConfig {
        enable_constant_folding: false,
        enable_hierarchy_rewriting: false,
        enable_cse: false,
        cse_threshold: 2,
        enable_projection_pruning: false,
        max_passes: 10,
    };

    let sql6 = "SELECT id FROM nodes WHERE PATH_STARTS_WITH(path, '/content/') LIMIT 5";
    match QueryPlan::from_sql_with_config(sql6, config) {
        Ok(plan) => {
            println!("{}", plan.explain());
            println!("\nNote: With all optimizations disabled, the plan remains unchanged.");
        }
        Err(e) => eprintln!("Error: {}", e),
    }

    println!("\n\n=== Demo Complete ===");
}
