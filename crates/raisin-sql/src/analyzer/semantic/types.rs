//! Type definitions for semantic analysis
//!
//! This module contains all the public types used in semantic analysis,
//! including analyzed statements, queries, joins, and table references.

use super::super::{
    catalog::{SchemaTableKind, TableDef},
    typed_expr::TypedExpr,
    types::DataType,
};
use crate::logical_plan::operators::AggregateExpr;

/// Order by specification for a single expression
///
/// Captures the full ordering semantics including direction and nulls handling.
#[derive(Debug, Clone)]
pub struct OrderBySpec {
    /// The expression to order by
    pub expr: TypedExpr,
    /// Sort direction: true = descending, false = ascending
    pub descending: bool,
    /// Nulls ordering: Some(true) = NULLS FIRST, Some(false) = NULLS LAST, None = default
    /// Default is NULLS LAST for ASC, NULLS FIRST for DESC (PostgreSQL behavior)
    pub nulls_first: Option<bool>,
}

impl OrderBySpec {
    /// Create a new order by spec with default nulls handling
    pub fn new(expr: TypedExpr, descending: bool) -> Self {
        Self {
            expr,
            descending,
            nulls_first: None,
        }
    }

    /// Create a new order by spec with explicit nulls handling
    pub fn with_nulls(expr: TypedExpr, descending: bool, nulls_first: Option<bool>) -> Self {
        Self {
            expr,
            descending,
            nulls_first,
        }
    }

    /// Returns true if nulls should sort first
    /// Uses PostgreSQL default behavior when nulls_first is None:
    /// - ASC: NULLS LAST (default)
    /// - DESC: NULLS FIRST (default)
    pub fn nulls_first(&self) -> bool {
        match self.nulls_first {
            Some(nf) => nf,
            None => self.descending, // DESC = NULLS FIRST, ASC = NULLS LAST
        }
    }
}

/// Analyzed statement (typed and validated)
#[derive(Debug, Clone)]
pub enum AnalyzedStatement {
    Query(AnalyzedQuery),
    Explain(ExplainStatement),
    Insert(AnalyzedInsert),
    Update(AnalyzedUpdate),
    Delete(AnalyzedDelete),
    // DDL statements for schema management
    Ddl(crate::ast::ddl::DdlStatement),
    // Transaction control statements
    Transaction(crate::ast::transaction::TransactionStatement),
    // ORDER statements for node sibling positioning
    Order(AnalyzedOrder),
    // MOVE statements for relocating nodes to new parents
    Move(AnalyzedMove),
    // COPY statements for duplicating nodes
    Copy(AnalyzedCopy),
    // TRANSLATE statements for locale-aware content updates
    Translate(AnalyzedTranslate),
    // RELATE statements for creating relationships between nodes
    Relate(AnalyzedRelate),
    // UNRELATE statements for removing relationships between nodes
    Unrelate(AnalyzedUnrelate),
    // BRANCH statements for branch management (CREATE/DROP/ALTER/MERGE/USE)
    Branch(crate::ast::branch::BranchStatement),
    // SHOW statements for PostgreSQL configuration variables
    Show(AnalyzedShow),
    // RESTORE statements for restoring nodes to previous revisions
    Restore(AnalyzedRestore),
    // Access control statements (CREATE/ALTER/DROP ROLE/GROUP/USER, GRANT/REVOKE, etc.)
    Acl(crate::ast::acl::AclStatement),
}

/// Analyzed ORDER statement for node sibling positioning
///
/// ```sql
/// ORDER Page SET path='/content/page1' ABOVE path='/content/page2'
/// ORDER BlogPost SET id='abc123' BELOW path='/target'
/// ```
#[derive(Debug, Clone)]
pub struct AnalyzedOrder {
    /// The table/node type name (e.g., "Page", "BlogPost")
    pub table: String,
    /// The resolved workspace from the table name
    pub workspace: String,
    /// The source node being moved
    pub source: crate::ast::order::NodeReference,
    /// The positioning directive (ABOVE or BELOW)
    pub position: crate::ast::order::OrderPosition,
    /// The target node to position relative to
    pub target: crate::ast::order::NodeReference,
    /// Optional branch override (from IN BRANCH clause)
    /// None = use default branch, Some(name) = operate on specific branch
    pub branch_override: Option<String>,
}

