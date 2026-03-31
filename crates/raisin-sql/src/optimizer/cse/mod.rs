//! Common Subexpression Elimination (CSE) Optimizer
//!
//! This module implements CSE optimization for SQL queries, identifying and extracting
//! repeated expressions to avoid redundant computation.
//!
//! # Overview
//!
//! CSE optimization works in three phases:
//!
//! 1. **Analysis** - Scan projection expressions to find repeated subexpressions
//! 2. **Candidate Selection** - Filter expressions by frequency threshold
//! 3. **Rewriting** - Transform the plan to extract common expressions
//!
//! # Architecture
//!
//! The CSE module is organized into focused submodules:
//!
//! - **hasher** - Expression hashing for structural equality detection
//! - **analyzer** - Frequency analysis and candidate selection
//! - **rewriter** - Plan transformation logic
//! - **config** - Configuration and context types
//! - **apply** - Core CSE application logic
//! - **recursive** - Recursive CSE application

mod analyzer;
mod apply;
mod arena;
mod config;
mod hasher;
mod recursive;
mod rewriter;

// Re-export public types
pub use analyzer::CseCandidate;
pub use apply::apply_cse;
pub use arena::{ExprId, ExpressionArena};
pub use config::{CseConfig, CseContext};
pub use recursive::apply_cse_recursive;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{typed_expr::*, DataType, Expr, TypedExpr};
    use crate::logical_plan::LogicalPlan;
    use crate::logical_plan::{ProjectionExpr, TableSchema};
    use std::sync::Arc;

    fn create_test_schema() -> Arc<TableSchema> {
        Arc::new(TableSchema {
            table_name: "nodes".to_string(),
            columns: vec![],
        })
    }

    #[test]
    fn test_apply_cse_with_defaults() {
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
                    expr: username_extract,
                    alias: "username2".to_string(),
                },
            ],
        };

        let config = CseConfig::default();
        let optimized = apply_cse(project, &config);

        // Should create nested projections
        assert!(matches!(optimized, LogicalPlan::Project { .. }));
    }

    #[test]
    fn test_apply_cse_no_optimization() {
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
                    expr: TypedExpr::column(
                        "nodes".to_string(),
                        "name".to_string(),
                        DataType::Text,
                    ),
                    alias: "name".to_string(),
                },
            ],
        };

        let config = CseConfig::default();
        let optimized = apply_cse(project, &config);

        // Should remain unchanged (no common subexpressions)
        if let LogicalPlan::Project { input, .. } = optimized {
            assert!(matches!(input.as_ref(), LogicalPlan::Scan { .. }));
        }
    }

    #[test]
    fn test_apply_cse_recursive() {
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

        // Create repeated expression
        let add_expr = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(TypedExpr::literal(Literal::Int(1))),
                op: BinaryOperator::Add,
                right: Box::new(TypedExpr::literal(Literal::Int(2))),
            },
            DataType::Int,
        );

        // Inner projection with repeated expression
        let inner_project = LogicalPlan::Project {
            input: Box::new(scan),
            exprs: vec![
                ProjectionExpr {
                    expr: add_expr.clone(),
                    alias: "sum1".to_string(),
                },
                ProjectionExpr {
                    expr: add_expr.clone(),
                    alias: "sum2".to_string(),
                },
            ],
        };

        // Outer projection
        let outer_project = LogicalPlan::Project {
            input: Box::new(inner_project),
            exprs: vec![ProjectionExpr {
                expr: TypedExpr::column("nodes".to_string(), "sum1".to_string(), DataType::Int),
                alias: "result".to_string(),
            }],
        };

        let config = CseConfig::default();
        let optimized = apply_cse_recursive(outer_project, &config);

        // Should optimize the inner projection
        assert!(matches!(optimized, LogicalPlan::Project { .. }));
    }

    #[test]
    fn test_cse_config_custom_threshold() {
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

        // Use expensive JSON extraction (cost >= 10) instead of cheap arithmetic
        let props_col = TypedExpr::column(
            "nodes".to_string(),
            "properties".to_string(),
            DataType::JsonB,
        );
        let expr = TypedExpr::new(
            Expr::JsonExtractText {
                object: Box::new(props_col),
                key: Box::new(TypedExpr::literal(Literal::Text("key".to_string()))),
            },
            DataType::Nullable(Box::new(DataType::Text)),
        );

        // Expression appears exactly 2 times
        let project = LogicalPlan::Project {
            input: Box::new(scan),
            exprs: vec![
                ProjectionExpr {
                    expr: expr.clone(),
                    alias: "expr1".to_string(),
                },
                ProjectionExpr {
                    expr,
                    alias: "expr2".to_string(),
                },
            ],
        };

        // With threshold=2, should optimize (expression appears 2 times, meets cost threshold)
        let config2 = CseConfig { threshold: 2 };
        let optimized2 = apply_cse(project.clone(), &config2);
        if let LogicalPlan::Project { input, .. } = optimized2 {
            // Should have intermediate projection
            assert!(matches!(input.as_ref(), LogicalPlan::Project { .. }));
        }

        // With threshold=3, should NOT optimize (expression only appears 2 times)
        let config3 = CseConfig { threshold: 3 };
        let optimized3 = apply_cse(project, &config3);
        if let LogicalPlan::Project { input, .. } = optimized3 {
            // Should NOT have intermediate projection
            assert!(matches!(input.as_ref(), LogicalPlan::Scan { .. }));
        }
    }
}
