//! Physical Planner
//!
//! Converts logical plans to physical plans with intelligent scan selection.
//! The planner analyzes filter predicates to choose the best access method.
//!
//! # Module Organization
//!
//! - `compound_index` - Compound index matching and constant evaluation
//! - `property_order` - Property-ordered scan optimization
//! - `plan_dispatch` - Core planning dispatch (LogicalPlan -> PhysicalPlan)
//! - `scan_planning` - Scan method selection with filter analysis
//! - `filter_analysis` - Predicate canonicalization, extraction, and combination
//! - `vector_search` - Vector k-NN search pattern detection
//! - `join_planning` - Index lookup join optimization

mod compound_index;
mod filter_analysis;
mod join_planning;
mod plan_dispatch;
mod predicate_ops;
mod property_order;
mod scan_planning;
mod vector_search;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_spatial;

use super::catalog::IndexCatalog;
use super::operators::{OrderCursor, PhysicalPlan, ScanReason, VectorDistanceMetric};
use raisin_error::Error;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_sql::analyzer::{BinaryOperator, DataType, Expr, Literal, TypedExpr};
use raisin_sql::logical_plan::{
    AggregateFunction, LogicalPlan, ProjectionExpr, SortExpr, TableSchema,
};
use raisin_sql::optimizer::hierarchy_rewrite::{CanonicalPredicate, ComparisonOp};
use std::sync::Arc;

/// Pre-computed schema statistics for data-driven selectivity estimation.
///
/// When populated via [`PhysicalPlanner::set_schema_statistics`], the planner
/// uses these counts to derive per-column selectivity instead of falling back
/// to hard-coded defaults (e.g. `1 / node_type_count` instead of `0.05`).
#[derive(Debug, Clone, Default)]
pub struct SchemaStats {
    /// Number of distinct NodeType definitions on this branch.
    pub node_type_count: usize,
    /// Number of distinct Archetype definitions on this branch.
    pub archetype_count: usize,
}

/// Buffer size for scan limit pushdown.
/// We use a large value to ensure post-scan filtering doesn't cause
/// fewer results than expected from the LIMIT clause.
const SCAN_LIMIT_BUFFER: usize = 200_000;

/// Hint about filter selectivity for scan selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterSelectivityHint {
    /// Filter is highly selective (node_type = 'X' or multiple property filters)
    /// Use filter-first strategy: PropertyIndexScan + TopN
    HighlySelective,
    /// Filter has unknown selectivity, use default PropertyOrderScan
    Unknown,
}

/// Context information propagated down during planning to inform scan selection
#[derive(Debug, Clone)]
struct PlanContext {
    /// Limit value if there's a LIMIT operator above
    limit: Option<usize>,
    /// Sort column and direction if there's an ORDER BY above
    /// Format: (column_name, is_ascending)
    order_by: Option<(String, bool)>,
    /// The first ORDER BY expression verbatim, when there is an ORDER BY above.
    ///
    /// `order_by` only survives when the sort key reduces to a column NAME, which
    /// `ORDER BY ST_DISTANCE(loc, ST_POINT(..))` does not. Without the expression
    /// a scan cannot tell "no ORDER BY" (safe to bound in its own order) from "an
    /// ORDER BY it happens to satisfy" or "an ORDER BY it does not" — and getting
    /// that wrong is how a LIMIT truncates in the wrong order.
    order_by_expr: Option<(TypedExpr, bool)>,
    /// True if this scan will feed into a COUNT(*) aggregate
    is_count_star: bool,
    /// True when the query is scoped to an EXPLICIT historical revision
    /// (`WHERE __revision = N`), which the analyzer strips into
    /// `ExecutionContext::max_revision`.
    ///
    /// Read by spatial planning: the spatial index is pruned by a compaction
    /// filter that keeps the newest entry per node per cell plus a bounded
    /// recent window, so a read at HEAD is exact but a read at an OLD revision
    /// may resolve against entries that have been pruned away. A historical
    /// spatial predicate is therefore routed to the row-scan fallback, which is
    /// always exact. HEAD queries (the overwhelming majority, and the ones the
    /// index performance work is for) are unaffected — this flag is false for
    /// them.
    historical_revision: bool,
}

impl PlanContext {
    /// Create empty context with no parent operators
    fn empty() -> Self {
        Self {
            limit: None,
            order_by: None,
            order_by_expr: None,
            is_count_star: false,
            historical_revision: false,
        }
    }

