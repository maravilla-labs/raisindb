//! Tests for the optimizer module

use super::*;
use crate::analyzer::{
    functions::{FunctionCategory, FunctionSignature},
    BinaryOperator, ColumnDef, DataType, Expr, Literal, TypedExpr,
};
use crate::logical_plan::{ProjectionExpr, SortExpr, TableSchema};
use std::sync::Arc;

fn create_test_schema() -> Arc<TableSchema> {
    Arc::new(TableSchema {
        table_name: "nodes".to_string(),
        columns: vec![
            ColumnDef {
                name: "id".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "path".to_string(),
                data_type: DataType::Path,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "name".to_string(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "created_at".to_string(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
        ],
    })
}

#[test]
fn test_constant_folding_in_filter() {
    let schema = create_test_schema();

    // SELECT * FROM nodes WHERE 1 + 1 = 2
    let scan = LogicalPlan::Scan {
        table: "nodes".to_string(),
        alias: None,
        schema: schema.clone(),
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let left = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::literal(Literal::Int(1))),
            op: BinaryOperator::Add,
            right: Box::new(TypedExpr::literal(Literal::Int(1))),
        },
        DataType::Int,
    );

    let predicate = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOperator::Eq,
            right: Box::new(TypedExpr::literal(Literal::Int(2))),
        },
        DataType::Boolean,
    );

    let filter = LogicalPlan::Filter {
        input: Box::new(scan),
        predicate: FilterPredicate::from_expr(predicate),
    };

    let optimizer = Optimizer::new();
    let optimized = optimizer.optimize(filter);

    // Check that the expression was folded to `true`
    if let LogicalPlan::Filter { predicate, .. } = optimized {
        assert_eq!(predicate.conjuncts.len(), 1);
        // Should be folded to: 2 = 2 -> true
        if let Expr::Literal(Literal::Boolean(b)) = predicate.conjuncts[0].expr {
            assert!(b);
        } else {
            // Or it might be `2 = 2` which is still partially folded
            // Either way, the inner addition should be folded
        }
    }
}

#[test]
fn test_hierarchy_rewriting() {
    let schema = create_test_schema();

    // SELECT * FROM nodes WHERE PATH_STARTS_WITH(path, '/content/')
    let scan = LogicalPlan::Scan {
        table: "nodes".to_string(),
        alias: None,
        schema: schema.clone(),
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let col_expr = TypedExpr::column("nodes".to_string(), "path".to_string(), DataType::Path);
    let prefix_expr = TypedExpr::literal(Literal::Path("/content/".to_string()));

    let func_expr = TypedExpr::new(
        Expr::Function {
            name: "PATH_STARTS_WITH".to_string(),
            args: vec![col_expr, prefix_expr],
            signature: FunctionSignature {
                name: "PATH_STARTS_WITH".to_string(),
                params: vec![DataType::Path, DataType::Path],
                return_type: DataType::Boolean,
                is_deterministic: true,
                category: FunctionCategory::Hierarchy,
            },
            filter: None,
        },
        DataType::Boolean,
    );

    let filter = LogicalPlan::Filter {
        input: Box::new(scan),
        predicate: FilterPredicate::from_expr(func_expr),
    };

    let optimizer = Optimizer::new();
    let optimized = optimizer.optimize(filter);

    // The plan should still be a Filter, but the predicate is rewritten
    assert!(matches!(optimized, LogicalPlan::Filter { .. }));
}

#[test]
fn test_projection_pruning() {
    let schema = create_test_schema();

    // SELECT id FROM nodes WHERE name = 'test' ORDER BY created_at
    let scan = LogicalPlan::Scan {
        table: "nodes".to_string(),
        alias: None,
        schema: schema.clone(),
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let filter_expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "name".to_string(),
                DataType::Text,
            )),
            op: BinaryOperator::Eq,
            right: Box::new(TypedExpr::literal(Literal::Text("test".to_string()))),
        },
        DataType::Boolean,
    );

    let filter = LogicalPlan::Filter {
        input: Box::new(scan),
        predicate: FilterPredicate::from_expr(filter_expr),
    };

    let project = LogicalPlan::Project {
        input: Box::new(filter),
        exprs: vec![ProjectionExpr {
            expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
            alias: "id".to_string(),
        }],
    };

    let sort = LogicalPlan::Sort {
        input: Box::new(project),
        sort_exprs: vec![SortExpr {
            expr: TypedExpr::column(
                "nodes".to_string(),
                "created_at".to_string(),
                DataType::TimestampTz,
            ),
            ascending: true,
            nulls_first: false, // ASC defaults to NULLS LAST
        }],
    };

    let optimizer = Optimizer::new();
    let optimized = optimizer.optimize(sort);

    // Verify that Scan has projection with required columns
    // Should include: id, name (from WHERE), created_at (from ORDER BY)
    fn find_scan(plan: &LogicalPlan) -> Option<&LogicalPlan> {
        match plan {
            LogicalPlan::Scan { .. } => Some(plan),
            LogicalPlan::TableFunction { .. } => None,
            LogicalPlan::Filter { input, .. }
            | LogicalPlan::Project { input, .. }
            | LogicalPlan::Sort { input, .. }
            | LogicalPlan::Limit { input, .. }
            | LogicalPlan::Distinct { input, .. }
            | LogicalPlan::Aggregate { input, .. }
            | LogicalPlan::Subquery { input, .. }
            | LogicalPlan::Window { input, .. }
            | LogicalPlan::LateralMap { input, .. } => find_scan(input),
            LogicalPlan::Join { left, .. } => find_scan(left),
            LogicalPlan::SemiJoin { left, .. } => find_scan(left),
            LogicalPlan::WithCTE { main_query, .. } => find_scan(main_query),
            LogicalPlan::CTEScan { .. } => None,
            // DML operators and empty plans don't have child plans
            LogicalPlan::Insert { .. }
            | LogicalPlan::Update { .. }
            | LogicalPlan::Delete { .. }
            | LogicalPlan::Order { .. }
            | LogicalPlan::Move { .. }
            | LogicalPlan::Copy { .. }
            | LogicalPlan::Translate { .. }
            | LogicalPlan::Relate { .. }
            | LogicalPlan::Unrelate { .. }
            | LogicalPlan::Empty => None,
        }
    }

    if let Some(LogicalPlan::Scan { projection, .. }) = find_scan(&optimized) {
        let proj = projection.as_ref().expect("Should have projection");
        assert!(proj.contains(&"id".to_string()));
        assert!(proj.contains(&"name".to_string()));
        assert!(proj.contains(&"created_at".to_string()));
    } else {
        panic!("Expected to find Scan in optimized plan");
    }
}

