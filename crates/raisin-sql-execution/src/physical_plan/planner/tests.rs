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

// ── IN(...) index expansion ──────────────────────────────────────────────

fn scan_nodes(table: &str) -> LogicalPlan {
    LogicalPlan::Scan {
        table: table.to_string(),
        alias: None,
        schema: Arc::new(TableSchema {
            table_name: table.to_string(),
            columns: vec![],
        }),
        filter: None,
        projection: None,
        workspace: None,
        max_revision: None,
        branch_override: None,
        locales: vec![],
    }
}

fn plan_with_in(table: &str, lhs: TypedExpr, values: Vec<Literal>) -> PhysicalPlan {
    use raisin_sql::analyzer::Expr;
    let in_expr = TypedExpr::new(
        Expr::InList {
            expr: Box::new(lhs),
            list: values.into_iter().map(TypedExpr::literal).collect(),
            negated: false,
        },
        DataType::Boolean,
    );
    let filter = LogicalPlan::Filter {
        input: Box::new(scan_nodes(table)),
        predicate: FilterPredicate::from_expr(in_expr),
    };
    PhysicalPlanner::new().plan(&filter).unwrap()
}

#[test]
fn test_path_in_expands_to_union_of_path_index_scans() {
    let physical = plan_with_in(
        "nodes",
        TypedExpr::column("nodes".to_string(), "path".to_string(), DataType::Text),
        vec![
            Literal::Text("/a".to_string()),
            Literal::Text("/b".to_string()),
        ],
    );
    let explain = physical.explain();
    assert!(explain.contains("Union: 2 branch(es)"), "{}", explain);
    assert_eq!(explain.matches("PathIndexScan").count(), 2, "{}", explain);
}

#[test]
fn test_id_in_expands_to_union_of_node_id_scans() {
    let physical = plan_with_in(
        "nodes",
        TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
        vec![
            Literal::Text("id1".to_string()),
            Literal::Text("id2".to_string()),
            Literal::Text("id3".to_string()),
        ],
    );
    let explain = physical.explain();
    assert!(explain.contains("Union: 3 branch(es)"), "{}", explain);
    assert_eq!(explain.matches("NodeIdScan").count(), 3, "{}", explain);
}

#[test]
fn test_node_type_in_expands_to_union_of_property_index_scans() {
    let physical = plan_with_in(
        "nodes",
        TypedExpr::column("nodes".to_string(), "node_type".to_string(), DataType::Text),
        vec![
            Literal::Text("post".to_string()),
            Literal::Text("comment".to_string()),
        ],
    );
    let explain = physical.explain();
    assert!(explain.contains("Union: 2 branch(es)"), "{}", explain);
    assert_eq!(
        explain.matches("PropertyIndexScan").count(),
        2,
        "{}",
        explain
    );
    assert_eq!(
        explain.matches("__node_type=post").count(),
        1,
        "{}",
        explain
    );
    assert_eq!(
        explain.matches("__node_type=comment").count(),
        1,
        "{}",
        explain
    );
}

#[test]
fn test_json_property_in_expands_to_union_of_property_index_scans() {
    use raisin_sql::analyzer::Expr;
    let lhs = TypedExpr::new(
        Expr::JsonExtractText {
            object: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "properties".to_string(),
                DataType::JsonB,
            )),
            key: Box::new(TypedExpr::literal(Literal::Text("status".to_string()))),
        },
        DataType::Nullable(Box::new(DataType::Text)),
    );
    let physical = plan_with_in(
        "nodes",
        lhs,
        vec![
            Literal::Text("in_use".to_string()),
            Literal::Text("reserved".to_string()),
        ],
    );
    let explain = physical.explain();
    assert!(explain.contains("Union: 2 branch(es)"), "{}", explain);
    assert_eq!(
        explain.matches("PropertyIndexScan").count(),
        2,
        "{}",
        explain
    );
}

#[test]
fn test_path_in_deduplicates_and_collapses_single_branch() {
    // path IN ('/a', '/a') → a single PathIndexScan (no Union).
    let physical = plan_with_in(
        "nodes",
        TypedExpr::column("nodes".to_string(), "path".to_string(), DataType::Text),
        vec![
            Literal::Text("/a".to_string()),
            Literal::Text("/a".to_string()),
        ],
    );
    let explain = physical.explain();
    assert!(!explain.contains("Union"), "{}", explain);
    assert_eq!(explain.matches("PathIndexScan").count(), 1, "{}", explain);
}

// ── OR folding / BETWEEN / ranges / pseudo-properties ────────────────────

fn plan_with_filter(table: &str, filter_expr: TypedExpr) -> PhysicalPlan {
    let filter = LogicalPlan::Filter {
        input: Box::new(scan_nodes(table)),
        predicate: FilterPredicate::from_expr(filter_expr),
    };
    PhysicalPlanner::new().plan(&filter).unwrap()
}

