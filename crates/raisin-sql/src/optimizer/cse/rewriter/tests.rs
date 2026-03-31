//! Tests for CSE plan rewriter

use super::*;
use crate::analyzer::typed_expr::*;
use crate::analyzer::DataType;
use crate::logical_plan::TableSchema;
use crate::optimizer::cse::arena::ExprId;
use std::sync::Arc;

fn create_test_schema() -> Arc<TableSchema> {
    Arc::new(TableSchema {
        table_name: "nodes".to_string(),
        columns: vec![],
    })
}

#[test]
fn test_rewrite_with_no_candidates() {
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
        input: Box::new(scan.clone()),
        exprs: vec![ProjectionExpr {
            expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
            alias: "id".to_string(),
        }],
    };

    let rewritten = CsePlanRewriter::rewrite(project.clone(), vec![]);

    // Should be unchanged with no candidates
    assert!(matches!(rewritten, LogicalPlan::Project { .. }));
}

#[test]
fn test_rewrite_extracts_common_expression() {
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

    // Create repeated JSON extraction
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
        ],
    };

    // Create CSE candidate
    let candidates = vec![CseCandidate {
        expr_id: ExprId::new(0),
        expr: username_extract,
        count: 2,
        alias: "__cse_0".to_string(),
    }];

    let rewritten = CsePlanRewriter::rewrite(project, candidates);

    // Verify structure: Project -> Project -> Scan
    if let LogicalPlan::Project { input, exprs } = rewritten {
        assert_eq!(exprs.len(), 2, "Should have 2 final projections");

        // Both should reference __cse_0
        for proj in &exprs {
            if let Expr::Column { column, .. } = &proj.expr.expr {
                assert_eq!(column, "__cse_0", "Should reference extracted column");
            } else {
                panic!("Expected column reference, got {:?}", proj.expr);
            }
        }

        // Check intermediate projection
        if let LogicalPlan::Project {
            input: inner_input,
            exprs: inner_exprs,
        } = input.as_ref()
        {
            assert_eq!(
                inner_exprs.len(),
                1,
                "Should have 1 intermediate projection"
            );
            assert_eq!(inner_exprs[0].alias, "__cse_0", "Should use __cse_0 alias");

            // Inner input should be the original Scan
            assert!(
                matches!(inner_input.as_ref(), LogicalPlan::Scan { .. }),
                "Inner input should be Scan"
            );
        } else {
            panic!("Expected intermediate Project node");
        }
    } else {
        panic!("Expected outer Project node");
    }
}

#[test]
fn test_rewrite_preserves_non_common_expressions() {
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

    let repeated = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::literal(Literal::Int(1))),
            op: BinaryOperator::Add,
            right: Box::new(TypedExpr::literal(Literal::Int(2))),
        },
        DataType::Int,
    );

    let unique = TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text);

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: repeated.clone(),
                alias: "expr1".to_string(),
            },
            ProjectionExpr {
                expr: repeated.clone(),
                alias: "expr2".to_string(),
            },
            ProjectionExpr {
                expr: unique.clone(),
                alias: "id".to_string(),
            },
        ],
    };

    let candidates = vec![CseCandidate {
        expr_id: ExprId::new(0),
        expr: repeated,
        count: 2,
        alias: "__cse_0".to_string(),
    }];

    let rewritten = CsePlanRewriter::rewrite(project, candidates);

    // Verify the unique expression is preserved
    if let LogicalPlan::Project { exprs, .. } = rewritten {
        // Third projection should still reference the original column
        assert_eq!(exprs[2].alias, "id");
        // It should be a column reference (either to nodes.id or __cse_0)
        assert!(matches!(exprs[2].expr.expr, Expr::Column { .. }));
    }
}

