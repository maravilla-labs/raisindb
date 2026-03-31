//! Integration tests for logical planning

use crate::analyzer::{Analyzer, DataType, StaticCatalog};
use crate::logical_plan::{LogicalPlan, PlanBuilder};

#[test]
fn test_simple_select() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer.analyze("SELECT id, name FROM nodes").unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Verify plan structure: Project(Scan)
    match plan {
        LogicalPlan::Project { input, exprs } => {
            assert_eq!(exprs.len(), 2);
            assert_eq!(exprs[0].alias, "id");
            assert_eq!(exprs[1].alias, "name");

            match *input {
                LogicalPlan::Scan { ref table, .. } => {
                    assert_eq!(table, "nodes");
                }
                _ => panic!("Expected Scan"),
            }
        }
        _ => panic!("Expected Project"),
    }
}

#[test]
fn test_select_with_where() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT id FROM nodes WHERE name = 'test'")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Verify plan structure: Project(Scan with filter)
    // With predicate pushdown, the filter is now inside the Scan node
    match plan {
        LogicalPlan::Project { input, .. } => match *input {
            LogicalPlan::Scan {
                ref table,
                ref filter,
                ..
            } => {
                assert_eq!(table, "nodes");
                assert!(filter.is_some(), "Filter should be pushed down to Scan");
            }
            _ => panic!("Expected Scan with filter"),
        },
        _ => panic!("Expected Project"),
    }
}

#[test]
fn test_select_with_order_by_limit() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT id, name FROM nodes ORDER BY created_at DESC LIMIT 10")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Verify plan structure: Limit(Sort(Project(Scan)))
    match plan {
        LogicalPlan::Limit {
            input,
            limit,
            offset,
        } => {
            assert_eq!(limit, 10);
            assert_eq!(offset, 0);

            match *input {
                LogicalPlan::Sort { input, sort_exprs } => {
                    assert_eq!(sort_exprs.len(), 1);
                    assert!(!sort_exprs[0].ascending); // DESC

                    match *input {
                        LogicalPlan::Project { .. } => {}
                        _ => panic!("Expected Project"),
                    }
                }
                _ => panic!("Expected Sort"),
            }
        }
        _ => panic!("Expected Limit"),
    }
}

#[test]
fn test_hierarchy_query_plan() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze(
            "SELECT id, DEPTH(path) as depth FROM nodes WHERE PATH_STARTS_WITH(path, '/content/')",
        )
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Plan should be: Project(Scan with filter)
    // With predicate pushdown, PATH_STARTS_WITH is now in the Scan filter
    match plan {
        LogicalPlan::Project { input, exprs } => {
            assert_eq!(exprs.len(), 2);
            assert_eq!(exprs[0].alias, "id");
            assert_eq!(exprs[1].alias, "depth");

            match *input {
                LogicalPlan::Scan { ref filter, .. } => {
                    assert!(filter.is_some(), "Filter should be pushed down to Scan");
                }
                _ => panic!("Expected Scan with filter"),
            }
        }
        _ => panic!("Expected Project"),
    }
}

#[test]
fn test_plan_explain() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT id, name FROM nodes WHERE id = 'test' ORDER BY name LIMIT 5")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    let explain = plan.explain();
    println!("Plan:\n{}", explain);

    // Should contain all operators
    // With predicate pushdown, Filter is now inside Scan, not a separate operator
    assert!(explain.contains("Limit"));
    assert!(explain.contains("Sort"));
    assert!(explain.contains("Project"));
    assert!(explain.contains("Scan"));
    // The filter predicate should be shown in the Scan operator
}

#[test]
fn test_plan_schema_propagation() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT id, name, path FROM nodes")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Get output schema
    let schema = plan.schema();
    assert_eq!(schema.len(), 3);
    assert_eq!(schema[0].name, "id");
    assert_eq!(schema[1].name, "name");
    assert_eq!(schema[2].name, "path");

    // Verify types
    assert_eq!(schema[0].data_type, DataType::Text);
    assert_eq!(schema[1].data_type, DataType::Text);
    assert_eq!(schema[2].data_type, DataType::Path);
}

#[test]
fn test_json_operations_plan() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT id, properties ->> 'title' AS title FROM nodes WHERE properties @> '{\"status\": \"published\"}'")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Should be Project(Scan with filter)
    // With predicate pushdown, the filter is now inside the Scan node
    match plan {
        LogicalPlan::Project { input, exprs } => {
            assert_eq!(exprs.len(), 2);
            assert_eq!(exprs[0].alias, "id");
            assert_eq!(exprs[1].alias, "title");

            match *input {
                LogicalPlan::Scan { ref filter, .. } => {
                    assert!(filter.is_some(), "Filter should be pushed down to Scan");
                }
                _ => panic!("Expected Scan with filter"),
            }
        }
        _ => panic!("Expected Project"),
    }
}