/// Analyzed MOVE statement for relocating nodes to new parents
///
/// ```sql
/// MOVE Page SET path='/content/page1' TO path='/archive'
/// MOVE BlogPost SET id='abc123' TO id='target-parent-id'
/// ```
///
/// Moves a node (and all its descendants) to become a child of the target parent.
/// Node IDs are preserved during the move operation.
#[derive(Debug, Clone)]
pub struct AnalyzedMove {
    /// The table/node type name (e.g., "Page", "BlogPost")
    pub table: String,
    /// The resolved workspace from the table name
    pub workspace: String,
    /// The source node being moved (can be path or ID reference)
    pub source: crate::ast::order::NodeReference,
    /// The target parent node (where to move the node to)
    pub target_parent: crate::ast::order::NodeReference,
    /// Optional branch override (from IN BRANCH clause)
    /// None = use default branch, Some(name) = operate on specific branch
    pub branch_override: Option<String>,
}

/// Analyzed COPY statement for duplicating nodes
///
/// ```sql
/// COPY Page SET path='/content/page1' TO path='/archive'
/// COPY Page SET id='abc123' TO path='/archive' AS 'new-name'
/// COPY TREE BlogPost SET path='/blog' TO path='/archive'
/// ```
///
/// Copies a node (and optionally its descendants if COPY TREE) to become a child
/// of the target parent. New node IDs are generated. Publish state is cleared.
#[derive(Debug, Clone)]
pub struct AnalyzedCopy {
    /// The table/node type name (e.g., "Page", "BlogPost")
    pub table: String,
    /// The resolved workspace from the table name
    pub workspace: String,
    /// The source node being copied (can be path or ID reference)
    pub source: crate::ast::order::NodeReference,
    /// The target parent node (where to copy the node to)
    pub target_parent: crate::ast::order::NodeReference,
    /// Optional new name for the copied node (from AS 'name' clause)
    /// If None, uses the source node's name
    pub new_name: Option<String>,
    /// Whether to copy recursively (COPY TREE) or just the single node (COPY)
    pub recursive: bool,
    /// Optional branch override (from IN BRANCH clause)
    /// None = use default branch, Some(name) = operate on specific branch
    pub branch_override: Option<String>,
}

/// Analyzed TRANSLATE statement for locale-aware content updates
///
/// ```sql
/// UPDATE Page FOR LOCALE 'de' SET title = 'Titel' WHERE path = '/post'
/// UPDATE Article FOR LOCALE 'fr' SET metadata.author = 'Jean' WHERE id = 'abc'
/// UPDATE Page FOR LOCALE 'de' SET blocks[uuid='550e8400'].text = 'Hallo' WHERE path = '/post'
/// ```
///
/// Updates translations for nodes in a specific locale.
/// Node-level translations use JsonPointer format.
/// Block-level translations are stored separately keyed by block UUID.
#[derive(Debug, Clone)]
pub struct AnalyzedTranslate {
    /// The table/node type name (e.g., "Page", "BlogPost")
    pub table: String,
    /// The resolved workspace from the table name
    pub workspace: String,
    /// The target locale code (e.g., "de", "fr", "en-US")
    pub locale: String,
    /// Node-level translations: JsonPointer -> value
    /// e.g., "/title" -> "Titel", "/metadata/author" -> "Jean"
    pub node_translations: std::collections::HashMap<String, AnalyzedTranslationValue>,
    /// Block-level translations: block_uuid -> (JsonPointer -> value)
    /// e.g., "550e8400" -> { "/content/text" -> "Hallo" }
    pub block_translations: std::collections::HashMap<
        String,
        std::collections::HashMap<String, AnalyzedTranslationValue>,
    >,
    /// Filter to select nodes to translate
    pub filter: Option<AnalyzedTranslateFilter>,
    /// Optional branch override (from IN BRANCH clause)
    /// None = use default branch, Some(name) = operate on specific branch
    pub branch_override: Option<String>,
}