    /// Create context with a limit
    fn with_limit(limit: usize) -> Self {
        Self {
            limit: Some(limit),
            order_by: None,
            order_by_expr: None,
            is_count_star: false,
            historical_revision: false,
        }
    }

    /// Add order by information to context
    fn with_order_by(mut self, column: String, is_asc: bool) -> Self {
        self.order_by = Some((column, is_asc));
        self
    }

    /// Record the verbatim first ORDER BY expression and its direction.
    fn with_order_by_expr(mut self, expr: TypedExpr, is_asc: bool) -> Self {
        self.order_by_expr = Some((expr, is_asc));
        self
    }

    /// True when there is no ORDER BY above this scan at all, so the scan's own
    /// iteration order is free to bound the result.
    fn has_no_ordering(&self) -> bool {
        self.order_by.is_none() && self.order_by_expr.is_none()
    }

    /// Record that this scan reads at an explicit historical revision.
    fn with_historical_revision(mut self, historical: bool) -> Self {
        self.historical_revision = historical;
        self
    }

    /// Mark this context as feeding a COUNT(*)
    fn with_count_star(mut self) -> Self {
        self.is_count_star = true;
        self
    }
}

/// Physical planner that converts logical plans to physical plans
pub struct PhysicalPlanner {
    /// Default tenant ID (can be overridden per query) - Arc for cheap cloning
    default_tenant_id: Arc<str>,
    /// Default repository ID - Arc for cheap cloning
    default_repo_id: Arc<str>,
    /// Default branch - Arc for cheap cloning
    default_branch: Arc<str>,
    /// Default workspace - Arc for cheap cloning
    default_workspace: Arc<str>,
    /// Index catalog for scan selection
    index_catalog: Arc<dyn IndexCatalog>,
    /// Available compound indexes for the current query context.
    /// Populated from NodeType schemas when set_compound_indexes is called.
    compound_indexes: Vec<CompoundIndexDefinition>,
    /// Optional schema statistics for data-driven selectivity estimation.
    /// When set, equality predicates on `node_type` and `archetype` columns
    /// use `1 / count` instead of the default 0.05 heuristic.
    schema_stats: Option<SchemaStats>,
}

impl PhysicalPlanner {
    /// Full-pipeline EXPLAIN: everything [`raisin_sql::QueryPlan::explain`]
    /// renders (SQL, analyzed statement, logical plan, optimized logical plan),
    /// plus the PHYSICAL plan this planner produces for it.
    ///
    /// `QueryPlan` lives in `raisin-sql`, which cannot see physical plans at
    /// all, so its own `explain()` structurally stops at the logical level.
    /// Every decision that actually determines how a query runs — the join
    /// algorithm, the scan chosen, a pushed-down limit — is invisible there,
    /// which is how a wrong scan limit stayed hidden behind a plausible-looking
    /// EXPLAIN. Use this when diagnosing a plan.
    ///
    /// (The `EXPLAIN <stmt>` SQL statement already renders the physical plan;
    /// this is the equivalent for callers holding a `QueryPlan` directly.)
    pub fn explain_query_plan(&self, plan: &raisin_sql::QueryPlan) -> Result<String, Error> {
        let mut output = plan.explain();
        let physical = self.plan(&plan.optimized)?;
        output.push_str("\n=== Physical Execution Plan ===\n");
        output.push_str(&physical.explain());
        Ok(output)
    }

    /// Create a new physical planner with default context and RocksDB catalog
    pub fn new() -> Self {
        use super::catalog::RocksDBIndexCatalog;
        Self {
            default_tenant_id: Arc::from("default"),
            default_repo_id: Arc::from("default"),
            default_branch: Arc::from("main"),
            default_workspace: Arc::from("default"),
            index_catalog: Arc::new(RocksDBIndexCatalog::new()),
            compound_indexes: Vec::new(),
            schema_stats: None,
        }
    }

    /// Create a planner with specific context and default RocksDB catalog
    pub fn with_context(
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace: String,
    ) -> Self {
        use super::catalog::RocksDBIndexCatalog;
        Self {
            default_tenant_id: Arc::from(tenant_id),
            default_repo_id: Arc::from(repo_id),
            default_branch: Arc::from(branch),
            default_workspace: Arc::from(workspace),
            index_catalog: Arc::new(RocksDBIndexCatalog::new()),
            compound_indexes: Vec::new(),
            schema_stats: None,
        }
    }