#[test]
fn test_multiple_order_by_columns() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    // Updated: removed workspace column, use node_type instead
    let analyzed = analyzer
        .analyze("SELECT id FROM nodes ORDER BY node_type ASC, path DESC")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    match plan {
        LogicalPlan::Sort { sort_exprs, .. } => {
            assert_eq!(sort_exprs.len(), 2);
            assert!(sort_exprs[0].ascending); // ASC
            assert!(!sort_exprs[1].ascending); // DESC
        }
        _ => panic!("Expected Sort"),
    }
}

#[test]
fn test_limit_without_order_by() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer.analyze("SELECT id FROM nodes LIMIT 100").unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Should be Limit(Project(Scan)) - no Sort
    match plan {
        LogicalPlan::Limit { input, .. } => match *input {
            LogicalPlan::Project { input, .. } => match *input {
                LogicalPlan::Scan { .. } => {}
                _ => panic!("Expected Scan"),
            },
            _ => panic!("Expected Project"),
        },
        _ => panic!("Expected Limit"),
    }
}

#[test]
fn test_offset_only() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer.analyze("SELECT id FROM nodes OFFSET 10").unwrap();
    let plan = planner.build(&analyzed).unwrap();

    match plan {
        LogicalPlan::Limit {
            limit,
            offset,
            input,
        } => {
            assert_eq!(limit, usize::MAX); // No limit specified
            assert_eq!(offset, 10);

            match *input {
                LogicalPlan::Project { .. } => {}
                _ => panic!("Expected Project"),
            }
        }
        _ => panic!("Expected Limit"),
    }
}

#[test]
fn test_wildcard_expansion() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer.analyze("SELECT * FROM nodes LIMIT 1").unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Get the projection
    match plan {
        LogicalPlan::Limit { input, .. } => match *input {
            LogicalPlan::Project { exprs, .. } => {
                // Should have expanded to all columns in the table
                assert!(exprs.len() > 10); // nodes table has many columns
            }
            _ => panic!("Expected Project"),
        },
        _ => panic!("Expected Limit"),
    }
}

#[test]
fn test_complex_where_clause() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    // Updated: use nodes table with 2 filters instead of 3
    let analyzed = analyzer
        .analyze("SELECT id FROM nodes WHERE node_type = 'document' AND version > 1")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // With predicate pushdown, the filter is now inside the Scan node
    match plan {
        LogicalPlan::Project { input, .. } => match *input {
            LogicalPlan::Scan { ref filter, .. } => {
                assert!(filter.is_some(), "Filter should be pushed down to Scan");
                // Both predicates (node_type = 'document' AND version > 1) reference only
                // the nodes table, so they should be pushed down
            }
            _ => panic!("Expected Scan with filter"),
        },
        _ => panic!("Expected Project"),
    }
}

#[test]
fn test_alias_in_projection() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT id AS node_id, name AS node_name FROM nodes")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    match plan {
        LogicalPlan::Project { exprs, .. } => {
            assert_eq!(exprs.len(), 2);
            assert_eq!(exprs[0].alias, "node_id");
            assert_eq!(exprs[1].alias, "node_name");
        }
        _ => panic!("Expected Project"),
    }
}

#[test]
fn test_function_in_projection() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT DEPTH(path) as d, PARENT(path) as p FROM nodes")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Get schema first before moving plan
    let schema = plan.schema();
    assert_eq!(schema[0].data_type, DataType::Int); // DEPTH returns Int
    assert_eq!(
        schema[1].data_type,
        DataType::Nullable(Box::new(DataType::Path))
    ); // PARENT returns Nullable(Path)

    match plan {
        LogicalPlan::Project { exprs, .. } => {
            assert_eq!(exprs.len(), 2);
            assert_eq!(exprs[0].alias, "d");
            assert_eq!(exprs[1].alias, "p");
        }
        _ => panic!("Expected Project"),
    }
}

#[test]
fn test_between_predicate() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT id FROM nodes WHERE version BETWEEN 1 AND 10")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    match plan {
        LogicalPlan::Project { input, .. } => match *input {
            LogicalPlan::Scan { ref filter, .. } => {
                assert!(filter.is_some(), "Filter should be pushed down to Scan");
            }
            _ => panic!("Expected Scan with filter"),
        },
        _ => panic!("Expected Project"),
    }
}

#[test]
fn test_in_list_predicate() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT id FROM nodes WHERE node_type IN ('document', 'folder', 'page')")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    match plan {
        LogicalPlan::Project { input, .. } => match *input {
            LogicalPlan::Scan { ref filter, .. } => {
                assert!(filter.is_some(), "Filter should be pushed down to Scan");
            }
            _ => panic!("Expected Scan with filter"),
        },
        _ => panic!("Expected Project"),
    }
}
