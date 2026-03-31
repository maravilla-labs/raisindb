use super::*;
use crate::analyzer::{Analyzer, StaticCatalog};
use crate::AnalyzedQuery;

#[test]
fn test_simple_scan() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer.analyze("SELECT * FROM nodes").unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Should be: Project(Scan)
    match plan {
        LogicalPlan::Project { input, .. } => match *input {
            LogicalPlan::Scan { ref table, .. } => {
                assert_eq!(table, "nodes");
            }
            _ => panic!("Expected Scan"),
        },
        _ => panic!("Expected Project"),
    }
}

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
        LogicalPlan::Project { input, exprs } => {
            assert_eq!(exprs.len(), 1);
            assert_eq!(exprs[0].alias, "id");

            match *input {
                LogicalPlan::Scan {
                    ref table,
                    ref filter,
                    ..
                } => {
                    assert_eq!(table, "nodes");
                    assert!(filter.is_some(), "Filter should be pushed down to Scan");
                }
                _ => panic!("Expected Scan with filter"),
            }
        }
        _ => panic!("Expected Project"),
    }
}

#[test]
fn test_select_with_order_by() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT id, name FROM nodes ORDER BY created_at DESC")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Verify plan structure: Sort(Project(Scan))
    match plan {
        LogicalPlan::Sort { input, sort_exprs } => {
            assert_eq!(sort_exprs.len(), 1);
            assert!(!sort_exprs[0].ascending); // DESC

            match *input {
                LogicalPlan::Project { input, .. } => match *input {
                    LogicalPlan::Scan { .. } => {}
                    _ => panic!("Expected Scan"),
                },
                _ => panic!("Expected Project"),
            }
        }
        _ => panic!("Expected Sort"),
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
fn test_select_with_limit_offset() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT id FROM nodes LIMIT 5 OFFSET 10")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Verify plan structure: Limit(Project(Scan))
    match plan {
        LogicalPlan::Limit {
            input,
            limit,
            offset,
        } => {
            assert_eq!(limit, 5);
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
fn test_complex_nested_plan() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    // Updated: keep WHERE clause but use different column
    let analyzed = analyzer
        .analyze("SELECT id, name FROM nodes WHERE version > 0 ORDER BY name ASC LIMIT 20 OFFSET 5")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Verify plan structure: Limit(Sort(Project(Scan with filter)))
    // With predicate pushdown, the filter is now inside the Scan node
    match plan {
        LogicalPlan::Limit { input, .. } => match *input {
            LogicalPlan::Sort { input, .. } => match *input {
                LogicalPlan::Project { input, .. } => match *input {
                    LogicalPlan::Scan { ref filter, .. } => {
                        assert!(filter.is_some(), "Filter should be pushed down to Scan");
                    }
                    _ => panic!("Expected Scan with filter"),
                },
                _ => panic!("Expected Project"),
            },
            _ => panic!("Expected Sort"),
        },
        _ => panic!("Expected Limit"),
    }
}

#[test]
fn test_workspace_table_plan() {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("any_workspace".to_string());
    let planner = PlanBuilder::new(&catalog);

    // Workspace existence is validated at analysis time (fail-fast)
    let analyzed = AnalyzedStatement::Query(AnalyzedQuery {
        ctes: vec![],
        projection: vec![],
        from: vec![crate::analyzer::TableRef {
            table: "any_workspace".to_string(),
            alias: None,
            workspace: None,
            table_function: None,
            subquery: None,
            lateral_function: None,
        }],
        joins: vec![],
        selection: None,
        group_by: vec![],
        aggregates: vec![],
        order_by: vec![],
        limit: None,
        offset: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
        distinct: None,
    });

    // Plan building should succeed for any workspace name
    let result = planner.build(&analyzed);
    assert!(
        result.is_ok(),
        "Plan should be created for any workspace name"
    );
}

#[test]
fn test_table_function_plan() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    let analyzed = analyzer
        .analyze("SELECT * FROM CYPHER('MATCH (n) RETURN n')")
        .expect("analysis should succeed for CYPHER function");
    let plan = planner
        .build(&analyzed)
        .expect("plan builder should handle table function");

    match plan {
        LogicalPlan::Project { input, .. } => match *input {
            LogicalPlan::TableFunction { ref name, .. } => {
                assert_eq!(name, "CYPHER");
            }
            other => panic!("expected table function plan, got {:?}", other),
        },
        other => panic!("expected projection over table function, got {:?}", other),
    }
}

#[test]
fn test_cross_join() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    // Comma-separated tables create a CROSS JOIN
    let analyzed = analyzer
        .analyze("SELECT * FROM nodes, nodes AS n2 LIMIT 10")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Verify plan structure: Limit(Project(Join(Scan, Scan)))
    match plan {
        LogicalPlan::Limit {
            input,
            limit,
            offset,
        } => {
            assert_eq!(limit, 10);
            assert_eq!(offset, 0);

            match *input {
                LogicalPlan::Project { input, .. } => match *input {
                    LogicalPlan::Join {
                        left,
                        right,
                        join_type,
                        condition,
                    } => {
                        // Verify it's a CROSS JOIN
                        assert_eq!(join_type, crate::analyzer::JoinType::Cross);
                        assert!(condition.is_none());

                        // Verify left and right are both scans
                        match *left {
                            LogicalPlan::Scan { ref table, .. } => {
                                assert_eq!(table, "nodes");
                            }
                            _ => panic!("Expected Scan on left side"),
                        }

                        match *right {
                            LogicalPlan::Scan { ref table, .. } => {
                                assert_eq!(table, "nodes");
                            }
                            _ => panic!("Expected Scan on right side"),
                        }
                    }
                    _ => panic!("Expected Join"),
                },
                _ => panic!("Expected Project"),
            }
        }
        _ => panic!("Expected Limit"),
    }
}

