//! Integration tests for physical plan execution
//!
//! These tests verify end-to-end query execution from SQL to results.

#[cfg(test)]
mod integration_tests {
    use crate::physical_plan::operators::{PhysicalPlan, ScanReason};
    use crate::physical_plan::planner::PhysicalPlanner;
    use crate::physical_plan::types::{from_property_value, to_property_value};
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_sql::analyzer::Literal;

    #[test]
    fn test_type_conversions_roundtrip() {
        // Test string
        let pv = PropertyValue::String("hello".to_string());
        let lit = from_property_value(&pv).unwrap();
        let back = to_property_value(&lit).unwrap();
        assert_eq!(pv, back);

        // Test float
        let pv = PropertyValue::Float(42.5);
        let lit = from_property_value(&pv).unwrap();
        let back = to_property_value(&lit).unwrap();
        assert_eq!(pv, back);

        // Test integer
        let pv = PropertyValue::Integer(42);
        let lit = from_property_value(&pv).unwrap();
        let back = to_property_value(&lit).unwrap();
        assert_eq!(pv, PropertyValue::Integer(42));

        // Test boolean
        let pv = PropertyValue::Boolean(true);
        let lit = from_property_value(&pv).unwrap();
        let back = to_property_value(&lit).unwrap();
        assert_eq!(pv, back);
    }

    #[test]
    fn test_physical_plan_description() {
        use raisin_sql::logical_plan::TableSchema;
        use std::sync::Arc;

        let plan = PhysicalPlan::TableScan {
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            workspace: "w1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        let desc = plan.describe();
        assert!(desc.contains("TableScan"));
        assert!(desc.contains("nodes"));
    }

    #[test]
    fn test_physical_plan_explain() {
        use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
        use raisin_sql::logical_plan::TableSchema;
        use std::sync::Arc;

        let scan = PhysicalPlan::TableScan {
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            workspace: "w1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        let filter = PhysicalPlan::Filter {
            input: Box::new(scan),
            predicates: vec![TypedExpr::new(
                Expr::Literal(Literal::Boolean(true)),
                DataType::Boolean,
            )],
        };

        let explain = filter.explain();
        assert!(explain.contains("Filter"));
        assert!(explain.contains("TableScan"));
    }

    #[test]
    fn test_planner_creates_physical_plan() {
        use raisin_sql::logical_plan::{LogicalPlan, TableSchema};
        use std::sync::Arc;

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

        let physical = planner.plan(&logical);
        assert!(physical.is_ok());
        assert!(matches!(physical.unwrap(), PhysicalPlan::TableScan { .. }));
    }

    #[test]
    fn test_workspace_context_extraction() {
        use raisin_sql::logical_plan::TableSchema;
        use std::sync::Arc;

        let plan = PhysicalPlan::TableScan {
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            workspace: "workspace1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        let ctx = plan.workspace_context();
        assert_eq!(ctx, Some(("tenant1", "repo1", "main", "workspace1")));
    }

    #[test]
    fn test_limit_describe() {
        use raisin_sql::logical_plan::TableSchema;
        use std::sync::Arc;

        let scan = PhysicalPlan::TableScan {
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            workspace: "w1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        let limit = PhysicalPlan::Limit {
            input: Box::new(scan),
            limit: 10,
            offset: 5,
        };

        let desc = limit.describe();
        assert_eq!(desc, "Limit: limit=10, offset=5");
    }

    // ── Residual scoping predicates ───────────────────────────────────────
    //
    // Both retrieval surfaces used to accept a scoping predicate and then throw
    // it away: `FULLTEXT_MATCH(..) AND node_type = 'X'` returned every fulltext
    // hit of every type, and `WHERE node_type = 'X' ORDER BY embedding <=> ..
    // LIMIT k` returned the global k nearest of any type. No error, no EXPLAIN
    // signal. These pin the residual `Filter` that must sit above each scan.

    fn text_lit(value: &str) -> raisin_sql::analyzer::TypedExpr {
        use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
        TypedExpr::new(
            Expr::Literal(Literal::Text(value.to_string())),
            DataType::Text,
        )
    }

    fn column(name: &str) -> raisin_sql::analyzer::TypedExpr {
        use raisin_sql::analyzer::{DataType, Expr, TypedExpr};
        TypedExpr::new(
            Expr::Column {
                table: String::new(),
                column: name.to_string(),
            },
            DataType::Text,
        )
    }

    fn node_type_eq(value: &str) -> raisin_sql::analyzer::TypedExpr {
        use raisin_sql::analyzer::{BinaryOperator, DataType, Expr, TypedExpr};
        TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(column("node_type")),
                op: BinaryOperator::Eq,
                right: Box::new(text_lit(value)),
            },
            DataType::Boolean,
        )
    }

