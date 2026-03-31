//! Physical Planner Demo
//!
//! Demonstrates the complete query pipeline by showing both logical and physical plans.
//! This example illustrates how the physical planner converts logical operators into
//! concrete execution strategies with intelligent scan selection.
//!
//! Run with: cargo run --example physical_planner_demo

use raisin_sql_execution::physical_plan::RocksDBIndexCatalog;
use raisin_sql_execution::{Analyzer, Optimizer, PhysicalPlanner, PlanBuilder, StaticCatalog};
use std::sync::Arc;

fn main() {
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║         RaisinSQL Physical Planner Demo                       ║");
    println!("║         Logical Plans → Physical Plans                        ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();

    // Setup components
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let logical_planner = PlanBuilder::new(&catalog);
    let optimizer = Optimizer::new();

    // Use RocksDB index catalog with all indexes available
    let index_catalog = Arc::new(RocksDBIndexCatalog::new());
    let physical_planner = PhysicalPlanner::with_catalog(
        "tenant1".into(),
        "repo1".into(),
        "main".into(),
        "workspace1".into(),
        index_catalog,
    );

    // Example 1: Simple SELECT with TableScan
    demonstrate_query(
        &analyzer,
        &logical_planner,
        &optimizer,
        &physical_planner,
        "SELECT id, name FROM nodes LIMIT 10",
        "Simple SELECT - Basic TableScan",
    );

    // Example 2: Prefix Scan for Hierarchy
    demonstrate_query(
        &analyzer,
        &logical_planner,
        &optimizer,
        &physical_planner,
        "SELECT id, path, name FROM nodes WHERE PATH_STARTS_WITH(path, '/content/blog/')",
        "Hierarchy Query - PrefixScan Optimization",
    );

    // Example 3: Property Index Scan
    demonstrate_query(
        &analyzer,
        &logical_planner,
        &optimizer,
        &physical_planner,
        "SELECT id, name FROM nodes WHERE properties @> '{\"status\": \"published\"}'",
        "Property Filter - PropertyIndexScan",
    );

    // Example 4: Full-Text Search
    demonstrate_query(
        &analyzer,
        &logical_planner,
        &optimizer,
        &physical_planner,
        "SELECT id, properties ->> 'title' as title FROM nodes WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'rust & performance')",
        "Full-Text Search - FullTextScan with Tantivy",
    );

    // Example 5: Complex Multi-Operator Query
    demonstrate_query(
        &analyzer,
        &logical_planner,
        &optimizer,
        &physical_planner,
        "SELECT id, name, DEPTH(path) as depth FROM nodes WHERE PATH_STARTS_WITH(path, '/content/') AND node_type = 'page' ORDER BY depth DESC LIMIT 20",
        "Complex Query - Multiple Optimizations",
    );

    // Example 6: Filter Pushdown vs Runtime Filter
    demonstrate_query(
        &analyzer,
        &logical_planner,
        &optimizer,
        &physical_planner,
        "SELECT id, path FROM nodes WHERE workspace = 'default' AND DEPTH(path) = 2",
        "Mixed Predicates - Pushdown vs Runtime Evaluation",
    );

    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║  Summary: Scan Selection Strategy                             ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();
    println!("The physical planner selects scans in priority order:");
    println!();
    println!("1. 🥇 FullTextScan      - to_tsvector @@ to_tsquery (Tantivy index)");
    println!("   → O(log n + k) complexity via inverted index");
    println!();
    println!("2. 🥈 PrefixScan        - PATH_STARTS_WITH(path, prefix)");
    println!("   → O(k) complexity via RocksDB path_index CF");
    println!();
    println!("3. 🥉 PropertyIndexScan - properties @> '{{\"key\": \"value\"}}'");
    println!("   → O(k) complexity via property_index CF");
    println!();
    println!("4. 🏅 TableScan         - Fallback with optional filter pushdown");
    println!("   → O(n) complexity, full table scan");
    println!();
    println!("Where:");
    println!("  n = total nodes in table");
    println!("  k = matching nodes (result set size)");
    println!();
    println!("✨ Runtime filters (like DEPTH) are evaluated after scan!");
    println!();
}

fn demonstrate_query(
    analyzer: &Analyzer,
    logical_planner: &PlanBuilder,
    optimizer: &Optimizer,
    physical_planner: &PhysicalPlanner,
    sql: &str,
    description: &str,
) {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📋 {}", description);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();
    println!("SQL:");
    println!("  {}", sql);
    println!();

    // Step 1: Parse and Analyze
    let analyzed = match analyzer.analyze(sql) {
        Ok(a) => a,
        Err(e) => {
            println!("❌ Analysis failed: {}", e);
            println!();
            return;
        }
    };

    // Step 2: Build Logical Plan
    let logical_plan = match logical_planner.build(&analyzed) {
        Ok(p) => p,
        Err(e) => {
            println!("❌ Logical planning failed: {}", e);
            println!();
            return;
        }
    };

    // Step 3: Optimize Logical Plan
    let optimized_plan = optimizer.optimize(logical_plan);

    // Step 4: Build Physical Plan
    let physical_plan = match physical_planner.plan(&optimized_plan) {
        Ok(p) => p,
        Err(e) => {
            println!("❌ Physical planning failed: {}", e);
            println!();
            return;
        }
    };

    // Display both plans side-by-side
    println!("📊 Logical Plan (Phase 3 - Optimized):");
    println!("{}", indent_output(&optimized_plan.explain(), 2));

    println!("🔧 Physical Plan (Phase 5):");
    println!("{}", indent_output(&physical_plan.explain(), 2));

    // Analyze optimizations
    print_optimizations(&optimized_plan, &physical_plan);

    println!();
}

fn indent_output(text: &str, spaces: usize) -> String {
    let prefix = " ".repeat(spaces);
    text.lines()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("{}{}", prefix, line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn print_optimizations(
    logical_plan: &raisin_sql::logical_plan::LogicalPlan,
    physical_plan: &raisin_sql_execution::physical_plan::PhysicalPlan,
) {
    println!("✨ Optimizations Applied:");

    // Detect scan type change
    let scan_type = detect_scan_type(physical_plan);
    match scan_type {
        ScanType::FullText => {
            println!("  • Full-text search detected → FullTextScan (Tantivy index)");
            println!("    Performance: O(log n + k) where k = result size");
        }
        ScanType::Prefix => {
            println!("  • Hierarchy predicate detected → PrefixScan (path_index CF)");
            println!("    Performance: O(k) where k = nodes under prefix");
        }
        ScanType::PropertyIndex => {
            println!("  • Property equality detected → PropertyIndexScan (property_index CF)");
            println!("    Performance: O(k) where k = matching nodes");
        }
        ScanType::Table => {
            println!("  • No index available → TableScan (full scan)");
            println!("    Performance: O(n) where n = total nodes");
        }
    }

    // Check for projection pruning
    if let Some(projection) = extract_projection(physical_plan) {
        if !projection.is_empty() {
            println!(
                "  • Projection pruning: only reading {} columns",
                projection.len()
            );
            println!("    Columns: [{}]", projection.join(", "));
        }
    }

    // Check for computed depth filter
    if has_depth_filter(logical_plan) {
        println!("  • DEPTH() computed at runtime (not indexed)");
        println!("    Applied as post-scan filter");
    }

    // Check for streaming vs blocking
    if has_sort_operator(physical_plan) {
        println!("  • Sort operator: blocking (materializes all rows)");
    } else {
        println!("  • Fully streaming execution (constant memory)");
    }
}

#[derive(Debug)]
enum ScanType {
    FullText,
    Prefix,
    PropertyIndex,
    Table,
}

fn detect_scan_type(plan: &raisin_sql_execution::physical_plan::PhysicalPlan) -> ScanType {
    use raisin_sql_execution::physical_plan::PhysicalPlan;

    match plan {
        PhysicalPlan::FullTextScan { .. } => ScanType::FullText,
        PhysicalPlan::PrefixScan { .. } => ScanType::Prefix,
        PhysicalPlan::PropertyIndexScan { .. } => ScanType::PropertyIndex,
        PhysicalPlan::TableScan { .. } => ScanType::Table,
        PhysicalPlan::Filter { input, .. }
        | PhysicalPlan::Project { input, .. }
        | PhysicalPlan::Sort { input, .. }
        | PhysicalPlan::TopN { input, .. }
        | PhysicalPlan::Limit { input, .. } => detect_scan_type(input),
        PhysicalPlan::NestedLoopJoin { left, .. } | PhysicalPlan::HashJoin { left, .. } => {
            detect_scan_type(left)
        }
        PhysicalPlan::NeighborsScan { .. } => ScanType::Table, // Default for neighbors scan
        _ => ScanType::Table,                                  // Default for other scan types
    }
}

fn extract_projection(
    plan: &raisin_sql_execution::physical_plan::PhysicalPlan,
) -> Option<Vec<String>> {
    use raisin_sql_execution::physical_plan::PhysicalPlan;

    match plan {
        PhysicalPlan::TableScan { projection, .. }
        | PhysicalPlan::PrefixScan { projection, .. }
        | PhysicalPlan::PropertyIndexScan { projection, .. }
        | PhysicalPlan::FullTextScan { projection, .. } => projection.clone(),
        PhysicalPlan::Filter { input, .. }
        | PhysicalPlan::Project { input, .. }
        | PhysicalPlan::Sort { input, .. }
        | PhysicalPlan::TopN { input, .. }
        | PhysicalPlan::Limit { input, .. } => extract_projection(input),
        PhysicalPlan::NestedLoopJoin { left, .. } | PhysicalPlan::HashJoin { left, .. } => {
            extract_projection(left)
        }
        PhysicalPlan::NeighborsScan { .. } => None, // No projection info for neighbors scan
        _ => None,                                  // Default for other plan types
    }
}

fn has_depth_filter(plan: &raisin_sql::logical_plan::LogicalPlan) -> bool {
    use raisin_sql::analyzer::Expr;
    use raisin_sql::logical_plan::LogicalPlan;

    match plan {
        LogicalPlan::Filter { predicate, input } => {
            let has_depth = predicate
                .conjuncts
                .iter()
                .any(|expr| matches!(&expr.expr, Expr::Function { name, .. } if name == "DEPTH"));
            has_depth || has_depth_filter(input)
        }
        LogicalPlan::Project { input, .. }
        | LogicalPlan::Sort { input, .. }
        | LogicalPlan::Limit { input, .. } => has_depth_filter(input),
        _ => false,
    }
}

fn has_sort_operator(plan: &raisin_sql_execution::physical_plan::PhysicalPlan) -> bool {
    use raisin_sql_execution::physical_plan::PhysicalPlan;

    match plan {
        PhysicalPlan::Sort { .. } => true,
        PhysicalPlan::TopN { .. } => true, // TopN is also blocking
        PhysicalPlan::Filter { input, .. }
        | PhysicalPlan::Project { input, .. }
        | PhysicalPlan::Limit { input, .. } => has_sort_operator(input),
        _ => false,
    }
}