/// A translation value that has been analyzed and validated
#[derive(Debug, Clone)]
pub enum AnalyzedTranslationValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
}

/// Filter for TRANSLATE statement WHERE clause
#[derive(Debug, Clone)]
pub enum AnalyzedTranslateFilter {
    /// Filter by path: WHERE path = '/post'
    Path(String),
    /// Filter by ID: WHERE id = 'abc123'
    Id(String),
    /// Filter by path and node_type: WHERE path = '/post' AND node_type = 'Article'
    PathAndType { path: String, node_type: String },
    /// Filter by ID and node_type: WHERE id = 'abc' AND node_type = 'Article'
    IdAndType { id: String, node_type: String },
    /// Filter by node_type only (bulk update): WHERE node_type = 'Article'
    NodeType(String),
}

/// Analyzed RELATE statement for creating relationships between nodes
///
/// ```sql
/// RELATE FROM path='/content/page' TO path='/assets/image' TYPE 'references';
/// RELATE FROM path='/page' IN WORKSPACE 'main' TO path='/asset' IN WORKSPACE 'media' WEIGHT 1.5;
/// ```
///
/// Creates a directed relationship from source node to target node.
/// Supports cross-workspace relationships with optional weight and type.
#[derive(Debug, Clone)]
pub struct AnalyzedRelate {
    /// Source node reference
    pub source: AnalyzedRelateEndpoint,
    /// Target node reference
    pub target: AnalyzedRelateEndpoint,
    /// Relationship type (defaults to "references" if not specified)
    pub relation_type: String,
    /// Optional weight for graph algorithms
    pub weight: Option<f64>,
    /// Optional branch override (from IN BRANCH clause)
    pub branch_override: Option<String>,
}

/// Analyzed UNRELATE statement for removing relationships between nodes
///
/// ```sql
/// UNRELATE FROM path='/content/page' TO path='/assets/image';
/// UNRELATE FROM path='/page' TO path='/asset' TYPE 'tagged';
/// ```
///
/// Removes a directed relationship from source node to target node.
#[derive(Debug, Clone)]
pub struct AnalyzedUnrelate {
    /// Source node reference
    pub source: AnalyzedRelateEndpoint,
    /// Target node reference
    pub target: AnalyzedRelateEndpoint,
    /// Optional relationship type filter (if specified, only removes this type)
    pub relation_type: Option<String>,
    /// Optional branch override (from IN BRANCH clause)
    pub branch_override: Option<String>,
}

/// Endpoint for RELATE/UNRELATE statements
#[derive(Debug, Clone)]
pub struct AnalyzedRelateEndpoint {
    /// Node reference (path or id)
    pub node_ref: crate::ast::relate::RelateNodeReference,
    /// Resolved workspace name (uses default if not specified)
    pub workspace: String,
}

/// Target table for DML operations
#[derive(Debug, Clone)]
pub enum DmlTableTarget {
    /// Schema table (NodeTypes, Archetypes, ElementTypes)
    SchemaTable(SchemaTableKind),
    /// Workspace/nodes table (future support)
    Workspace(String),
}

impl DmlTableTarget {
    /// Get the table name for display/error messages
    pub fn table_name(&self) -> String {
        match self {
            DmlTableTarget::SchemaTable(kind) => kind.table_name().to_string(),
            DmlTableTarget::Workspace(name) => name.clone(),
        }
    }
}

/// Analyzed INSERT statement
///
/// Also used for UPSERT statements when `is_upsert` is true.
/// UPSERT uses `put_node()` (create-or-update) instead of `add_node()` (create-only).
#[derive(Debug, Clone)]
pub struct AnalyzedInsert {
    /// Target table
    pub target: DmlTableTarget,
    /// Table schema (for column validation)
    pub schema: TableDef,
    /// Column names being inserted (if specified, empty means all columns)
    pub columns: Vec<String>,
    /// Values to insert - each inner Vec is a row, containing typed expressions
    pub values: Vec<Vec<TypedExpr>>,
    /// Whether this is an UPSERT operation (create-or-update) vs INSERT (create-only)
    /// When true, uses `put_node()` which will update if node exists at path
    /// When false, uses `add_node()` which fails if node already exists
    pub is_upsert: bool,
}