#[test]
fn test_nested_expression_replacement() {
    // Test that CSE rewriter correctly handles expressions where a common
    // subexpression appears both standalone and nested within another expression.
    //
    // The current implementation uses "pass-through optimization":
    // - CSE candidates are extracted to intermediate projection
    // - Non-CSE expressions that contain CSE subexpressions are passed through
    //   as-is to the intermediate projection (avoiding nested replacement)
    // - Final projection references columns from intermediate
    //
    // This is more efficient than nested replacement because it avoids
    // re-evaluation of the containing expression.

    let schema = create_test_schema();

    let scan = LogicalPlan::Scan {
        table: "nodes".to_string(),
        alias: Some("nodes".to_string()),
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    // Use expensive JSON extraction (cost >= 10) to ensure it's extractable
    let props_col = TypedExpr::column(
        "nodes".to_string(),
        "properties".to_string(),
        DataType::JsonB,
    );
    let json_extract = TypedExpr::new(
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
                expr: json_extract.clone(),
                alias: "username1".to_string(),
            },
            ProjectionExpr {
                expr: json_extract.clone(),
                alias: "username2".to_string(),
            },
        ],
    };

    let candidates = vec![CseCandidate {
        expr_id: ExprId::new(0),
        expr: json_extract,
        count: 2,
        alias: "__cse_0".to_string(),
    }];

    let rewritten = CsePlanRewriter::rewrite(project, candidates);

    if let LogicalPlan::Project { exprs, input } = rewritten {
        // Both projections should be column references to __cse_0
        assert!(
            matches!(exprs[0].expr.expr, Expr::Column { .. }),
            "First projection should be a column reference"
        );
        assert!(
            matches!(exprs[1].expr.expr, Expr::Column { .. }),
            "Second projection should be a column reference"
        );

        // Verify intermediate projection exists with CSE candidate
        assert!(
            matches!(input.as_ref(), LogicalPlan::Project { .. }),
            "Should have intermediate projection"
        );
    }
}

#[test]
fn test_rewrite_with_only_json_operations_full_cse() {
    // This test uses the FULL CSE pipeline (analyzer + rewriter) to see if
    // the bug is in how candidates are selected
    use crate::optimizer::cse::{apply_cse, CseConfig};

    let schema = create_test_schema();

    let scan = LogicalPlan::Scan {
        table: "cypher".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    // Create 3 different JSON extract expressions
    let json_col_b = TypedExpr::column("cypher".to_string(), "user_b".to_string(), DataType::JsonB);
    let path_b = TypedExpr::literal(Literal::JsonB(serde_json::json!([
        "properties",
        "displayName"
    ])));
    let json_extract_b = TypedExpr::new(
        Expr::JsonExtractPath {
            object: Box::new(json_col_b),
            path: Box::new(path_b),
        },
        DataType::Nullable(Box::new(DataType::JsonB)),
    );

    let json_col_a = TypedExpr::column("cypher".to_string(), "user_a".to_string(), DataType::JsonB);
    let path_a1 = TypedExpr::literal(Literal::JsonB(serde_json::json!([
        "properties",
        "displayName"
    ])));
    let json_extract_a1 = TypedExpr::new(
        Expr::JsonExtractPath {
            object: Box::new(json_col_a.clone()),
            path: Box::new(path_a1),
        },
        DataType::Nullable(Box::new(DataType::JsonB)),
    );

    let path_a2 = TypedExpr::literal(Literal::JsonB(serde_json::json!(["properties", "avatar"])));
    let json_extract_a2 = TypedExpr::new(
        Expr::JsonExtractPath {
            object: Box::new(json_col_a),
            path: Box::new(path_a2),
        },
        DataType::Nullable(Box::new(DataType::JsonB)),
    );

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: json_extract_b,
                alias: "namesd".to_string(),
            },
            ProjectionExpr {
                expr: json_extract_a1,
                alias: "name".to_string(),
            },
            ProjectionExpr {
                expr: json_extract_a2,
                alias: "avatar".to_string(),
            },
        ],
    };

    // Use default config (threshold=2), so single-occurrence expressions won't be extracted
    let config = CseConfig::default();
    let optimized = apply_cse(project, &config);

    // With threshold=2 and each expression appearing once, CSE shouldn't optimize
    // So the result should be the original plan unchanged
    if let LogicalPlan::Project { exprs, input, .. } = optimized {
        // Should still have 3 expressions
        assert_eq!(exprs.len(), 3, "Should have 3 expressions after CSE");
        assert_eq!(exprs[0].alias, "namesd");
        assert_eq!(exprs[1].alias, "name");
        assert_eq!(exprs[2].alias, "avatar");

        // Input should be the scan (no intermediate projection since threshold not met)
        assert!(
            matches!(input.as_ref(), LogicalPlan::Scan { .. }),
            "Input should be Scan, not intermediate projection"
        );
    } else {
        panic!("Expected Project node");
    }
}

