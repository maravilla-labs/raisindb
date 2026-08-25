//! Physical plan enum definition.
//!
//! Defines the `PhysicalPlan` enum representing concrete execution strategies.
//! Each variant maps to a specific operator that knows how to produce a stream of rows.
//!
//! The enum variants are organized into category files for maintainability:
//! - [`scan`] - basic scan operators (table, count, prefix, table function)
//! - [`index_scan`] - index-driven scans (property, path, spatial, vector, etc.)
//! - [`relational`] - relational algebra (filter, project, sort, aggregate, etc.)
//! - [`join`] - join algorithms (nested loop, hash, semi, index lookup)
//! - [`dml`] - data manipulation (insert, update, delete, move, copy, etc.)

pub mod dml;
pub mod index_scan;
pub mod join;
pub mod relational;
pub mod scan;

use raisin_models::nodes::properties::schema::CompoundColumnType;
use raisin_sql::analyzer::TypedExpr;
use raisin_sql::logical_plan::{ProjectionExpr, SortExpr, TableSchema, WindowExpr};
use std::sync::Arc;

use super::scan_types::{IndexLookupParams, ScanReason, VectorDistanceMetric};

/// An exclusive editorial-order keyset cursor pushed down into a scan.
///
/// The two forms are mutually exclusive because they address different orders:
/// `__order` orders siblings under one parent, while `__tree_order` orders a
/// whole subtree in document order. A scan is driven by one or the other, never
/// both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderCursor {
    /// Resume strictly after this sibling order label (`__order > 'x'`),
    /// seeking within one parent's children.
    Label(String),
    /// Resume strictly after this subtree tree-order value
    /// (`__tree_order > 'x'`), continuing a depth-first walk without re-walking
    /// what came before.
    TreeOrder(String),
}

impl OrderCursor {
    /// The sibling label form, if that is what this cursor is.
    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Label(label) => Some(label),
            Self::TreeOrder(_) => None,
        }
    }

    /// The subtree tree-order form, if that is what this cursor is.
    pub fn tree_order(&self) -> Option<&str> {
        match self {
            Self::TreeOrder(value) => Some(value),
            Self::Label(_) => None,
        }
    }
}

/// Generates the PhysicalPlan enum by composing variant groups from submodules.
///
/// Each `variants!` block corresponds to a category file that contains
/// detailed documentation for the variants in that group.
macro_rules! define_physical_plan {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($variant:tt)*
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $($variant)*
        }
    };
}

