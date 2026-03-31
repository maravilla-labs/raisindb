use super::*;
use crate::analyzer::{typed_expr::*, DataType, Expr};
use crate::logical_plan::{LogicalPlan, ProjectionExpr, TableSchema};
use std::sync::Arc;

fn create_test_schema() -> Arc<TableSchema> {
    Arc::new(TableSchema {
        table_name: "nodes".to_string(),
        columns: vec![],
    })
}

#[test]
fn test_no_duplicates_returns_empty() {
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

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
                alias: "id".to_string(),
            },
            ProjectionExpr {
                expr: TypedExpr::column("nodes".to_string(), "name".to_string(), DataType::Text),
                alias: "name".to_string(),
            },
        ],
    };

    let mut ctx =
        crate::optimizer::cse::CseContext::new(crate::optimizer::cse::CseConfig { threshold: 2 });
    let candidates = CseAnalyzer::analyze(&mut ctx, &project);

    assert_eq!(
        candidates.len(),
        0,
        "No duplicates should return no candidates"
    );
}

#[test]
fn test_duplicate_column_access_detected() {
    let schema = create_test_schema();

    let scan = LogicalPlan::Scan {
        table: "author".to_string(),
        alias: Some("author".to_string()),
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    // Repeated expression: author.properties
    let props_col = TypedExpr::column(
        "author".to_string(),
        "properties".to_string(),
        DataType::JsonB,
    );

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: props_col.clone(),
                alias: "author_props".to_string(),
            },
            ProjectionExpr {
                expr: TypedExpr::new(
                    Expr::JsonExtractText {
                        object: Box::new(props_col.clone()),
                        key: Box::new(TypedExpr::literal(Literal::Text("username".to_string()))),
                    },
                    DataType::Nullable(Box::new(DataType::Text)),
                ),
                alias: "username".to_string(),
            },
            ProjectionExpr {
                expr: TypedExpr::new(
                    Expr::JsonExtractText {
                        object: Box::new(props_col),
                        key: Box::new(TypedExpr::literal(Literal::Text("displayName".to_string()))),
                    },
                    DataType::Nullable(Box::new(DataType::Text)),
                ),
                alias: "displayName".to_string(),
            },
        ],
    };

    let mut ctx =
        crate::optimizer::cse::CseContext::new(crate::optimizer::cse::CseConfig { threshold: 2 });
    let candidates = CseAnalyzer::analyze(&mut ctx, &project);

    // Should NOT extract author.properties because it's a simple column
    // (Our is_extractable excludes simple columns)
    assert_eq!(
        candidates.len(),
        0,
        "Simple column references should not be extracted"
    );
}

#[test]
fn test_duplicate_json_extract_detected() {
    let schema = create_test_schema();

    let scan = LogicalPlan::Scan {
        table: "author".to_string(),
        alias: Some("author".to_string()),
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    // Create a repeated JSON extraction expression
    let props_col = TypedExpr::column(
        "author".to_string(),
        "properties".to_string(),
        DataType::JsonB,
    );
    let username_extract = TypedExpr::new(
        Expr::JsonExtractText {
            object: Box::new(props_col.clone()),
            key: Box::new(TypedExpr::literal(Literal::Text("username".to_string()))),
        },
        DataType::Nullable(Box::new(DataType::Text)),
    );

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: username_extract.clone(),
                alias: "username1".to_string(),
            },
            ProjectionExpr {
                expr: username_extract.clone(),
                alias: "username2".to_string(),
            },
            ProjectionExpr {
                expr: TypedExpr::new(
                    Expr::JsonExtractText {
                        object: Box::new(props_col),
                        key: Box::new(TypedExpr::literal(Literal::Text("displayName".to_string()))),
                    },
                    DataType::Nullable(Box::new(DataType::Text)),
                ),
                alias: "displayName".to_string(),
            },
        ],
    };

    let mut ctx =
        crate::optimizer::cse::CseContext::new(crate::optimizer::cse::CseConfig { threshold: 2 });
    let candidates = CseAnalyzer::analyze(&mut ctx, &project);

    // Should extract the repeated username extraction
    assert_eq!(candidates.len(), 1, "Should find one CSE candidate");
    assert_eq!(candidates[0].count, 2, "Should appear exactly 2 times");
    assert_eq!(
        candidates[0].alias, "__cse_0",
        "Should generate __cse_0 alias"
    );
}

#[test]
fn test_threshold_filtering() {
    let schema = create_test_schema();

    let scan = LogicalPlan::Scan {
        table: "author".to_string(),
        alias: Some("author".to_string()),
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let props_col = TypedExpr::column(
        "author".to_string(),
        "properties".to_string(),
        DataType::JsonB,
    );
    let username_extract = TypedExpr::new(
        Expr::JsonExtractText {
            object: Box::new(props_col),
            key: Box::new(TypedExpr::literal(Literal::Text("username".to_string()))),
        },
        DataType::Nullable(Box::new(DataType::Text)),
    );

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: username_extract.clone(),
                alias: "username1".to_string(),
            },
            ProjectionExpr {
                expr: username_extract,
                alias: "username2".to_string(),
            },
        ],
    };

    // With threshold=3, no candidates should be found
    let mut ctx =
        crate::optimizer::cse::CseContext::new(crate::optimizer::cse::CseConfig { threshold: 3 });
    let candidates = CseAnalyzer::analyze(&mut ctx, &project);

    assert_eq!(
        candidates.len(),
        0,
        "Should find no candidates with threshold=3"
    );
}

