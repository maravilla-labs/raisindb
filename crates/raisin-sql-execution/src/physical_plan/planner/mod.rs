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

use super::catalog::IndexCatalog;
use super::operators::{PhysicalPlan, ScanReason, VectorDistanceMetric};
use raisin_error::Error;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_sql::analyzer::{BinaryOperator, DataType, Expr, Literal, TypedExpr};
use raisin_sql::logical_plan::{
    AggregateFunction, LogicalPlan, ProjectionExpr, SortExpr, TableSchema,
};
use raisin_sql::optimizer::hierarchy_rewrite::{CanonicalPredicate, ComparisonOp};
use std::sync::Arc;

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
    /// True if this scan will feed into a COUNT(*) aggregate
    is_count_star: bool,
}

impl PlanContext {
    /// Create empty context with no parent operators
    fn empty() -> Self {
        Self {
            limit: None,
            order_by: None,
            is_count_star: false,
        }
    }

    /// Create context with a limit
    fn with_limit(limit: usize) -> Self {
        Self {
            limit: Some(limit),
            order_by: None,
            is_count_star: false,
        }
    }

    /// Add order by information to context
    fn with_order_by(mut self, column: String, is_asc: bool) -> Self {
        self.order_by = Some((column, is_asc));
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
}

impl PhysicalPlanner {
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
        }
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
