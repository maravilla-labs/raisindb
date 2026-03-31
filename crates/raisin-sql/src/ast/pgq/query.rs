//! Graph query and clause types
//!
//! Contains the top-level `GraphTableQuery` and clause types (`MatchClause`,
//! `WhereClause`, `ColumnsClause`) for SQL/PGQ GRAPH_TABLE queries.

use serde::{Deserialize, Serialize};

use super::expressions::Expr;
use super::patterns::PathPattern;

/// Default graph name when none is specified
pub const DEFAULT_GRAPH_NAME: &str = "NODES_GRAPH";

/// Source location for error reporting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    /// Start byte offset
    pub start: usize,
    /// End byte offset
    pub end: usize,
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (1-indexed)
    pub column: usize,
}

impl SourceSpan {
    /// Create a new source span
    pub fn new(start: usize, end: usize, line: usize, column: usize) -> Self {
        Self {
            start,
            end,
            line,
            column,
        }
    }

    /// Create an empty/unknown span
    pub fn empty() -> Self {
        Self {
            start: 0,
            end: 0,
            line: 0,
            column: 0,
        }
    }
}

impl Default for SourceSpan {
    fn default() -> Self {
        Self::empty()
    }
}

/// Complete GRAPH_TABLE query
///
/// ```sql
/// SELECT * FROM GRAPH_TABLE(NODES_GRAPH
///   MATCH (a:User)-[:follows]->(b:User)
///   WHERE a.id = 'alice'
///   COLUMNS (a.name, b.name AS friend_name)
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphTableQuery {
    /// Graph name (None = NODES_GRAPH)
    pub graph_name: Option<String>,
    /// MATCH clause pattern
    pub match_clause: MatchClause,
    /// Optional WHERE clause
    pub where_clause: Option<WhereClause>,
    /// COLUMNS clause
    pub columns_clause: ColumnsClause,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

impl GraphTableQuery {
    /// Get effective graph name (defaults to NODES_GRAPH)
    pub fn effective_graph_name(&self) -> &str {
        self.graph_name.as_deref().unwrap_or(DEFAULT_GRAPH_NAME)
    }
}

/// MATCH clause containing graph patterns
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchClause {
    /// One or more path patterns (comma-separated)
    pub patterns: Vec<PathPattern>,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

/// WHERE clause for filtering
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhereClause {
    /// Filter expression
    pub expression: Expr,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

/// COLUMNS clause specifying output columns
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnsClause {
    /// Column expressions
    pub columns: Vec<ColumnExpr>,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

/// Single column expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnExpr {
    /// The expression
    pub expr: Expr,
    /// Optional alias
    pub alias: Option<String>,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

/// System fields that resolve directly (not from properties JSONB)
pub const SYSTEM_FIELDS: &[&str] = &[
    "id",
    "workspace",
    "node_type",
    "path",
    "name",
    "parent_id",
    "created_at",
    "updated_at",
];

/// Check if a field is a system field
pub fn is_system_field(name: &str) -> bool {
    SYSTEM_FIELDS.contains(&name)
}