#[test]
fn test_nested_expressions() {
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

    // Create nested expression: (a + b) * 2
    let col_a = TypedExpr::column("nodes".to_string(), "a".to_string(), DataType::Int);
    let col_b = TypedExpr::column("nodes".to_string(), "b".to_string(), DataType::Int);
    let add_expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(col_a),
            op: BinaryOperator::Add,
            right: Box::new(col_b),
        },
        DataType::Int,
    );

    let mul_expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(add_expr.clone()),
            op: BinaryOperator::Multiply,
            right: Box::new(TypedExpr::literal(Literal::Int(2))),
        },
        DataType::Int,
    );

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: add_expr.clone(),
                alias: "sum".to_string(),
            },
            ProjectionExpr {
                expr: mul_expr.clone(),
                alias: "doubled".to_string(),
            },
            ProjectionExpr {
                expr: mul_expr,
                alias: "doubled2".to_string(),
            },
        ],
    };

    let mut ctx =
        crate::optimizer::cse::CseContext::new(crate::optimizer::cse::CseConfig { threshold: 2 });
    let candidates = CseAnalyzer::analyze(&mut ctx, &project);

    // Should find both (a+b) appearing 3 times and (a+b)*2 appearing 2 times
    assert!(
        candidates.len() >= 1,
        "Should find at least one CSE candidate"
    );
}

/// CRITICAL TEST: Volatile functions must NEVER be extracted
///
/// Bug without this fix:
/// SELECT random() as r1, random() as r2
/// Would extract to: SELECT __cse_0 as r1, __cse_0 as r2 WHERE __cse_0 = random()
/// Result: r1 == r2 (WRONG! Should be different random values)
#[test]
fn test_volatile_functions_not_extracted() {
    use crate::analyzer::functions::{FunctionCategory, FunctionSignature};

    let schema = create_test_schema();
    let scan = LogicalPlan::Scan {
        table: "test".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    // Create two calls to random() function (non-deterministic)
    let random_call1 = TypedExpr::new(
        Expr::Function {
            name: "random".to_string(),
            args: vec![],
            signature: FunctionSignature {
                name: "random".to_string(),
                params: vec![],
                return_type: DataType::Double,
                is_deterministic: false, // CRITICAL: marked as non-deterministic
                category: FunctionCategory::Scalar,
            },
            filter: None,
        },
        DataType::Double,
    );

    let random_call2 = random_call1.clone();

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: random_call1,
                alias: "r1".to_string(),
            },
            ProjectionExpr {
                expr: random_call2,
                alias: "r2".to_string(),
            },
        ],
    };

    let mut ctx =
        crate::optimizer::cse::CseContext::new(crate::optimizer::cse::CseConfig { threshold: 2 });
    let candidates = CseAnalyzer::analyze(&mut ctx, &project);

    // CRITICAL ASSERTION: random() should NOT be extracted
    assert_eq!(
        candidates.len(),
        0,
        "CRITICAL: Volatile functions like random() must NEVER be extracted by CSE"
    );
}

/// CRITICAL TEST: CASE expressions must not have branches extracted
///
/// Bug without this fix:
/// SELECT CASE WHEN false THEN 1/0 ELSE 1 END
/// Would extract 1/0, causing division by zero even though branch never executes
#[test]
fn test_case_expressions_not_extracted() {
    let schema = create_test_schema();
    let scan = LogicalPlan::Scan {
        table: "test".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    // CASE WHEN condition THEN result ELSE else_result END
    let case_expr = TypedExpr::new(
        Expr::Case {
            conditions: vec![(
                TypedExpr::literal(Literal::Boolean(false)),
                // This division by zero should NEVER execute
                TypedExpr::new(
                    Expr::BinaryOp {
                        left: Box::new(TypedExpr::literal(Literal::Int(1))),
                        op: BinaryOperator::Divide,
                        right: Box::new(TypedExpr::literal(Literal::Int(0))),
                    },
                    DataType::Int,
                ),
            )],
            else_expr: Some(Box::new(TypedExpr::literal(Literal::Int(1)))),
        },
        DataType::Int,
    );

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: case_expr.clone(),
                alias: "result1".to_string(),
            },
            ProjectionExpr {
                expr: case_expr,
                alias: "result2".to_string(),
            },
        ],
    };

    let mut ctx =
        crate::optimizer::cse::CseContext::new(crate::optimizer::cse::CseConfig { threshold: 2 });
    let candidates = CseAnalyzer::analyze(&mut ctx, &project);

    // CRITICAL ASSERTION: CASE expressions should NOT be extracted
    // This prevents lifting 1/0 out of the branch that never executes
    assert_eq!(
        candidates.len(),
        0,
        "CRITICAL: CASE expressions must not be extracted (short-circuit safety)"
    );
}

