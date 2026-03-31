use super::*;
use raisin_sql::logical_plan::{FilterPredicate, TableSchema};
use std::sync::Arc;

#[test]
fn test_planner_table_scan_no_filter() {
    let planner = PhysicalPlanner::new();
    let schema = Arc::new(TableSchema {
        table_name: "nodes".to_string(),
        columns: vec![],
    });

    let logical = LogicalPlan::Scan {
        table: "nodes".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let physical = planner.plan(&logical).unwrap();
    assert!(matches!(physical, PhysicalPlan::TableScan { .. }));
}

#[test]
fn test_planner_filter() {
    let planner = PhysicalPlanner::new();
    let schema = Arc::new(TableSchema {
        table_name: "nodes".to_string(),
        columns: vec![],
    });

    let scan = LogicalPlan::Scan {
        table: "nodes".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let filter = LogicalPlan::Filter {
        input: Box::new(scan),
        predicate: FilterPredicate::from_expr(TypedExpr::literal(Literal::Boolean(true))),
    };

    let physical = planner.plan(&filter).unwrap();
    // The planner optimizes Filter + Scan into a single TableScan with filter pushdown
    assert!(matches!(physical, PhysicalPlan::TableScan { .. }));
}

#[test]
fn test_planner_project() {
    let planner = PhysicalPlanner::new();
    let schema = Arc::new(TableSchema {
        table_name: "nodes".to_string(),
        columns: vec![],
    });

    let scan = LogicalPlan::Scan {
        table: "nodes".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![ProjectionExpr {
            expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
            alias: "id".to_string(),
        }],
    };

    let physical = planner.plan(&project).unwrap();
    assert!(matches!(physical, PhysicalPlan::Project { .. }));
}

#[test]
fn test_planner_property_order_scan() {
    use raisin_sql::analyzer::Expr;

    let planner = PhysicalPlanner::new();
    let schema = Arc::new(TableSchema {
        table_name: "social".to_string(),
        columns: vec![],
    });

    let scan = LogicalPlan::Scan {
        table: "social".to_string(),
        alias: Some("social".to_string()),
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let filter = LogicalPlan::Filter {
        input: Box::new(scan),
        predicate: FilterPredicate::from_expr(TypedExpr::literal(Literal::Boolean(true))),
    };

    let project = LogicalPlan::Project {
        input: Box::new(filter),
        exprs: vec![ProjectionExpr {
            expr: TypedExpr::column("social".to_string(), "path".to_string(), DataType::Text),
            alias: "path".to_string(),
        }],
    };

    let sort = LogicalPlan::Sort {
        input: Box::new(project),
        sort_exprs: vec![SortExpr {
            expr: TypedExpr::new(
                Expr::Column {
                    table: "social".to_string(),
                    column: "created_at".to_string(),
                },
                DataType::TimestampTz,
            ),
            ascending: false,
            nulls_first: true, // DESC defaults to NULLS FIRST
        }],
    };

    let logical = LogicalPlan::Limit {
        input: Box::new(sort),
        limit: 5,
        offset: 0,
    };

    let physical = planner.plan(&logical).unwrap();

    match physical {
        PhysicalPlan::Limit { limit, input, .. } => {
            assert_eq!(limit, 5);
            match input.as_ref() {
                PhysicalPlan::Project { input, .. } => match input.as_ref() {
                    PhysicalPlan::PropertyOrderScan {
                        property_name,
                        ascending,
                        ..
                    } => {
                        assert_eq!(property_name, "__created_at");
                        assert!(!ascending);
                    }
                    other => panic!("Expected PropertyOrderScan, got {:?}", other),
                },
                other => panic!("Expected Project, got {:?}", other),
            }
        }
        other => panic!("Expected Limit plan, got {:?}", other),
    }
}