fn col_eq(table: &str, column: &str, value: Literal) -> TypedExpr {
    use raisin_sql::analyzer::Expr;
    TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::column(
                table.to_string(),
                column.to_string(),
                DataType::Text,
            )),
            op: raisin_sql::analyzer::BinaryOperator::Eq,
            right: Box::new(TypedExpr::literal(value)),
        },
        DataType::Boolean,
    )
}

fn or_expr(left: TypedExpr, right: TypedExpr) -> TypedExpr {
    use raisin_sql::analyzer::Expr;
    TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(left),
            op: raisin_sql::analyzer::BinaryOperator::Or,
            right: Box::new(right),
        },
        DataType::Boolean,
    )
}

#[test]
fn test_same_column_or_folds_to_union_of_path_scans() {
    // path = '/a' OR path = '/b' ≡ path IN ('/a','/b') → Union of PathIndexScans.
    let physical = plan_with_filter(
        "nodes",
        or_expr(
            col_eq("nodes", "path", Literal::Text("/a".to_string())),
            col_eq("nodes", "path", Literal::Text("/b".to_string())),
        ),
    );
    let explain = physical.explain();
    assert!(explain.contains("Union: 2 branch(es)"), "{}", explain);
    assert_eq!(explain.matches("PathIndexScan").count(), 2, "{}", explain);
}

#[test]
fn test_three_way_or_folds_to_union() {
    // node_type = 'a' OR node_type = 'b' OR node_type = 'c' → 3-branch Union.
    let physical = plan_with_filter(
        "nodes",
        or_expr(
            or_expr(
                col_eq("nodes", "node_type", Literal::Text("a".to_string())),
                col_eq("nodes", "node_type", Literal::Text("b".to_string())),
            ),
            col_eq("nodes", "node_type", Literal::Text("c".to_string())),
        ),
    );
    let explain = physical.explain();
    assert!(explain.contains("Union: 3 branch(es)"), "{}", explain);
    assert_eq!(
        explain.matches("PropertyIndexScan").count(),
        3,
        "{}",
        explain
    );
}

#[test]
fn test_heterogeneous_or_stays_table_scan() {
    // path = '/a' OR node_type = 'b' — branches can overlap, must NOT Union.
    let physical = plan_with_filter(
        "nodes",
        or_expr(
            col_eq("nodes", "path", Literal::Text("/a".to_string())),
            col_eq("nodes", "node_type", Literal::Text("b".to_string())),
        ),
    );
    let explain = physical.explain();
    assert!(!explain.contains("Union"), "{}", explain);
    assert!(explain.contains("TableScan"), "{}", explain);
}

#[test]
fn test_created_at_between_uses_range_scan_with_both_bounds() {
    use raisin_sql::analyzer::Expr;
    let between = TypedExpr::new(
        Expr::Between {
            expr: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "created_at".to_string(),
                DataType::TimestampTz,
            )),
            low: Box::new(TypedExpr::literal(Literal::Int(100))),
            high: Box::new(TypedExpr::literal(Literal::Int(200))),
        },
        DataType::Boolean,
    );
    let physical = plan_with_filter("nodes", between);
    let explain = physical.explain();
    assert!(explain.contains("PropertyRangeScan"), "{}", explain);
    assert!(explain.contains("__created_at"), "{}", explain);
    // Both bounds present: ">= …100 AND <= …200"
    assert!(explain.contains(">="), "{}", explain);
    assert!(explain.contains("<="), "{}", explain);
}

#[test]
fn test_two_sided_range_keeps_both_bounds() {
    use raisin_sql::analyzer::Expr;
    // Regression: created_at >= 100 AND created_at <= 200 previously dropped
    // the second bound from the scan AND from the residual filter.
    let ge = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "created_at".to_string(),
                DataType::TimestampTz,
            )),
            op: raisin_sql::analyzer::BinaryOperator::GtEq,
            right: Box::new(TypedExpr::literal(Literal::Int(100))),
        },
        DataType::Boolean,
    );
    let le = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "created_at".to_string(),
                DataType::TimestampTz,
            )),
            op: raisin_sql::analyzer::BinaryOperator::LtEq,
            right: Box::new(TypedExpr::literal(Literal::Int(200))),
        },
        DataType::Boolean,
    );
    let and = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(ge),
            op: raisin_sql::analyzer::BinaryOperator::And,
            right: Box::new(le),
        },
        DataType::Boolean,
    );
    let physical = plan_with_filter("nodes", and);
    let explain = physical.explain();
    assert!(explain.contains("PropertyRangeScan"), "{}", explain);
    assert!(explain.contains(">="), "{}", explain);
    assert!(explain.contains("<="), "{}", explain);
}