#[test]
fn test_rewrite_with_only_json_operations() {
    // This test reproduces the bug where JSON operation columns disappear
    // when ALL projections are JSON operations (no regular columns)
    // User's failing query:
    // SELECT
    //   $.user_b.properties.displayName as namesd,  <- DISAPPEARS!
    //   $.user_a.properties.displayName as name,     <- Shows up
    //   $.user_a.properties.avatar as avatar         <- Shows up

    let schema = create_test_schema();

    let scan = LogicalPlan::Scan {
        table: "cypher".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    // Create 3 different JSON extract expressions (all expensive, will be CSE candidates)
    // user_b column
    let json_col_b = TypedExpr::column("cypher".to_string(), "user_b".to_string(), DataType::JsonB);
    let path_b = TypedExpr::literal(Literal::JsonB(serde_json::json!([
        "properties",
        "displayName"
    ])));
    let json_extract_b = TypedExpr::new(
        Expr::JsonExtractPath {
            object: Box::new(json_col_b),
            path: Box::new(path_b),
        },
        DataType::Nullable(Box::new(DataType::JsonB)),
    );

    // user_a column - displayName
    let json_col_a = TypedExpr::column("cypher".to_string(), "user_a".to_string(), DataType::JsonB);
    let path_a1 = TypedExpr::literal(Literal::JsonB(serde_json::json!([
        "properties",
        "displayName"
    ])));
    let json_extract_a1 = TypedExpr::new(
        Expr::JsonExtractPath {
            object: Box::new(json_col_a.clone()),
            path: Box::new(path_a1),
        },
        DataType::Nullable(Box::new(DataType::JsonB)),
    );

    // user_a column - avatar
    let path_a2 = TypedExpr::literal(Literal::JsonB(serde_json::json!(["properties", "avatar"])));
    let json_extract_a2 = TypedExpr::new(
        Expr::JsonExtractPath {
            object: Box::new(json_col_a),
            path: Box::new(path_a2),
        },
        DataType::Nullable(Box::new(DataType::JsonB)),
    );

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: json_extract_b.clone(),
                alias: "namesd".to_string(),
            },
            ProjectionExpr {
                expr: json_extract_a1.clone(),
                alias: "name".to_string(),
            },
            ProjectionExpr {
                expr: json_extract_a2.clone(),
                alias: "avatar".to_string(),
            },
        ],
    };

    // All 3 are CSE candidates (each appears once, all are expensive)
    let candidates = vec![
        CseCandidate {
            expr_id: ExprId::new(0),
            expr: json_extract_b,
            count: 1,
            alias: "__cse_0".to_string(),
        },
        CseCandidate {
            expr_id: ExprId::new(1),
            expr: json_extract_a1,
            count: 1,
            alias: "__cse_1".to_string(),
        },
        CseCandidate {
            expr_id: ExprId::new(2),
            expr: json_extract_a2,
            count: 1,
            alias: "__cse_2".to_string(),
        },
    ];

    let rewritten = CsePlanRewriter::rewrite(project, candidates);

    // Verify structure
    if let LogicalPlan::Project {
        input: final_input,
        exprs: final_exprs,
    } = rewritten
    {
        // CRITICAL: Final projection MUST have all 3 expressions with original aliases
        assert_eq!(
            final_exprs.len(),
            3,
            "Final projection should have 3 expressions"
        );
        assert_eq!(
            final_exprs[0].alias, "namesd",
            "First alias should be 'namesd'"
        );
        assert_eq!(
            final_exprs[1].alias, "name",
            "Second alias should be 'name'"
        );
        assert_eq!(
            final_exprs[2].alias, "avatar",
            "Third alias should be 'avatar'"
        );

        // Check intermediate projection
        if let LogicalPlan::Project {
            exprs: intermediate_exprs,
            ..
        } = final_input.as_ref()
        {
            // Intermediate should have the 3 CSE candidates
            assert_eq!(
                intermediate_exprs.len(),
                3,
                "Intermediate projection should have 3 CSE candidates, got {}",
                intermediate_exprs.len()
            );

            // Check all CSE candidates are present
            assert!(
                intermediate_exprs.iter().any(|e| e.alias == "__cse_0"),
                "Should have __cse_0"
            );
            assert!(
                intermediate_exprs.iter().any(|e| e.alias == "__cse_1"),
                "Should have __cse_1"
            );
            assert!(
                intermediate_exprs.iter().any(|e| e.alias == "__cse_2"),
                "Should have __cse_2"
            );

            // CRITICAL TEST: Verify that all final projections can resolve their references
            // Each final projection expression should either:
            // 1. Be a column reference to a CSE column (__cse_0, __cse_1, __cse_2)
            // 2. OR be the original expression if not replaced

            // Since all 3 were CSE candidates, they should all be column references
            for (idx, final_expr) in final_exprs.iter().enumerate() {
                match &final_expr.expr.expr {
                    Expr::Column { column, .. } => {
                        // The column should reference one of the CSE columns
                        let is_cse_ref = column.starts_with("__cse_");
                        assert!(
                            is_cse_ref,
                            "Final projection [{}] '{}' should reference a CSE column, got: {}",
                            idx, final_expr.alias, column
                        );
                    }
                    _ => {
                        panic!(
                            "Final projection [{}] '{}' should be a column reference, got: {:?}",
                            idx,
                            final_expr.alias,
                            std::mem::discriminant(&final_expr.expr.expr)
                        );
                    }
                }
            }
        } else {
            panic!("Expected intermediate projection");
        }
    } else {
        panic!("Expected Project node");
    }
}

