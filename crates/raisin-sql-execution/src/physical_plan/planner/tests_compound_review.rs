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
        owner_node_type: None,
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