    /// Create a planner with specific context and custom index catalog
    pub fn with_catalog(
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace: String,
        catalog: Arc<dyn IndexCatalog>,
    ) -> Self {
        Self {
            default_tenant_id: Arc::from(tenant_id),
            default_repo_id: Arc::from(repo_id),
            default_branch: Arc::from(branch),
            default_workspace: Arc::from(workspace),
            index_catalog: catalog,
            compound_indexes: Vec::new(),
            schema_stats: None,
        }
    }

    /// Spatial index availability for one (workspace, branch, property).
    ///
    /// The single place the planner asks that question, so the scope is assembled
    /// consistently: tenant and repo come from the planner's own context, while
    /// branch and workspace come from the query — a spatial index is per branch,
    /// and a query against a feature branch must not be planned against main's
    /// index state.
    ///
    /// # Wildcard paths are never index-backed, and that guard lives HERE
    ///
    /// `properties->>'stops[].geo'` names every element of an array. Each element
    /// has its OWN index-key namespace (`stops.0.geo`, `stops.1.geo`, …), so a
    /// single cell-ring scan over `stops[].geo` would read a prefix that holds
    /// nothing and return **zero rows** — the exact silent-empty failure this
    /// subsystem has spent a pass eliminating.
    ///
    /// The state record, on the other hand, IS keyed by `stops[].geo` (array
    /// indices normalise, see `policy_key_for_path`), so availability would
    /// legitimately answer `Ready`. Answering `Unusable` here instead is what
    /// routes a wildcard onto `build_spatial_fallback_scan`: correct rows, a full
    /// scan, and a warning naming the path. Placing the guard at the planner's one
    /// availability call site means every caller — `collect_index_options`,
    /// `build_spatial_scan`, the fallback's own EXPLAIN detail — inherits it and
    /// none can disagree.
    pub(super) fn spatial_availability(
        &self,
        workspace: &str,
        branch: &str,
        property: &str,
    ) -> super::catalog::SpatialAvailability {
        if raisin_models::nodes::properties::is_wildcard_property_path(property) {
            return super::catalog::SpatialAvailability::Unusable(format!(
                "'{}' is a wildcard over an array of geometries; each element is indexed under \
                 its own concrete path, so no single index scan can answer it. The predicate is \
                 applied per row instead (correct, but a full scan). Name one element \
                 (properties->>'{}') to use the index.",
                property,
                property.replacen("[]", ".0", 1)
            ));
        }
        self.index_catalog.spatial_index_availability(
            &self.default_tenant_id,
            &self.default_repo_id,
            branch,
            workspace,
            property,
        )
    }

    /// Convert a logical plan to a physical plan (public entry point)
    pub fn plan(&self, logical: &LogicalPlan) -> Result<PhysicalPlan, Error> {
        // Start with empty context - no parent operators
        self.plan_with_context(logical, &PlanContext::empty())
    }
}

impl Default for PhysicalPlanner {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a literal to JSON value
pub(super) fn literal_to_json(lit: &Literal) -> Result<serde_json::Value, Error> {
    match lit {
        Literal::Text(s) => Ok(serde_json::Value::String(s.clone())),
        Literal::Int(i) => Ok(serde_json::Value::Number((*i).into())),
        Literal::BigInt(i) => Ok(serde_json::Value::Number((*i).into())),
        Literal::Double(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| Error::Validation("Invalid float for JSON".to_string())),
        Literal::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Literal::JsonB(j) => Ok(j.clone()),
        Literal::Null => Ok(serde_json::Value::Null),
        _ => Err(Error::Validation(
            "Cannot convert literal to JSON".to_string(),
        )),
    }
}

/// Components extracted from a LogicalPlan tree when checking for
/// property-ordered scan optimization
struct PropertyOrderComponents {
    project_exprs: Vec<ProjectionExpr>,
    filter_expr: Option<TypedExpr>,
    scan_info: ScanNodeInfo,
}

/// Information about a scan node extracted from the logical plan
struct ScanNodeInfo {
    table: String,
    alias: Option<String>,
    schema: Arc<TableSchema>,
    workspace: Option<String>,
    branch_override: Option<String>,
    projection: Option<Vec<String>>,
}
