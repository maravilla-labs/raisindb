//! LogicalPlan enum definition
//!
//! Defines the logical operator tree representing relational algebra operations.

use crate::analyzer::{catalog::TableDef, DmlTableTarget, TypedExpr};
use std::sync::Arc;

use super::supporting_types::{
    AggregateExpr, DistinctSpec, FilterPredicate, ProjectionExpr, SortExpr, TableSchema, WindowExpr,
};

/// Logical plan node representing a relational operator
#[derive(Debug, Clone)]
pub enum LogicalPlan {
    /// Scan a table
    Scan {
        table: String,
        /// Optional table alias (qualifier used for column resolution)
        alias: Option<String>,
        schema: Arc<TableSchema>,
        /// Optional workspace name (for workspace-scoped tables)
        workspace: Option<String>,
        /// Maximum revision to read (for point-in-time queries)
        /// None = HEAD (latest), Some(rev) = specific revision
        max_revision: Option<raisin_hlc::HLC>,
        /// Optional branch override (for cross-branch queries)
        /// None = use default branch, Some(name) = query specific branch
        branch_override: Option<String>,
        /// Locales for translation resolution (extracted from WHERE locale = 'X' or WHERE locale IN (...))
        /// Empty vec = no locale filtering, use default behavior
        /// Non-empty vec = use these locales for translation resolution, return one row per locale per node
        locales: Vec<String>,
        /// Optional pushed-down filter (for optimization)
        filter: Option<TypedExpr>,
        /// Optional column projection (for optimization)
        /// If None, all columns are read. If Some, only specified columns are read.
        projection: Option<Vec<String>>,
    },

    /// Invoke a table-valued function
    TableFunction {
        name: String,
        alias: Option<String>,
        args: Vec<TypedExpr>,
        schema: Arc<TableSchema>,
        workspace: Option<String>,
        branch_override: Option<String>,
        max_revision: Option<raisin_hlc::HLC>,
        /// Locales for translation resolution
        locales: Vec<String>,
    },

    /// Filter rows based on predicate
    Filter {
        input: Box<LogicalPlan>,
        predicate: FilterPredicate,
    },

    /// Project (select) specific expressions
    Project {
        input: Box<LogicalPlan>,
        exprs: Vec<ProjectionExpr>,
    },

    /// Sort rows
    Sort {
        input: Box<LogicalPlan>,
        sort_exprs: Vec<SortExpr>,
    },

    /// Limit number of rows
    Limit {
        input: Box<LogicalPlan>,
        limit: usize,
        offset: usize,
    },

    /// Remove duplicate rows
    ///
    /// For basic DISTINCT: deduplicates based on all projected columns
    /// For DISTINCT ON: deduplicates based on specified expressions, keeps first row per group
    Distinct {
        input: Box<LogicalPlan>,
        distinct_spec: DistinctSpec,
    },

    /// Aggregate with optional grouping
    Aggregate {
        input: Box<LogicalPlan>,
        group_by: Vec<TypedExpr>,
        aggregates: Vec<AggregateExpr>,
    },

    /// Join two tables
    Join {
        left: Box<LogicalPlan>,
        right: Box<LogicalPlan>,
        join_type: crate::analyzer::JoinType,
        condition: Option<TypedExpr>,
    },

    /// Semi-join for IN subquery and EXISTS support
    ///
    /// Returns rows from left where at least one match exists in right.
    /// Eliminates duplicate matches naturally (IN semantics).
    SemiJoin {
        /// Left input (the main query being filtered)
        left: Box<LogicalPlan>,
        /// Right input (the subquery results to check against)
        right: Box<LogicalPlan>,
        /// Expression from the left side to check for membership
        left_key: TypedExpr,
        /// Expression from the right side (subquery column) to match against
        right_key: TypedExpr,
        /// When true, this is an anti-join (NOT IN semantics)
        /// Returns rows where NO match exists
        anti: bool,
    },

    /// CTE (Common Table Expression) wrapper
    /// Executes CTEs first, then executes main query with CTE results available
    WithCTE {
        /// CTE definitions (name, query)
        ctes: Vec<(String, Box<LogicalPlan>)>,
        /// Main query that can reference CTEs
        main_query: Box<LogicalPlan>,
    },

    /// Reference to a CTE defined in outer WITH clause
    CTEScan {
        /// Name of the CTE to scan
        cte_name: String,
        /// Schema of the CTE (inferred from its SELECT list)
        schema: Arc<TableSchema>,
        /// Optional alias for the CTE reference
        alias: Option<String>,
    },

    /// Subquery (derived table) in FROM clause
    /// Similar to CTE but inline - the subquery is materialized and then scanned
    Subquery {
        /// The subquery logical plan to execute
        input: Box<LogicalPlan>,
        /// Alias for the subquery (required)
        alias: String,
        /// Schema of the subquery (inferred from its SELECT list)
        schema: Arc<TableSchema>,
    },