/// Analyzed UPDATE statement
#[derive(Debug, Clone)]
pub struct AnalyzedUpdate {
    /// Target table
    pub target: DmlTableTarget,
    /// Table schema (for column validation)
    pub schema: TableDef,
    /// SET clause assignments: (column_name, new_value_expression)
    pub assignments: Vec<(String, TypedExpr)>,
    /// WHERE clause filter (None means update all rows - dangerous!)
    pub filter: Option<TypedExpr>,
    /// Optional branch override (from __branch = 'x' in WHERE clause)
    /// None = use default branch, Some(name) = operate on specific branch
    pub branch_override: Option<String>,
}

/// Analyzed DELETE statement
#[derive(Debug, Clone)]
pub struct AnalyzedDelete {
    /// Target table
    pub target: DmlTableTarget,
    /// Table schema (for column validation)
    pub schema: TableDef,
    /// WHERE clause filter (None means delete all rows - dangerous!)
    pub filter: Option<TypedExpr>,
    /// Optional branch override (from __branch = 'x' in WHERE clause)
    /// None = use default branch, Some(name) = operate on specific branch
    pub branch_override: Option<String>,
}

/// EXPLAIN statement with options
#[derive(Debug, Clone)]
pub struct ExplainStatement {
    /// The query to explain
    pub query: Box<AnalyzedQuery>,
    /// Whether to include actual execution (EXPLAIN ANALYZE)
    pub analyze: bool,
    /// Format for output (TEXT, JSON)
    pub format: ExplainFormat,
    /// Whether to show verbose details (logical + optimized plans)
    pub verbose: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplainFormat {
    Text,
    Json,
}

/// DISTINCT specification for SELECT queries
#[derive(Debug, Clone)]
pub enum AnalyzedDistinct {
    /// Simple DISTINCT: eliminate duplicate rows based on all projected columns
    All,
    /// DISTINCT ON(columns): PostgreSQL extension - keeps first row per distinct key
    On(Vec<TypedExpr>),
}

#[derive(Debug, Clone)]
pub struct AnalyzedQuery {
    /// CTE (Common Table Expression) definitions
    /// Each CTE is a (name, query) pair that can be referenced in the main query
    pub ctes: Vec<(String, Box<AnalyzedQuery>)>,
    pub projection: Vec<(TypedExpr, Option<String>)>, // (expr, alias)
    pub from: Vec<TableRef>,
    pub joins: Vec<JoinInfo>,
    pub selection: Option<TypedExpr>,
    pub group_by: Vec<TypedExpr>,       // GROUP BY expressions
    pub aggregates: Vec<AggregateExpr>, // Aggregate function calls
    pub order_by: Vec<OrderBySpec>,     // ORDER BY specifications
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    /// Maximum revision to query (for point-in-time queries)
    /// None = HEAD (latest), Some(rev) = specific revision
    pub max_revision: Option<raisin_hlc::HLC>,
    /// Branch override for cross-branch queries
    /// None = use QueryEngine's default branch, Some(name) = query specific branch
    pub branch_override: Option<String>,
    /// Locales extracted from WHERE clause (e.g., WHERE locale = 'en' or WHERE locale IN ('en', 'de'))
    /// Empty vec = no locale filtering, use default behavior
    /// Non-empty vec = use these locales for translation resolution, return one row per locale per node
    pub locales: Vec<String>,
    /// DISTINCT specification (None = no distinct)
    pub distinct: Option<AnalyzedDistinct>,
}

/// CTE (Common Table Expression) definition
/// Represents a WITH clause that can be referenced as a table in queries
#[derive(Debug, Clone)]
pub struct CteDefinition {
    /// Name of the CTE (table alias)
    pub name: String,
    /// The analyzed query that defines the CTE
    pub query: Box<AnalyzedQuery>,
    /// Inferred schema from the CTE's SELECT list
    pub schema: TableDef,
}

/// Join information
#[derive(Debug, Clone)]
pub struct JoinInfo {
    pub join_type: JoinType,
    pub right_table: TableRef,
    pub condition: Option<TypedExpr>,
}

/// Join type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

#[derive(Debug, Clone)]
pub struct TableRef {
    pub table: String,
    pub alias: Option<String>,
    /// The workspace name if this table reference is to a workspace
    pub workspace: Option<String>,
    /// Table-valued function metadata, if this reference is a table function call
    pub table_function: Option<TableFunctionRef>,
    /// Subquery (derived table) metadata, if this reference is a subquery in FROM clause
    pub subquery: Option<SubqueryRef>,
    /// LATERAL function metadata, if this reference is a LATERAL function call in FROM clause
    pub lateral_function: Option<LateralFunctionRef>,
}

impl TableRef {
    /// Get the name to use for column resolution (alias if present, otherwise table name)
    pub fn name(&self) -> &str {
        self.alias.as_ref().unwrap_or(&self.table)
    }