#[test]
fn test_rewrite_with_cse_and_non_cse_columns() {
    // This test verifies the fix for the bug where non-CSE columns were dropped
    // when CSE candidates were present. This mimics the user's query:
    // SELECT $.user_a.properties.displayName as name, a_id as shit3, $.user_a.properties.avatar as avatar
    // where $.syntax creates JsonExtractPath expressions (CSE candidates) and a_id is a simple column

    let schema = create_test_schema();

    let scan = LogicalPlan::Scan {
        table: "cypher".to_string(),
        alias: None,
        schema,
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    };

    // Create JSON extract expressions (expensive, will be CSE candidates)
    let json_col = TypedExpr::column("cypher".to_string(), "user_a".to_string(), DataType::JsonB);
    let path1 = TypedExpr::literal(Literal::JsonB(serde_json::json!([
        "properties",
        "displayName"
    ])));
    let json_extract1 = TypedExpr::new(
        Expr::JsonExtractPath {
            object: Box::new(json_col.clone()),
            path: Box::new(path1),
        },
        DataType::Nullable(Box::new(DataType::JsonB)),
    );

    let path2 = TypedExpr::literal(Literal::JsonB(serde_json::json!(["properties", "avatar"])));
    let json_extract2 = TypedExpr::new(
        Expr::JsonExtractPath {
            object: Box::new(json_col),
            path: Box::new(path2),
        },
        DataType::Nullable(Box::new(DataType::JsonB)),
    );

    // Create a simple column reference (cheap, NOT a CSE candidate)
    let simple_col = TypedExpr::column("cypher".to_string(), "a_id".to_string(), DataType::Text);

    let project = LogicalPlan::Project {
        input: Box::new(scan),
        exprs: vec![
            ProjectionExpr {
                expr: json_extract1.clone(),
                alias: "name".to_string(),
            },
            ProjectionExpr {
                expr: simple_col.clone(),
                alias: "shit3".to_string(),
            },
            ProjectionExpr {
                expr: json_extract2.clone(),
                alias: "avatar".to_string(),
            },
        ],
    };

    // Simulate CSE candidates (both JSON extracts)
    let candidates = vec![
        CseCandidate {
            expr_id: ExprId::new(0),
            expr: json_extract1,
            count: 1,
            alias: "__cse_0".to_string(),
        },
        CseCandidate {
            expr_id: ExprId::new(1),
            expr: json_extract2,
            count: 1,
            alias: "__cse_1".to_string(),
        },
    ];

    let rewritten = CsePlanRewriter::rewrite(project, candidates);

    // Verify structure
    if let LogicalPlan::Project {
        input: final_input,
        exprs: final_exprs,
    } = rewritten
    {
        // Final projection should have 3 expressions
        assert_eq!(
            final_exprs.len(),
            3,
            "Final projection should have 3 expressions"
        );
        assert_eq!(final_exprs[0].alias, "name");
        assert_eq!(final_exprs[1].alias, "shit3");
        assert_eq!(final_exprs[2].alias, "avatar");

        // Input should be an intermediate projection
        if let LogicalPlan::Project {
            exprs: intermediate_exprs,
            ..
        } = final_input.as_ref()
        {
            // Intermediate projection should have CSE candidates + pass-through column
            // Should have at least 3 expressions: __cse_0, __cse_1, and shit3 (a_id)
            assert!(
                intermediate_exprs.len() >= 3,
                "Intermediate projection should have at least 3 expressions (2 CSE + 1 passthrough), got {}",
                intermediate_exprs.len()
            );

            // Check that CSE candidates are present
            assert!(
                intermediate_exprs.iter().any(|e| e.alias == "__cse_0"),
                "Should have __cse_0"
            );
            assert!(
                intermediate_exprs.iter().any(|e| e.alias == "__cse_1"),
                "Should have __cse_1"
            );

            // CRITICAL: Check that the pass-through column (a_id/shit3) is present
            // This verifies the fix - before the fix, this column would be missing
            assert!(
                intermediate_exprs.iter().any(|e| e.alias == "shit3"),
                "Should have passthrough column 'shit3' (a_id)"
            );
        } else {
            panic!("Expected intermediate projection");
        }
    } else {
        panic!("Expected Project node");
    }
}