    /// Window function operation
    /// Computes window functions over partitions with optional ordering and framing
    Window {
        input: Box<LogicalPlan>,
        /// Window expressions to compute
        window_exprs: Vec<WindowExpr>,
    },

    /// Insert rows into a table
    ///
    /// Also used for UPSERT operations when `is_upsert` is true.
    Insert {
        /// Target table for insertion
        target: DmlTableTarget,
        /// Table schema for validation
        schema: TableDef,
        /// Column names being inserted (empty means all columns in schema order)
        columns: Vec<String>,
        /// Values to insert - each Vec represents a row
        values: Vec<Vec<TypedExpr>>,
        /// Whether this is an UPSERT (create-or-update) vs INSERT (create-only)
        is_upsert: bool,
    },

    /// Update rows in a table
    Update {
        /// Target table for update
        target: DmlTableTarget,
        /// Table schema for validation
        schema: TableDef,
        /// SET clause assignments: (column_name, new_value_expression)
        assignments: Vec<(String, TypedExpr)>,
        /// WHERE clause filter (None means update all rows)
        filter: Option<TypedExpr>,
        /// Optional branch override (for cross-branch operations)
        branch_override: Option<String>,
    },

    /// Delete rows from a table
    Delete {
        /// Target table for deletion
        target: DmlTableTarget,
        /// Table schema for validation
        schema: TableDef,
        /// WHERE clause filter (None means delete all rows)
        filter: Option<TypedExpr>,
        /// Optional branch override (for cross-branch operations)
        branch_override: Option<String>,
    },

    /// Reorder a node relative to a sibling
    Order {
        /// The source node being moved
        source: crate::ast::order::NodeReference,
        /// The target node to position relative to
        target: crate::ast::order::NodeReference,
        /// Position relative to target (Above = before, Below = after)
        position: crate::ast::order::OrderPosition,
        /// Workspace containing the nodes
        workspace: Option<String>,
        /// Optional branch override
        branch_override: Option<String>,
    },

    /// Move node to new parent
    Move {
        /// The source node being moved
        source: crate::ast::order::NodeReference,
        /// The target parent node (where to move the node to)
        target_parent: crate::ast::order::NodeReference,
        /// Workspace containing the nodes
        workspace: Option<String>,
        /// Optional branch override
        branch_override: Option<String>,
    },

    /// Copy node to new parent
    Copy {
        /// The source node being copied
        source: crate::ast::order::NodeReference,
        /// The target parent node (where to copy the node to)
        target_parent: crate::ast::order::NodeReference,
        /// Optional new name for the copied node
        new_name: Option<String>,
        /// Whether to copy recursively (COPY TREE) or just the single node (COPY)
        recursive: bool,
        /// Workspace containing the nodes
        workspace: Option<String>,
        /// Optional branch override
        branch_override: Option<String>,
    },

    /// Translate node content to a specific locale
    Translate {
        /// Target locale code (e.g., "de", "fr", "en-US")
        locale: String,
        /// Node-level translations: JsonPointer -> value
        node_translations:
            std::collections::HashMap<String, crate::analyzer::AnalyzedTranslationValue>,
        /// Block-level translations: block_uuid -> (JsonPointer -> value)
        block_translations: std::collections::HashMap<
            String,
            std::collections::HashMap<String, crate::analyzer::AnalyzedTranslationValue>,
        >,
        /// Filter to select nodes to translate
        filter: Option<crate::analyzer::AnalyzedTranslateFilter>,
        /// Workspace containing the nodes
        workspace: Option<String>,
        /// Optional branch override
        branch_override: Option<String>,
    },

    /// Create a relationship between two nodes
    Relate {
        /// Source node endpoint
        source: crate::analyzer::AnalyzedRelateEndpoint,
        /// Target node endpoint
        target: crate::analyzer::AnalyzedRelateEndpoint,
        /// Relationship type (e.g., "references", "tagged_with")
        relation_type: String,
        /// Optional weight for graph algorithms
        weight: Option<f64>,
        /// Optional branch override
        branch_override: Option<String>,
    },

    /// Remove a relationship between two nodes
    Unrelate {
        /// Source node endpoint
        source: crate::analyzer::AnalyzedRelateEndpoint,
        /// Target node endpoint
        target: crate::analyzer::AnalyzedRelateEndpoint,
        /// Optional relationship type filter (only remove specific type)
        relation_type: Option<String>,
        /// Optional branch override
        branch_override: Option<String>,
    },

    /// Apply a scalar function to each row and add result as a new column.
    /// Used for `LATERAL function() AS alias` in FROM clause.
    LateralMap {
        input: Box<LogicalPlan>,
        /// The function call expression to evaluate per row
        function_expr: TypedExpr,
        /// Output column name (from the LATERAL alias)
        column_name: String,
    },

    /// Empty plan (for DDL statements that bypass logical planning)
    Empty,
}