#[test]
fn test_json_property_text_range_uses_property_range_scan() {
    use raisin_sql::analyzer::Expr;
    // properties->>'sku' > 'M' → lexicographic PropertyRangeScan on 'sku'.
    let extract = TypedExpr::new(
        Expr::JsonExtractText {
            object: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "properties".to_string(),
                DataType::JsonB,
            )),
            key: Box::new(TypedExpr::literal(Literal::Text("sku".to_string()))),
        },
        DataType::Nullable(Box::new(DataType::Text)),
    );
    let gt = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(extract),
            op: raisin_sql::analyzer::BinaryOperator::Gt,
            right: Box::new(TypedExpr::literal(Literal::Text("M".to_string()))),
        },
        DataType::Boolean,
    );
    let physical = plan_with_filter("nodes", gt);
    let explain = physical.explain();
    assert!(explain.contains("PropertyRangeScan"), "{}", explain);
    assert!(explain.contains("sku"), "{}", explain);
}

#[test]
fn test_archetype_equality_uses_property_index_scan() {
    let physical = plan_with_filter(
        "nodes",
        col_eq("nodes", "archetype", Literal::Text("cms:page".to_string())),
    );
    let explain = physical.explain();
    assert!(
        explain.contains("PropertyIndexScan: __archetype=cms:page"),
        "{}",
        explain
    );
}

// ── Compound-index prefix matching ───────────────────────────────────────

fn planner_with_compound_index() -> PhysicalPlanner {
    use raisin_models::nodes::properties::schema::{
        CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
    };
    let mut planner = PhysicalPlanner::new();
    planner.set_compound_indexes(vec![CompoundIndexDefinition {
        name: "grp_status_time".to_string(),
        columns: vec![
            CompoundIndexColumn {
                property: "group".to_string(),
                column_type: CompoundColumnType::String,
                ascending: None,
            },
            CompoundIndexColumn {
                property: "status".to_string(),
                column_type: CompoundColumnType::String,
                ascending: None,
            },
            CompoundIndexColumn {
                property: "__created_at".to_string(),
                column_type: CompoundColumnType::Timestamp,
                ascending: None,
            },
        ],
        has_order_column: true,
    }]);
    planner
}

fn json_eq(table: &str, key: &str, value: &str) -> TypedExpr {
    eq(
        json_ref(table, key),
        TypedExpr::literal(Literal::Text(value.to_string())),
    )
}

fn and(left: TypedExpr, right: TypedExpr) -> TypedExpr {
    use raisin_sql::analyzer::Expr;
    TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(left),
            op: raisin_sql::analyzer::BinaryOperator::And,
            right: Box::new(right),
        },
        DataType::Boolean,
    )
}

#[test]
fn test_compound_index_full_equality_match() {
    let planner = planner_with_compound_index();
    let filter = LogicalPlan::Filter {
        input: Box::new(scan_nodes("nodes")),
        predicate: FilterPredicate::from_expr(and(
            json_eq("nodes", "group", "/g1"),
            json_eq("nodes", "status", "open"),
        )),
    };
    let physical = planner.plan(&filter).unwrap();
    let explain = physical.explain();
    assert!(
        explain.contains("CompoundIndexScan: grp_status_time [group=/g1, status=open]"),
        "{}",
        explain
    );
}

#[test]
fn test_compound_index_prefix_match_uses_leading_column() {
    // Only `group` (the first equality column) is constrained — the index
    // should still be used as a prefix scan, with nothing dropped.
    let planner = planner_with_compound_index();
    let filter = LogicalPlan::Filter {
        input: Box::new(scan_nodes("nodes")),
        predicate: FilterPredicate::from_expr(json_eq("nodes", "group", "/g1")),
    };
    let physical = planner.plan(&filter).unwrap();
    let explain = physical.explain();
    assert!(
        explain.contains("CompoundIndexScan: grp_status_time [group=/g1]"),
        "{}",
        explain
    );
    assert!(!explain.contains("TableScan"), "{}", explain);
}

#[test]
fn test_compound_index_non_leading_column_no_prefix_match() {
    // Only `status` (the SECOND equality column) — a prefix match is not
    // possible; must fall back to the single-property index, not the compound.
    let planner = planner_with_compound_index();
    let filter = LogicalPlan::Filter {
        input: Box::new(scan_nodes("nodes")),
        predicate: FilterPredicate::from_expr(json_eq("nodes", "status", "open")),
    };
    let physical = planner.plan(&filter).unwrap();
    let explain = physical.explain();
    assert!(!explain.contains("CompoundIndexScan"), "{}", explain);
    assert!(explain.contains("PropertyIndexScan"), "{}", explain);
}

// ── COUNT pushdown over Union ────────────────────────────────────────────

