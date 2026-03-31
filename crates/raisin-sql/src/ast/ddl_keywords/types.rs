//! Core types for DDL keyword definitions

use serde::{Deserialize, Serialize};

#[cfg(feature = "ts-export")]
use ts_rs::TS;

/// Keyword with documentation for Monaco hover tooltips
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS))]
#[cfg_attr(feature = "ts-export", ts(export, export_to = "ddl/"))]
pub struct KeywordInfo {
    /// The keyword itself (e.g., "CREATE")
    pub keyword: String,
    /// Category for grouping (e.g., "Statement", "Clause", "Modifier")
    pub category: KeywordCategory,
    /// Human-readable description for hover tooltips
    pub description: String,
    /// Optional syntax pattern
    pub syntax: Option<String>,
    /// Optional example SQL
    pub example: Option<String>,
}

/// Keyword category for syntax highlighting and grouping
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS))]
#[cfg_attr(feature = "ts-export", ts(export, export_to = "ddl/"))]
pub enum KeywordCategory {
    /// DDL statement keywords (CREATE, ALTER, DROP)
    Statement,
    /// Schema object types (NODETYPE, ARCHETYPE, ELEMENTTYPE)
    SchemaObject,
    /// Clauses (EXTENDS, PROPERTIES, FIELDS)
    Clause,
    /// Property types (String, Number, Boolean, etc.)
    PropertyType,
    /// Property modifiers (REQUIRED, UNIQUE, FULLTEXT)
    Modifier,
    /// Boolean flags (VERSIONABLE, PUBLISHABLE)
    Flag,
    /// Operators and special keywords (OF, CASCADE, ADD)
    Operator,
    /// SQL functions (DEPTH, PARENT, FULLTEXT_MATCH)
    SqlFunction,
    /// JSON functions (JSON_VALUE, JSON_EXISTS)
    JsonFunction,
    /// Table-valued functions (KNN, NEIGHBORS, CYPHER)
    TableFunction,
    /// Aggregate functions (COUNT, SUM, AVG)
    AggregateFunction,
    /// Window functions (ROW_NUMBER, RANK)
    WindowFunction,
}

/// All DDL keywords with their documentation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS))]
#[cfg_attr(feature = "ts-export", ts(export, export_to = "ddl/"))]
pub struct DdlKeywords {
    pub keywords: Vec<KeywordInfo>,
}
