//! SQL/PGQ (Property Graph Queries) AST types
//!
//! Defines the abstract syntax tree for `GRAPH_TABLE` queries following ISO SQL:2023.
//!
//! # Grammar Overview
//!
//! ```text
//! GRAPH_TABLE([graph_name]
//!   MATCH graph_pattern
//!   [WHERE filter_expression]
//!   COLUMNS (column_list)
//! )
//! ```
//!
//! # Default Graph
//!
//! When no graph name is specified, `NODES_GRAPH` is used as the default.
//! This graph encompasses all nodes and relations in the database.

mod expressions;
mod patterns;
mod query;

pub use expressions::{BinaryOperator, Expr, Literal, UnaryOperator};
pub use patterns::{
    Direction, NodePattern, PathPattern, PathQuantifier, PatternElement, RelationshipPattern,
};
pub use query::{
    is_system_field, ColumnExpr, ColumnsClause, GraphTableQuery, MatchClause, SourceSpan,
    WhereClause, DEFAULT_GRAPH_NAME, SYSTEM_FIELDS,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_graph_name() {
        let query = GraphTableQuery {
            graph_name: None,
            match_clause: MatchClause {
                patterns: vec![],
                span: SourceSpan::empty(),
            },
            where_clause: None,
            columns_clause: ColumnsClause {
                columns: vec![],
                span: SourceSpan::empty(),
            },
            span: SourceSpan::empty(),
        };
        assert_eq!(query.effective_graph_name(), "NODES_GRAPH");
    }

    #[test]
    fn test_path_quantifier() {
        assert_eq!(PathQuantifier::exact(2).min, 2);
        assert_eq!(PathQuantifier::exact(2).max, Some(2));
        assert_eq!(PathQuantifier::unbounded().effective_max(), 10);
    }

    #[test]
    fn test_system_fields() {
        assert!(is_system_field("id"));
        assert!(is_system_field("workspace"));
        assert!(!is_system_field("title"));
        assert!(!is_system_field("author"));
    }
}
