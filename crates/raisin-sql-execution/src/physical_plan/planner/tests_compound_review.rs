//! Compound planning gates found by the Phase 8 review: the build's history
//! floor, and date-like literals whose index match differs from a row-level
//! comparison.

use super::tests::{and, json_eq, scan_nodes};
use super::*;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_sql::logical_plan::FilterPredicate;
use raisin_storage::compound::{CompoundAvailability, CompoundStateSource};

/// Every index `Ready`, built (history floor) at `self.0`.
#[derive(Debug)]
struct BuiltAt(HLC);

impl CompoundStateSource for BuiltAt {
    fn compound_availability(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        definition: &CompoundIndexDefinition,
    ) -> CompoundAvailability {
        CompoundAvailability::Ready {
            built_through: self.0,
            definition_hash: definition.definition_hash(),
        }
    }
}

fn planner(read: Option<HLC>) -> PhysicalPlanner {
    let catalog = crate::physical_plan::catalog::RocksDBIndexCatalog::new()
        .with_optional_compound_state(Some(std::sync::Arc::new(BuiltAt(HLC::new(100, 0)))));
    let mut planner = PhysicalPlanner::with_catalog(
        "default".into(),
        "default".into(),
        "main".into(),
        "default".into(),
        std::sync::Arc::new(catalog),
    );
    let column = |property: &str| CompoundIndexColumn {
        property: property.to_string(),
        column_type: CompoundColumnType::String,
        ascending: None,
    };
    planner.set_compound_indexes(vec![CompoundIndexDefinition {
        name: "grp_status".to_string(),
        columns: vec![column("group"), column("status")],
        has_order_column: false,
        owner: None,
    }]);
    planner.set_read_revision(read);
    planner
}

fn plan(read: Option<HLC>, status: &str) -> PhysicalPlan {
    let filter = LogicalPlan::Filter {
        input: Box::new(scan_nodes("nodes")),
        predicate: FilterPredicate::from_expr(and(
            json_eq("nodes", "group", "/g1"),
            json_eq("nodes", "status", status),
        )),
    };
    planner(read).plan(&filter).unwrap()
}

/// A build re-derives the keyspace as of a HEAD and keeps no history below
/// it: a `__revision = N` read below that floor is answered by a scan, never
/// by an index that lost the tombstones and older versions it would need.
#[test]
fn time_travel_below_the_build_floor_is_not_index_served() {
    let index_served = |read| plan(read, "open").explain().contains("CompoundIndexScan");
    assert!(index_served(None), "a HEAD read uses the index");
    assert!(index_served(Some(HLC::new(100, 0))), "at the floor");
    assert!(index_served(Some(HLC::new(150, 0))), "above the floor");
    assert!(
        !index_served(Some(HLC::new(50, 0))),
        "below the floor the index has no history"
    );
}

/// The index matches a date-like literal through the writer's canonical date
/// text; a row-level comparison sees the stored property's own spelling. The
/// equality stays a residual so the index-served rows are exactly the scan's.
#[test]
fn date_literal_equality_stays_a_residual_filter() {
    let residual = |status: &str| match plan(None, status) {
        PhysicalPlan::CompoundIndexScan { filter, .. } => filter.is_some(),
        other => panic!("expected a CompoundIndexScan, got:\n{}", other.explain()),
    };
    assert!(
        !residual("open"),
        "a plain literal is guaranteed by the index"
    );
    assert!(
        residual("2024-01-01T00:00:00Z"),
        "a date-like literal must be re-checked row by row"
    );
}