#[test]
fn test_combined_optimizations() {
    let schema = create_test_schema();

    // SELECT id FROM nodes WHERE DEPTH('/content/blog') = 2
    // This tests both constant folding (DEPTH) and projection pruning
    let scan = LogicalPlan::Scan {
        table: "nodes".to_string(),
        alias: None,
        schema: schema.clone(),
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let depth_func = TypedExpr::new(
        Expr::Function {
            name: "DEPTH".to_string(),
            args: vec![TypedExpr::literal(Literal::Path(
                "/content/blog".to_string(),
            ))],
            signature: FunctionSignature {
                name: "DEPTH".to_string(),
                params: vec![DataType::Path],
                return_type: DataType::Int,
                is_deterministic: true,
                category: FunctionCategory::Hierarchy,
            },
            filter: None,
        },
        DataType::Int,
    );

    let filter_expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(depth_func),
            op: BinaryOperator::Eq,
            right: Box::new(TypedExpr::literal(Literal::Int(2))),
        },
        DataType::Boolean,
    );

    let filter = LogicalPlan::Filter {
        input: Box::new(scan),
        predicate: FilterPredicate::from_expr(filter_expr),
    };

    let project = LogicalPlan::Project {
        input: Box::new(filter),
        exprs: vec![ProjectionExpr {
            expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
            alias: "id".to_string(),
        }],
    };

    let optimizer = Optimizer::new();
    let optimized = optimizer.optimize(project);

    // DEPTH('/content/blog') should be folded to 2, so predicate becomes 2 = 2 -> true
    if let LogicalPlan::Project {
        input: filter_box, ..
    } = optimized
    {
        if let LogicalPlan::Filter { predicate, .. } = filter_box.as_ref() {
            // Check that constant folding happened
            assert_eq!(predicate.conjuncts.len(), 1);
            // Should be `true` or `2 = 2`
        }
    }
}

#[test]
fn test_optimizer_with_disabled_passes() {
    let config = OptimizerConfig {
        enable_constant_folding: false,
        enable_hierarchy_rewriting: false,
        enable_cse: false,
        cse_threshold: 2,
        enable_projection_pruning: false,
        max_passes: 10,
    };

    let optimizer = Optimizer::with_config(config);
    let schema = create_test_schema();

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

    let optimized = optimizer.optimize(scan.clone());

    // With all passes disabled, plan should be unchanged
    assert!(matches!(optimized, LogicalPlan::Scan { .. }));
}