#[test]
fn test_count_over_node_type_in_uses_summed_index_count() {
    use raisin_sql::analyzer::Expr;
    use raisin_sql::logical_plan::{AggregateExpr, AggregateFunction};

    let in_expr = TypedExpr::new(
        Expr::InList {
            expr: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "node_type".to_string(),
                DataType::Text,
            )),
            list: vec![
                TypedExpr::literal(Literal::Text("post".to_string())),
                TypedExpr::literal(Literal::Text("comment".to_string())),
            ],
            negated: false,
        },
        DataType::Boolean,
    );
    let filter = LogicalPlan::Filter {
        input: Box::new(scan_nodes("nodes")),
        predicate: FilterPredicate::from_expr(in_expr),
    };
    let agg = LogicalPlan::Aggregate {
        input: Box::new(filter),
        group_by: vec![],
        aggregates: vec![AggregateExpr {
            func: AggregateFunction::Count,
            args: vec![],
            alias: "count_star".to_string(),
            return_type: DataType::BigInt,
            filter: None,
        }],
    };
    let physical = PhysicalPlanner::new().plan(&agg).unwrap();
    let explain = physical.explain();
    assert!(
        explain.contains("PropertyIndexCountScan: __node_type=post | __node_type=comment"),
        "{}",
        explain
    );
}

// ── Expression join keys ─────────────────────────────────────────────────

fn json_ref(table: &str, key: &str) -> TypedExpr {
    use raisin_sql::analyzer::Expr;
    TypedExpr::new(
        Expr::JsonExtractText {
            object: Box::new(TypedExpr::column(
                table.to_string(),
                "properties".to_string(),
                DataType::JsonB,
            )),
            key: Box::new(TypedExpr::literal(Literal::Text(key.to_string()))),
        },
        DataType::Nullable(Box::new(DataType::Text)),
    )
}

fn plan_join(condition: TypedExpr) -> PhysicalPlan {
    let join = LogicalPlan::Join {
        left: Box::new(scan_nodes("a")),
        right: Box::new(scan_nodes("b")),
        join_type: raisin_sql::analyzer::JoinType::Inner,
        condition: Some(condition),
    };
    PhysicalPlanner::new().plan(&join).unwrap()
}

fn eq(left: TypedExpr, right: TypedExpr) -> TypedExpr {
    use raisin_sql::analyzer::Expr;
    TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(left),
            op: raisin_sql::analyzer::BinaryOperator::Eq,
            right: Box::new(right),
        },
        DataType::Boolean,
    )
}

#[test]
fn test_json_extract_join_key_uses_hash_join() {
    // a.properties->>'ref_id' = b.id → HashJoin, not NestedLoopJoin.
    let physical = plan_join(eq(
        json_ref("a", "ref_id"),
        TypedExpr::column("b".to_string(), "id".to_string(), DataType::Text),
    ));
    assert!(
        matches!(physical, PhysicalPlan::HashJoin { .. }),
        "expected HashJoin, got {}",
        physical.describe()
    );
}

#[test]
fn test_reversed_join_condition_assigns_sides_correctly() {
    // ON b.id = a.properties->>'ref' — operands are side-swapped in the SQL;
    // left_keys must still reference table `a` (the join's left input).
    let physical = plan_join(eq(
        TypedExpr::column("b".to_string(), "id".to_string(), DataType::Text),
        json_ref("a", "ref"),
    ));
    match physical {
        PhysicalPlan::HashJoin {
            left_keys,
            right_keys,
            ..
        } => {
            let left_str = format!("{:?}", left_keys[0]);
            let right_str = format!("{:?}", right_keys[0]);
            assert!(
                left_str.contains("\"a\""),
                "left key must be a's: {}",
                left_str
            );
            assert!(
                right_str.contains("\"b\""),
                "right key must be b's: {}",
                right_str
            );
        }
        other => panic!("expected HashJoin, got {}", other.describe()),
    }
}

#[test]
fn test_same_side_equality_falls_back_to_nested_loop() {
    // a.x = a.y references only one side — not a join key.
    let physical = plan_join(eq(
        TypedExpr::column("a".to_string(), "x".to_string(), DataType::Text),
        TypedExpr::column("a".to_string(), "y".to_string(), DataType::Text),
    ));
    assert!(
        matches!(physical, PhysicalPlan::NestedLoopJoin { .. }),
        "expected NestedLoopJoin, got {}",
        physical.describe()
    );
}

#[test]
fn test_name_in_expands_to_union_of_property_scans() {
    let physical = plan_with_in(
        "nodes",
        TypedExpr::column("nodes".to_string(), "name".to_string(), DataType::Text),
        vec![
            Literal::Text("a".to_string()),
            Literal::Text("b".to_string()),
        ],
    );
    let explain = physical.explain();
    assert!(explain.contains("Union: 2 branch(es)"), "{}", explain);
    assert!(explain.contains("__name=a"), "{}", explain);
    assert!(explain.contains("__name=b"), "{}", explain);
}