/// Plan Phase 13e, the ownership gate: a WORKSPACE-owned index serves an
/// untyped query on its own workspace and nothing elsewhere; a NodeType-owned
/// one never serves an untyped query.
#[test]
fn ownership_gate_admits_a_workspace_index_for_its_own_workspace_only() {
    let authored = || {
        let column = |property: &str| CompoundIndexColumn {
            property: property.to_string(),
            column_type: CompoundColumnType::String,
            ascending: None,
        };
        CompoundIndexDefinition {
            name: "grp_status".to_string(),
            columns: vec![column("group"), column("status")],
            has_order_column: false,
            owner: None,
        }
    };
    let explain_with = |index: CompoundIndexDefinition| {
        let mut planner = planner(None);
        planner.set_compound_indexes(vec![index]);
        let filter = LogicalPlan::Filter {
            input: Box::new(scan_nodes("nodes")),
            predicate: FilterPredicate::from_expr(and(
                json_eq("nodes", "group", "/g1"),
                json_eq("nodes", "status", "open"),
            )),
        };
        planner.plan(&filter).unwrap().explain()
    };
    let own = explain_with(authored().owned_by_workspace("default"));
    assert!(
        own.contains("CompoundIndexScan: @grp_status [")
            && own.contains("(owner: workspace default)"),
        "its own workspace's untyped query uses it:\n{own}"
    );
    assert!(
        !explain_with(authored().owned_by_workspace("other")).contains("CompoundIndexScan"),
        "another workspace's index holds none of these nodes"
    );
    assert!(
        !explain_with(authored().owned_by_node_type("t:Doc")).contains("CompoundIndexScan"),
        "a type-owned index never serves an untyped query"
    );
}

/// Plan Phase 13e review: a WORKSPACE index pinned to a parent by
/// `__parent_path` AND another equality column (`nav (__parent_path,
/// status, __created_at)`) must not replace a CHILD_OF listing's editorial
/// order — no ORDER BY, or `ORDER BY __order` — with index order: a menu
/// filtered on `status` would silently reorder itself by `created_at`. It
/// still serves the ORDER BY it was built for.
#[test]
fn a_workspace_index_with_an_extra_equality_column_keeps_editorial_order() {
    use raisin_sql::analyzer::{DataType, Expr, TypedExpr};
    use raisin_sql::logical_plan::SortExpr;
    let column = |property: &str, column_type: CompoundColumnType| CompoundIndexColumn {
        property: property.to_string(),
        column_type,
        ascending: None,
    };
    let nav = CompoundIndexDefinition {
        name: "nav".to_string(),
        columns: vec![
            column("__parent_path", CompoundColumnType::String),
            column("status", CompoundColumnType::String),
            column("__created_at", CompoundColumnType::Timestamp),
        ],
        has_order_column: true,
        owner: None,
    }
    .owned_by_workspace("default");
    let explain = |order_by: Option<(&str, DataType)>| {
        let mut planner = planner(None);
        planner.set_compound_indexes(vec![nav.clone()]);
        let mut input = LogicalPlan::Filter {
            input: Box::new(scan_nodes("nodes")),
            predicate: FilterPredicate::from_expr(and(
                super::tests::child_of("/menu"),
                json_eq("nodes", "status", "published"),
            )),
        };
        if let Some((name, data_type)) = order_by {
            input = LogicalPlan::Sort {
                input: Box::new(input),
                sort_exprs: vec![SortExpr {
                    expr: TypedExpr::new(
                        Expr::Column {
                            table: "nodes".to_string(),
                            column: name.to_string(),
                        },
                        data_type,
                    ),
                    ascending: name != "created_at",
                    nulls_first: true,
                }],
            };
        }
        let limited = LogicalPlan::Limit {
            input: Box::new(input),
            limit: 10,
            offset: 0,
        };
        planner.plan(&limited).unwrap().explain()
    };
    for (what, order_by) in [
        ("no ORDER BY", None),
        ("ORDER BY __order", Some(("__order", DataType::Text))),
    ] {
        let plan = explain(order_by);
        assert!(
            !plan.contains("CompoundIndexScan"),
            "{what}: an editorial listing took the workspace index:\n{plan}"
        );
    }
    let plan = explain(Some(("created_at", DataType::TimestampTz)));
    assert!(
        plan.contains("CompoundIndexScan: @nav"),
        "the index still serves its own ORDER BY:\n{plan}"
    );
}