/// Test that deterministic expensive operations ARE extracted
#[test]
fn test_deterministic_expensive_operations_extracted() {
    let schema = create_test_schema();
    let scan = LogicalPlan::Scan {
        table: "test".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let col = TypedExpr::column("test".to_string(), "data".to_string(), DataType::JsonB);

    // JSON extraction - expensive and deterministic
    let json_extract = TypedExpr::new(
        Expr::JsonExtractText {
            object: Box::new(col),
            key: Box::new(TypedExpr::literal(Literal::Text("key".to_string()))),
        },
        DataType::Nullable(Box::new(DataType::Text)),
    );

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: json_extract.clone(),
                alias: "v1".to_string(),
            },
            ProjectionExpr {
                expr: json_extract,
                alias: "v2".to_string(),
            },
        ],
    };

    let mut ctx =
        crate::optimizer::cse::CseContext::new(crate::optimizer::cse::CseConfig { threshold: 2 });
    let candidates = CseAnalyzer::analyze(&mut ctx, &project);

    // Should extract expensive deterministic operations
    assert_eq!(
        candidates.len(),
        1,
        "Deterministic expensive operations like JSON extraction should be extracted"
    );
}

/// Test that cheap operations are NOT extracted (cost model)
#[test]
fn test_cheap_operations_not_extracted() {
    let schema = create_test_schema();
    let scan = LogicalPlan::Scan {
        table: "test".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    let a = TypedExpr::column("test".to_string(), "a".to_string(), DataType::Int);
    let one = TypedExpr::literal(Literal::Int(1));

    // a + 1 is very cheap (cost ~5), not worth extracting
    let cheap_expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(a),
            op: BinaryOperator::Add,
            right: Box::new(one),
        },
        DataType::Int,
    );

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: cheap_expr.clone(),
                alias: "v1".to_string(),
            },
            ProjectionExpr {
                expr: cheap_expr,
                alias: "v2".to_string(),
            },
        ],
    };

    let mut ctx =
        crate::optimizer::cse::CseContext::new(crate::optimizer::cse::CseConfig { threshold: 2 });
    let candidates = CseAnalyzer::analyze(&mut ctx, &project);

    // Should NOT extract cheap operations (cost < MIN_EXTRACTION_COST)
    assert_eq!(
        candidates.len(),
        0,
        "Cheap operations like 'a + 1' should NOT be extracted (cost model)"
    );
}

/// CRITICAL TEST: Cast(Column) patterns must NOT be extracted
///
/// Bug without this fix:
/// SELECT $.properties.bio, $.properties.avatar FROM social
/// Would extract Cast(properties -> JSONB) as __cse_0, but the
/// CSE column reference has resolution issues causing NULL values.
#[test]
fn test_cast_column_not_extracted() {
    let schema = create_test_schema();
    let scan = LogicalPlan::Scan {
        table: "social".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    // Create Cast(properties -> JSONB) which is common in $.properties.* expressions
    let properties_col = TypedExpr::column(
        "social".to_string(),
        "properties".to_string(),
        DataType::JsonB,
    );
    let cast_to_jsonb = TypedExpr::new(
        Expr::Cast {
            expr: Box::new(properties_col),
            target_type: DataType::JsonB,
        },
        DataType::JsonB,
    );

    // Create two JsonExtractPath expressions that share the Cast(properties)
    let path_bio = TypedExpr::new(
        Expr::Literal(Literal::JsonB(serde_json::json!(["bio"]))),
        DataType::JsonB,
    );
    let path_avatar = TypedExpr::new(
        Expr::Literal(Literal::JsonB(serde_json::json!(["avatar"]))),
        DataType::JsonB,
    );

    let json_extract_bio = TypedExpr::new(
        Expr::JsonExtractPath {
            object: Box::new(cast_to_jsonb.clone()),
            path: Box::new(path_bio),
        },
        DataType::Nullable(Box::new(DataType::JsonB)),
    );

    let json_extract_avatar = TypedExpr::new(
        Expr::JsonExtractPath {
            object: Box::new(cast_to_jsonb),
            path: Box::new(path_avatar),
        },
        DataType::Nullable(Box::new(DataType::JsonB)),
    );

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: json_extract_bio,
                alias: "bio".to_string(),
            },
            ProjectionExpr {
                expr: json_extract_avatar,
                alias: "avatar".to_string(),
            },
        ],
    };

    let mut ctx =
        crate::optimizer::cse::CseContext::new(crate::optimizer::cse::CseConfig { threshold: 2 });
    let candidates = CseAnalyzer::analyze(&mut ctx, &project);

    // CRITICAL ASSERTION: Cast(Column) should NOT be extracted
    // If it were extracted, it would cause NULL values due to column resolution issues
    assert_eq!(
        candidates.len(),
        0,
        "CRITICAL: Cast(Column) patterns must NOT be extracted by CSE - \
         this causes NULL values in multiple $.properties.* expressions"
    );
}