define_physical_plan! {
    /// Physical plan node representing a concrete execution strategy.
    ///
    /// Physical plans are created by the physical planner from logical plans.
    /// They include specific scan methods (prefix scan, property index scan, etc.)
    /// and represent the actual execution strategy.
    #[derive(Debug, Clone)]
    pub enum PhysicalPlan {
        // ── Basic Scans ──────────────────────────────────────────

        /// Full table scan with optional filter pushdown
        TableScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            schema: Arc<TableSchema>,
            filter: Option<TypedExpr>,
            projection: Option<Vec<String>>,
            limit: Option<usize>,
            reason: ScanReason,
        },
        /// Optimized count scan for COUNT(*) queries
        CountScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            max_revision: Option<raisin_hlc::HLC>,
        },
        /// Table-valued function invocation
        TableFunction {
            name: String,
            alias: Option<String>,
            args: Vec<TypedExpr>,
            schema: Arc<TableSchema>,
            workspace: Option<String>,
            branch_override: Option<String>,
            max_revision: Option<raisin_hlc::HLC>,
        },
        /// Prefix scan on path hierarchy
        PrefixScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            path_prefix: String,
            projection: Option<Vec<String>>,
            direct_children_only: bool,
            limit: Option<usize>,
            /// Exclusive editorial-order keyset cursor pushed into the index
            /// seek, from a `__order > 'x'` or `__tree_order > 'x'` predicate.
            order_cursor: Option<OrderCursor>,
            /// Walk editorial order backwards (`ORDER BY __order DESC`).
            /// `direct_children_only` only — a subtree walk is forward-only.
            order_descending: bool,
            /// True when this scan's own iteration order already satisfies the
            /// query's `ORDER BY`, so the `Sort` above it was elided and the
            /// `limit` may be applied during the scan.
            claims_editorial_order: bool,
        },

        // ── Index Scans ──────────────────────────────────────────

        /// Property index scan for exact value match
        PropertyIndexScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            property_name: String,
            property_value: String,
            projection: Option<Vec<String>>,
            limit: Option<usize>,
        },
        /// Property index count scan (COUNT with property filter)
        ///
        /// `properties` holds one or more (name, value) pairs whose per-value
        /// index counts are SUMMED — used for `COUNT(*)` over a single property
        /// equality or over an `IN`/OR Union of disjoint property equalities.
        PropertyIndexCountScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            properties: Vec<(String, String)>,
        },
        /// Ordered property scan with sort direction
        PropertyOrderScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            schema: Arc<TableSchema>,
            projection: Option<Vec<String>>,
            filter: Option<TypedExpr>,
            property_name: String,
            ascending: bool,
            limit: usize,
        },
        /// Compound index scan for multi-column queries
        CompoundIndexScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            index_name: String,
            /// Matched equality columns as (property, value, declared type).
            /// The type is carried so the executor encodes the scan prefix the
            /// same way the index writer encoded the key.
            equality_columns: Vec<(String, String, CompoundColumnType)>,
            pre_sorted: bool,
            ascending: bool,
            projection: Option<Vec<String>>,
            filter: Option<TypedExpr>,
            limit: Option<usize>,
        },
        /// Property range scan with bounded range
        PropertyRangeScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            schema: Arc<TableSchema>,
            projection: Option<Vec<String>>,
            filter: Option<TypedExpr>,
            property_name: String,
            lower_bound: Option<(String, bool)>,
            upper_bound: Option<(String, bool)>,
            ascending: bool,
            limit: Option<usize>,
        },
        /// Path index scan for exact path lookups (O(1))
        PathIndexScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            path: String,
            projection: Option<Vec<String>>,
        },
        /// Node ID scan for direct node lookups (O(1))
        NodeIdScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            node_id: String,
            projection: Option<Vec<String>>,
        },
        /// Full-text search scan using Tantivy
        FullTextScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            language: String,
            query: String,
            limit: usize,
            projection: Option<Vec<String>>,
        },
        /// Graph neighbors scan using RELATION_INDEX
        NeighborsScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            alias: Option<String>,
            source_workspace: String,
            source_node_id: String,
            direction: String,
            relation_type: Option<String>,
            projection: Option<Vec<String>>,
            limit: Option<usize>,
        },
        /// Spatial proximity scan using geohash index.
        ///
        /// Emits a `__distance` column (metres, geodesic) and yields rows in
        /// ascending distance order.
        SpatialDistanceScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            property_name: String,
            center_lon: f64,
            center_lat: f64,
            radius_meters: f64,
            projection: Option<Vec<String>>,
            limit: Option<usize>,
            /// True when this scan's ascending-distance iteration order already
            /// satisfies the query's `ORDER BY`, so the `Sort` above it was
            /// elided and `limit` may be applied during the scan.
            ///
            /// READ by `sort_is_satisfied_by_scan` in the planner — a flag that
            /// is set and never consulted is worse than no flag (see the
            /// `CompoundIndexScan.pre_sorted` note in CLAUDE.md).
            claims_distance_order: bool,
            /// The geohash precisions the index was **actually built at**, as
            /// observed at PLAN time.
            ///
            /// Carried in the plan rather than re-read during execution so the
            /// coverage decision the planner made (and possibly stripped a
            /// predicate on) is the decision the scan executes. Re-reading could
            /// pick up a mid-flight reindex and answer a query the planner had
            /// proven coverable with a precision set that no longer covers it.
            precisions: Vec<usize>,
            /// `(property, value)` discriminator pre-filter, e.g.
            /// `("floor", "L2")`.
            ///
            /// A pure *selectivity* device: it rejects only candidates whose
            /// stored bucket provably differs, and index entries carrying no
            /// bucket are never rejected. The predicate it came from ALWAYS stays
            /// in the residual filter, so a stale or absent bucket can cost a
            /// missed optimisation but never a missed row.
            bucket_eq: Option<(String, String)>,
            /// The plan to run INSTEAD when the index refuses to answer at
            /// execution time.
            ///
            /// One thing can only be discovered while scanning: a cell prefix
            /// that exceeds the per-cell entry budget
            /// (`Error::SpatialBudgetExceeded`). The index then cannot answer
            /// without answering SHORT, and a short spatial answer is the one
            /// failure this subsystem refuses. Carrying the fallback plan here is
            /// what lets the executor degrade to a correct row scan rather than
            /// fail the query — it cannot re-plan on its own, because the
            /// predicate may have been stripped from the residual filter.
            ///
            /// `None` only where a fallback could not be built.
            fallback: Option<Box<PhysicalPlan>>,
        },
        /// Spatial k-nearest neighbors scan.
        ///
        /// Built from `ORDER BY ST_DISTANCE(geom, <const>) ASC LIMIT k`, which is
        /// the only shape with a bounded access path — a DESC or unbounded
        /// distance sort has to read everything.
        SpatialKnnScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            property_name: String,
            center_lon: f64,
            center_lat: f64,
            k: usize,
            projection: Option<Vec<String>>,
            /// Always true in practice (a k-NN scan exists because a distance
            /// ordering was requested), but carried explicitly so the sort
            /// elision reads a promise rather than re-deriving one.
            claims_distance_order: bool,
            /// Geohash precisions the index was built at; see
            /// `SpatialDistanceScan::precisions`.
            precisions: Vec<usize>,
        },
        /// Reference index scan using reverse reference index
        ReferenceIndexScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            target_workspace: String,
            target_path: String,
            projection: Option<Vec<String>>,
            limit: Option<usize>,
        },
        /// Vector similarity search using HNSW index
        VectorScan {
            tenant_id: String,
            repo_id: String,
            branch: String,
            workspace: String,
            table: String,
            alias: Option<String>,
            query_vector: TypedExpr,
            distance_metric: VectorDistanceMetric,
            vector_column: String,
            k: usize,
            max_distance: Option<f32>,
            projection: Option<Vec<String>>,
            distance_alias: Option<String>,
        },
        /// Materialise the `__distance` / `__matched_path` pseudo-columns on rows
        /// a spatial FALLBACK scan produced.
        ///
        /// A `SpatialDistanceScan` reads its distance straight off the index
        /// entry, so it injects both columns itself. The fallback path has no
        /// index entry to read: the predicate is evaluated per row, and the
        /// distance it was decided on is discarded. This operator recomputes it —
        /// the MINIMUM over the geometries the (possibly wildcard) property path
        /// addresses, plus the CONCRETE path of the geometry that achieved it.
        ///
        /// Planned only when the query actually asks for one of the two columns,
        /// so an ordinary spatial fallback pays nothing.
        SpatialAnnotate {
            input: Box<PhysicalPlan>,
            /// The property path as written in the predicate — may be a wildcard
            /// (`stops[].geo`), which is exactly the case this exists for.
            property_name: String,
            center_lon: f64,
            center_lat: f64,
        },
        /// Scan materialized CTE results
        CTEScan {
            cte_name: String,
            schema: Arc<TableSchema>,
        },

        // ── Relational Operators ─────────────────────────────────

        /// Filter rows based on predicates (AND-ed, CNF form)
        Filter {
            input: Box<PhysicalPlan>,
            predicates: Vec<TypedExpr>,
        },
        /// Project (select) specific expressions
        Project {
            input: Box<PhysicalPlan>,
            exprs: Vec<ProjectionExpr>,
        },
        /// Sort rows by one or more expressions (blocking operator)
        Sort {
            input: Box<PhysicalPlan>,
            sort_exprs: Vec<SortExpr>,
        },
        /// TopN - optimized sort with limit using heap
        TopN {
            input: Box<PhysicalPlan>,
            sort_exprs: Vec<SortExpr>,
            limit: usize,
        },
        /// Limit the number of rows and apply offset
        Limit {
            input: Box<PhysicalPlan>,
            limit: usize,
            offset: usize,
        },
        /// Hash-based aggregation with grouping
        HashAggregate {
            input: Box<PhysicalPlan>,
            group_by: Vec<TypedExpr>,
            aggregates: Vec<raisin_sql::logical_plan::AggregateExpr>,
        },
        /// Execute query with CTEs (Common Table Expressions)
        WithCTE {
            ctes: Vec<(String, Box<PhysicalPlan>)>,
            main_query: Box<PhysicalPlan>,
        },
        /// Window function operator
        Window {
            input: Box<PhysicalPlan>,
            window_exprs: Vec<WindowExpr>,
        },
        /// Remove duplicate rows using hash-based deduplication
        Distinct {
            input: Box<PhysicalPlan>,
            on_columns: Vec<String>,
        },
        /// Concatenate the row streams of several child plans in order.
        ///
        /// Used to expand `col IN (v1, v2, ...)` into a union of per-value
        /// indexed equality scans. Because each distinct literal over a single
        /// column selects a disjoint row set (a node has exactly one path, id,
        /// node_type, or value at a given JSON key), no de-duplication is needed.
        Union {
            inputs: Vec<PhysicalPlan>,
        },

        // ── Join Operators ───────────────────────────────────────

        /// Nested loop join - O(n*m) complexity
        NestedLoopJoin {
            left: Box<PhysicalPlan>,
            right: Box<PhysicalPlan>,
            join_type: raisin_sql::analyzer::JoinType,
            condition: Option<TypedExpr>,
        },
        /// Hash join - O(n+m) for equality conditions
        HashJoin {
            left: Box<PhysicalPlan>,
            right: Box<PhysicalPlan>,
            join_type: raisin_sql::analyzer::JoinType,
            left_keys: Vec<TypedExpr>,
            right_keys: Vec<TypedExpr>,
        },
        /// Hash-based semi-join for IN subquery support
        HashSemiJoin {
            left: Box<PhysicalPlan>,
            right: Box<PhysicalPlan>,
            left_key: TypedExpr,
            right_key: TypedExpr,
            anti: bool,
        },
        /// Index lookup join (nested loop with O(1) index lookup)
        IndexLookupJoin {
            outer: Box<PhysicalPlan>,
            join_type: raisin_sql::analyzer::JoinType,
            outer_key_column: String,
            inner_lookup: IndexLookupParams,
        },

        // ── DML Operations ──────────────────────────────────────

        /// Physical INSERT operation
        PhysicalInsert {
            target: raisin_sql::analyzer::DmlTableTarget,
            schema: raisin_sql::analyzer::catalog::TableDef,
            columns: Vec<String>,
            values: Vec<Vec<TypedExpr>>,
            is_upsert: bool,
        },
        /// Physical UPDATE operation
        PhysicalUpdate {
            target: raisin_sql::analyzer::DmlTableTarget,
            schema: raisin_sql::analyzer::catalog::TableDef,
            assignments: Vec<(String, TypedExpr)>,
            filter: Option<TypedExpr>,
            branch_override: Option<String>,
        },
        /// Physical DELETE operation
        PhysicalDelete {
            target: raisin_sql::analyzer::DmlTableTarget,
            schema: raisin_sql::analyzer::catalog::TableDef,
            filter: Option<TypedExpr>,
            branch_override: Option<String>,
        },
        /// Physical ORDER operation
        PhysicalOrder {
            source: raisin_sql::ast::order::NodeReference,
            target: raisin_sql::ast::order::NodeReference,
            position: raisin_sql::ast::order::OrderPosition,
            workspace: Option<String>,
            branch_override: Option<String>,
        },
        /// Physical MOVE operation
        PhysicalMove {
            source: raisin_sql::ast::order::NodeReference,
            target_parent: raisin_sql::ast::order::NodeReference,
            workspace: Option<String>,
            branch_override: Option<String>,
        },
        /// Physical COPY operation
        PhysicalCopy {
            source: raisin_sql::ast::order::NodeReference,
            target_parent: raisin_sql::ast::order::NodeReference,
            new_name: Option<String>,
            recursive: bool,
            workspace: Option<String>,
            branch_override: Option<String>,
        },
        /// Physical TRANSLATE operation
        PhysicalTranslate {
            locale: String,
            node_translations:
                std::collections::HashMap<String, raisin_sql::analyzer::AnalyzedTranslationValue>,
            filter: Option<raisin_sql::analyzer::AnalyzedTranslateFilter>,
            workspace: Option<String>,
            branch_override: Option<String>,
        },
        /// Physical RELATE operation
        PhysicalRelate {
            source: raisin_sql::analyzer::AnalyzedRelateEndpoint,
            target: raisin_sql::analyzer::AnalyzedRelateEndpoint,
            relation_type: String,
            weight: Option<f64>,
            branch_override: Option<String>,
        },
        /// Physical UNRELATE operation
        PhysicalUnrelate {
            source: raisin_sql::analyzer::AnalyzedRelateEndpoint,
            target: raisin_sql::analyzer::AnalyzedRelateEndpoint,
            relation_type: Option<String>,
            branch_override: Option<String>,
        },
        /// Physical RESTORE operation
        PhysicalRestore {
            node: raisin_sql::ast::order::NodeReference,
            revision: raisin_sql::ast::branch::RevisionRef,
            recursive: bool,
            translations: Option<Vec<String>>,
            branch_override: Option<String>,
        },

        // ── Lateral ──────────────────────────────────────────────

        /// Per-row function evaluation that adds a computed column.
        /// Used for `LATERAL function() AS alias` in FROM clause.
        LateralMap {
            input: Box<PhysicalPlan>,
            /// The function call expression to evaluate per row
            function_expr: TypedExpr,
            /// Output column name (from the LATERAL alias)
            column_name: String,
        },

        // ── Utility ──────────────────────────────────────────────

        /// Empty plan for DDL statements that bypass physical execution
        Empty,
    }
}