    fn and_expr(
        left: raisin_sql::analyzer::TypedExpr,
        right: raisin_sql::analyzer::TypedExpr,
    ) -> raisin_sql::analyzer::TypedExpr {
        use raisin_sql::analyzer::{BinaryOperator, DataType, Expr, TypedExpr};
        TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::And,
                right: Box::new(right),
            },
            DataType::Boolean,
        )
    }

    fn scan_with_filter(
        filter: Option<raisin_sql::analyzer::TypedExpr>,
    ) -> raisin_sql::logical_plan::LogicalPlan {
        use raisin_sql::logical_plan::{LogicalPlan, TableSchema};
        use std::sync::Arc;
        LogicalPlan::Scan {
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter,
            projection: None,
            workspace: Some("default".to_string()),
            max_revision: None,
            branch_override: None,
            locales: vec![],
        }
    }

    fn cosine_distance_sort() -> raisin_sql::logical_plan::SortExpr {
        use raisin_sql::analyzer::{BinaryOperator, DataType, Expr, Literal, TypedExpr};
        use raisin_sql::logical_plan::SortExpr;
        SortExpr {
            expr: TypedExpr::new(
                Expr::BinaryOp {
                    left: Box::new(TypedExpr::new(
                        Expr::Column {
                            table: String::new(),
                            column: "embedding".to_string(),
                        },
                        DataType::Vector(3),
                    )),
                    op: BinaryOperator::VectorCosineDistance,
                    right: Box::new(TypedExpr::new(
                        Expr::Literal(Literal::Vector(vec![0.1, 0.2, 0.3])),
                        DataType::Vector(3),
                    )),
                },
                DataType::Double,
            ),
            ascending: true,
            nulls_first: false,
        }
    }

    /// Does any residual `Filter` in the plan still mention this column?
    fn plan_filter_mentions(plan: &PhysicalPlan, needle: &str) -> bool {
        if let PhysicalPlan::Filter { predicates, .. } = plan {
            if predicates
                .iter()
                .any(|p| format!("{:?}", p).contains(needle))
            {
                return true;
            }
        }
        plan.inputs()
            .iter()
            .any(|i| plan_filter_mentions(i, needle))
    }

    fn find_node<'a>(
        plan: &'a PhysicalPlan,
        is_match: &dyn Fn(&PhysicalPlan) -> bool,
    ) -> Option<&'a PhysicalPlan> {
        if is_match(plan) {
            return Some(plan);
        }
        for input in plan.inputs() {
            if let Some(found) = find_node(input, is_match) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn fulltext_scan_keeps_its_scoping_predicate() {
        use raisin_sql::analyzer::functions::{FunctionCategory, FunctionSignature};
        use raisin_sql::analyzer::{DataType, Expr, TypedExpr};

        let fulltext = TypedExpr::new(
            Expr::Function {
                name: "FULLTEXT_MATCH".to_string(),
                args: vec![text_lit("mainsail"), text_lit("en")],
                signature: FunctionSignature {
                    name: "FULLTEXT_MATCH".to_string(),
                    params: vec![DataType::Text, DataType::Text],
                    return_type: DataType::Boolean,
                    is_deterministic: true,
                    category: FunctionCategory::FullText,
                },
                filter: None,
            },
            DataType::Boolean,
        );

        let planner = PhysicalPlanner::new();
        let logical = scan_with_filter(Some(and_expr(fulltext, node_type_eq("proof:Doc"))));
        let physical = planner.plan(&logical).expect("planning must succeed");

        // The fulltext index must still drive the access path...
        assert!(
            find_node(&physical, &|p| matches!(
                p,
                PhysicalPlan::FullTextScan { .. }
            ))
            .is_some(),
            "expected a FullTextScan in the plan, got: {}",
            physical.explain()
        );
        // ...but node_type has to survive as a row-level filter, or a scoped
        // search silently answers with the unscoped result set.
        assert!(
            plan_filter_mentions(&physical, "node_type"),
            "node_type predicate was dropped from the plan: {}",
            physical.explain()
        );
    }

    #[test]
    fn vector_knn_keeps_its_scoping_predicate() {
        use raisin_sql::logical_plan::LogicalPlan;

        // The filter is pushed INTO the scan, which is the shape the optimizer
        // actually produces — and the shape the k-NN planner used to ignore.
        let logical = LogicalPlan::Limit {
            input: Box::new(LogicalPlan::Sort {
                input: Box::new(scan_with_filter(Some(node_type_eq("proof:Doc")))),
                sort_exprs: vec![cosine_distance_sort()],
            }),
            limit: 5,
            offset: 0,
        };

        let planner = PhysicalPlanner::new();
        let physical = planner.plan(&logical).expect("planning must succeed");

        let vector_scan = find_node(&physical, &|p| matches!(p, PhysicalPlan::VectorScan { .. }));
        assert!(
            vector_scan.is_some(),
            "expected a VectorScan in the plan, got: {}",
            physical.explain()
        );
        assert!(
            plan_filter_mentions(&physical, "node_type"),
            "node_type predicate was dropped from the plan: {}",
            physical.explain()
        );

        // A residual filter runs after the index truncated to k, so the
        // candidate pool has to be widened or the query can come back empty.
        match vector_scan {
            Some(PhysicalPlan::VectorScan { k, overfetch, .. }) => {
                assert_eq!(*k, 5, "k must stay the user's LIMIT");
                assert!(
                    *overfetch > 1,
                    "a residual filter must widen the candidate pool, overfetch={}",
                    overfetch
                );
            }
            _ => unreachable!("checked above"),
        }
    }

    #[test]
    fn vector_knn_without_a_filter_does_not_overfetch() {
        use raisin_sql::logical_plan::LogicalPlan;

        let logical = LogicalPlan::Limit {
            input: Box::new(LogicalPlan::Sort {
                input: Box::new(scan_with_filter(None)),
                sort_exprs: vec![cosine_distance_sort()],
            }),
            limit: 5,
            offset: 0,
        };

        let planner = PhysicalPlanner::new();
        let physical = planner.plan(&logical).expect("planning must succeed");

        match physical {
            PhysicalPlan::VectorScan { k, overfetch, .. } => {
                assert_eq!(k, 5);
                assert_eq!(
                    overfetch, 1,
                    "an unfiltered k-NN must not pay for overfetch"
                );
            }
            other => panic!("expected a bare VectorScan, got: {}", other.explain()),
        }
    }
}