    /// Check if this is a workspace table
    pub fn is_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    /// Check if this reference is a table-valued function
    pub fn is_table_function(&self) -> bool {
        self.table_function.is_some()
    }

    /// Check if this reference is a subquery (derived table)
    pub fn is_subquery(&self) -> bool {
        self.subquery.is_some()
    }
}

/// Metadata for table-valued function references
#[derive(Debug, Clone)]
pub struct TableFunctionRef {
    pub name: String,
    pub args: Vec<TypedExpr>,
    pub schema: TableDef,
}

/// Metadata for LATERAL function references in FROM clause
///
/// Represents `LATERAL func(args) AS alias` syntax where a scalar function
/// is applied per-row and the result is added as a new column.
#[derive(Debug, Clone)]
pub struct LateralFunctionRef {
    /// The analyzed function call expression
    pub function_expr: TypedExpr,
    /// Output column name (from alias)
    pub column_name: String,
    /// Function return type
    pub return_type: DataType,
}

/// Metadata for subquery (derived table) references in FROM clause
#[derive(Debug, Clone)]
pub struct SubqueryRef {
    /// The analyzed subquery
    pub query: Box<AnalyzedQuery>,
    /// The schema (columns) exposed by this subquery
    pub schema: TableDef,
    /// Whether this is a LATERAL subquery (can reference outer columns)
    pub is_lateral: bool,
}

/// Analyzed SHOW statement for PostgreSQL configuration variables
///
/// Handles JDBC driver initialization queries like:
/// ```sql
/// SHOW TRANSACTION ISOLATION LEVEL
/// SHOW server_version
/// SHOW client_encoding
/// ```
#[derive(Debug, Clone)]
pub struct AnalyzedShow {
    /// The variable name being queried (e.g., "transaction isolation level", "server_version")
    pub variable: String,
}

/// Analyzed RESTORE statement for restoring nodes to previous revision states
///
/// ```sql
/// RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2
/// RESTORE TREE NODE path='/products/category' TO REVISION HEAD~5
/// RESTORE NODE id='uuid' TO REVISION HEAD~2 TRANSLATIONS ('en', 'de')
/// ```
///
/// Restores a node (and optionally its descendants) to its state at a previous revision.
/// The node stays at its current path - this is an in-place restore, not a copy.
#[derive(Debug, Clone)]
pub struct AnalyzedRestore {
    /// The node to restore (by path or id)
    pub node: crate::ast::order::NodeReference,
    /// The revision reference to restore from (resolved to HLC at execution time)
    pub revision: crate::ast::branch::RevisionRef,
    /// Whether to restore children (RESTORE TREE NODE)
    pub recursive: bool,
    /// Specific translations to restore (None = all translations)
    pub translations: Option<Vec<String>>,
    /// Optional branch override
    /// None = use default branch, Some(name) = operate on specific branch
    pub branch_override: Option<String>,
}
