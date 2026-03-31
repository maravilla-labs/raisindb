use super::*;
use crate::analyzer::{BinaryOperator, ColumnDef, DataType, Expr, Literal, TypedExpr};
use crate::logical_plan::{FilterPredicate, LogicalPlan, ProjectionExpr, SortExpr, TableSchema};
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
                name: "name".to_string(),
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
                name: "created_at".to_string(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
        ],
    })
}

#[test]
fn test_extract_column_refs_simple() {
    let expr = TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text);
    let refs = extract_column_refs(&expr);
    assert_eq!(refs.len(), 1);
    assert!(refs.contains("id"));
}

#[test]
fn test_extract_column_refs_binary_op() {
    let left = TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text);
    let right = TypedExpr::literal(Literal::Text("test".to_string()));
    let expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(left),
            op: BinaryOperator::Eq,
            right: Box::new(right),
        },
        DataType::Boolean,
    );

    let refs = extract_column_refs(&expr);
    assert_eq!(refs.len(), 1);
    assert!(refs.contains("id"));
}

#[test]
fn test_extract_column_refs_function() {
    use crate::analyzer::functions::FunctionSignature;
    use crate::analyzer::FunctionCategory;

    let arg = TypedExpr::column("nodes".to_string(), "path".to_string(), DataType::Path);
    let expr = TypedExpr::new(
        Expr::Function {
            name: "DEPTH".to_string(),
            args: vec![arg],
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

    let refs = extract_column_refs(&expr);
    assert_eq!(refs.len(), 1);
    assert!(refs.contains("path"));
}

#[test]
fn test_compute_required_columns_project_and_filter() {
    // SELECT id, name FROM nodes WHERE path = '/content/'
    let schema = create_test_schema();
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

    let path_col = TypedExpr::column("nodes".to_string(), "path".to_string(), DataType::Path);
    let path_lit = TypedExpr::literal(Literal::Path("/content/".to_string()));
    let filter_expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(path_col),
            op: BinaryOperator::Eq,
            right: Box::new(path_lit),
        },
        DataType::Boolean,
    );

    let filter = LogicalPlan::Filter {
        input: Box::new(scan),
        predicate: FilterPredicate::from_expr(filter_expr),
    };

    let id_expr = ProjectionExpr {
        expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
        alias: "id".to_string(),
    };
    let name_expr = ProjectionExpr {
        expr: TypedExpr::column("nodes".to_string(), "name".to_string(), DataType::Text),
        alias: "name".to_string(),
    };

    let project = LogicalPlan::Project {
        input: Box::new(filter),
        exprs: vec![id_expr, name_expr],
    };

    let required = compute_required_columns(&project);
    assert_eq!(required.len(), 3);
    assert!(required.contains("id"));
    assert!(required.contains("name"));
    assert!(required.contains("path")); // from WHERE clause
}

#[test]
fn test_compute_required_columns_order_by() {
    // SELECT id, name FROM nodes ORDER BY created_at DESC
    let schema = create_test_schema();
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

    let id_expr = ProjectionExpr {
        expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
        alias: "id".to_string(),
    };
    let name_expr = ProjectionExpr {
        expr: TypedExpr::column("nodes".to_string(), "name".to_string(), DataType::Text),
        alias: "name".to_string(),
    };

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![id_expr, name_expr],
    };

    let sort_expr = SortExpr {
        expr: TypedExpr::column(
            "nodes".to_string(),
            "created_at".to_string(),
            DataType::TimestampTz,
        ),
        ascending: false,
        nulls_first: true, // DESC defaults to NULLS FIRST
    };

    let sort = LogicalPlan::Sort {
        input: Box::new(project),
        sort_exprs: vec![sort_expr],
    };

    let required = compute_required_columns(&sort);
    assert_eq!(required.len(), 3);
    assert!(required.contains("id"));
    assert!(required.contains("name"));
    assert!(required.contains("created_at")); // ORDER BY column even though not projected
}

#[test]
fn test_apply_projection_pruning() {
    // SELECT id FROM nodes WHERE name = 'test'
    let schema = create_test_schema();
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

    let name_col = TypedExpr::column("nodes".to_string(), "name".to_string(), DataType::Text);
    let name_lit = TypedExpr::literal(Literal::Text("test".to_string()));
    let filter_expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(name_col),
            op: BinaryOperator::Eq,
            right: Box::new(name_lit),
        },
        DataType::Boolean,
    );

    let filter = LogicalPlan::Filter {
        input: Box::new(scan),
        predicate: FilterPredicate::from_expr(filter_expr),
    };

    let id_expr = ProjectionExpr {
        expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
        alias: "id".to_string(),
    };

    let project = LogicalPlan::Project {
        input: Box::new(filter),
        exprs: vec![id_expr],
    };

    let optimized = apply_projection_pruning(project);

    // Verify the Scan has projection pushed down
    if let LogicalPlan::Project { input, .. } = optimized {
        if let LogicalPlan::Filter { input, .. } = input.as_ref() {
            if let LogicalPlan::Scan { projection, .. } = input.as_ref() {
                let proj = projection.as_ref().expect("Should have projection");
                assert_eq!(proj.len(), 2);
                assert!(proj.contains(&"id".to_string()));
                assert!(proj.contains(&"name".to_string())); // from WHERE
            } else {
                panic!("Expected Scan");
            }
        } else {
            panic!("Expected Filter");
        }
    } else {
        panic!("Expected Project");
    }
}

#[test]
fn test_projection_pruning_with_limit() {
    // SELECT id FROM nodes LIMIT 10
    let schema = create_test_schema();
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

    let id_expr = ProjectionExpr {
        expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
        alias: "id".to_string(),
    };

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![id_expr],
    };

    let limit = LogicalPlan::Limit {
        input: Box::new(project),
        limit: 10,
        offset: 0,
    };

    let optimized = apply_projection_pruning(limit);

    // Verify only 'id' column is read
    if let LogicalPlan::Limit { input, .. } = optimized {
        if let LogicalPlan::Project { input, .. } = input.as_ref() {
            if let LogicalPlan::Scan { projection, .. } = input.as_ref() {
                let proj = projection.as_ref().expect("Should have projection");
                assert_eq!(proj.len(), 1);
                assert!(proj.contains(&"id".to_string()));
            } else {
                panic!("Expected Scan");
            }
        } else {
            panic!("Expected Project");
        }
    } else {
        panic!("Expected Limit");
    }
}