#[test]
fn test_multiple_joins() {
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    // Three tables: nodes, nodes AS n2, nodes AS n3
    let analyzed = analyzer
        .analyze("SELECT * FROM nodes, nodes AS n2, nodes AS n3")
        .unwrap();
    let plan = planner.build(&analyzed).unwrap();

    // Verify plan structure: Project(Join(Join(Scan, Scan), Scan))
    match plan {
        LogicalPlan::Project { input, .. } => match *input {
            LogicalPlan::Join { left, right, .. } => {
                // Second join should be on the left
                match *left {
                    LogicalPlan::Join { left, right, .. } => {
                        // First two tables
                        match *left {
                            LogicalPlan::Scan { ref table, .. } => {
                                assert_eq!(table, "nodes");
                            }
                            _ => panic!("Expected Scan"),
                        }
                        match *right {
                            LogicalPlan::Scan { ref table, .. } => {
                                assert_eq!(table, "nodes");
                            }
                            _ => panic!("Expected Scan"),
                        }
                    }
                    _ => panic!("Expected nested Join on left"),
                }

                // Third table
                match *right {
                    LogicalPlan::Scan { ref table, .. } => {
                        assert_eq!(table, "nodes");
                    }
                    _ => panic!("Expected Scan"),
                }
            }
            _ => panic!("Expected Join"),
        },
        _ => panic!("Expected Project"),
    }
}

#[test]
fn test_original_query_builds_logical_plan() {
    // This test demonstrates the progress on the original query from the user
    let catalog = StaticCatalog::default_nodes_schema();
    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let planner = PlanBuilder::new(&catalog);

    // The original query: SELECT * FROM test, wurst LIMIT 10
    // Using "nodes" instead since that's what the catalog has
    let analyzed = analyzer
        .analyze("SELECT * FROM nodes AS test, nodes AS wurst LIMIT 10")
        .unwrap();

    let plan = planner.build(&analyzed).unwrap();

    // Verify it builds successfully with a Join
    match plan {
        LogicalPlan::Limit {
            input,
            limit,
            offset,
        } => {
            assert_eq!(limit, 10);
            assert_eq!(offset, 0);

            // Should have Project(Join(...))
            match *input {
                LogicalPlan::Project { input, exprs } => {
                    // Should project all columns from both tables
                    assert!(exprs.len() > 0);

                    match *input {
                        LogicalPlan::Join { join_type, .. } => {
                            // Comma-separated tables create CROSS JOIN
                            assert_eq!(join_type, crate::analyzer::JoinType::Cross);
                            // Success! The logical plan builds correctly
                        }
                        _ => panic!("Expected Join node"),
                    }
                }
                _ => panic!("Expected Project"),
            }
        }
        _ => panic!("Expected Limit"),
    }
}
