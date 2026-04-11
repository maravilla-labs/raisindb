// TODO(v0.2): Semantic analysis helpers for future SQL features
#![allow(dead_code)]

//! Semantic Analysis
//!
//! This module performs type checking and semantic validation of SQL ASTs.
//!
//! # Module Organization
//!
//! The semantic analyzer is organized into focused submodules:
//!
//! - `types` - Public type definitions (AnalyzedQuery, TableRef, etc.)
//! - `query` - Statement, query, SELECT, CTE, and projection analysis
//! - `expression/` - Expression analysis (identifiers, literals, comparisons, etc.)
//! - `from_clause` - FROM clause, table factors, and JOIN analysis
//! - `functions` - Function call analysis (scalar, aggregate, window)
//! - `grouping` - GROUP BY, ORDER BY, DISTINCT, aggregates
//! - `operators` - Binary/unary operator and type coercion
//! - `json_ops` - JSON operator analysis (->, ->>, @>, etc.)
//! - `dml` - INSERT, UPDATE, DELETE analysis
//! - `predicates` - Predicate extraction utilities (branch, revision, locale)
//! - `equivalence` - Expression equivalence checking

mod dml;
mod equivalence;
mod expression;
mod from_clause;
mod functions;
mod grouping;
mod json_ops;
mod operators;
mod predicates;
mod query;
mod types;

// Re-export public types
pub use types::*;

use super::{
    catalog::{Catalog, ColumnDef, TableDef},
    error::{AnalysisError, Result},
    functions::FunctionRegistry,
    types::DataType,
};
use std::collections::HashMap;

/// Semantic analyzer context
///
/// Holds shared state for analyzing SQL statements including
/// catalog references, function registry, and current table scope.
pub(super) struct AnalyzerContext<'a> {
    catalog: &'a dyn Catalog,
    functions: &'a FunctionRegistry,
    current_tables: Vec<TableRef>,
    /// CTE catalog for name resolution.
    /// Maps CTE name to its definition (schema).
    cte_catalog: HashMap<String, CteDefinition>,
    /// Whether the current INSERT statement is actually an UPSERT.
    /// Set to true before analyzing an UPSERT statement.
    is_upsert: bool,
}

impl<'a> AnalyzerContext<'a> {
    pub fn new(catalog: &'a dyn Catalog, functions: &'a FunctionRegistry) -> Self {
        Self {
            catalog,
            functions,
            current_tables: Vec::new(),
            cte_catalog: HashMap::new(),
            is_upsert: false,
        }
    }

    /// Set the upsert flag for the next INSERT analysis
    pub fn set_upsert(&mut self, is_upsert: bool) {
        self.is_upsert = is_upsert;
    }

    /// Get table definition, checking CTEs, regular tables, workspace tables,
    /// and table-valued functions
    pub(super) fn get_table_def(
        &self,
        table_name: &str,
    ) -> Result<Option<super::catalog::TableDef>> {
        // First check CTEs (Common Table Expressions)
        if let Some(cte_def) = self.cte_catalog.get(table_name) {
            tracing::debug!(
                "Found CTE '{}' with {} columns",
                table_name,
                cte_def.schema.columns.len()
            );
            return Ok(Some(cte_def.schema.clone()));
        }

        // Check pg_catalog virtual tables
        if let Some(table_def) = crate::analyzer::pg_catalog::get_pg_catalog_table(table_name) {
            tracing::debug!(
                "Found pg_catalog table '{}' with {} columns",
                table_name,
                table_def.columns.len()
            );
            return Ok(Some(table_def));
        }

        // Then check regular tables
        if let Some(table) = self.catalog.get_table(table_name) {
            return Ok(Some(table.clone()));
        }

        // Check table-valued functions
        if let Some(tvf_def) = self.get_table_valued_function_def(table_name) {
            return Ok(Some(tvf_def));
        }

        // Then check workspace tables
        if let Some(workspace_table) = self.catalog.get_workspace_table(table_name) {
            return Ok(Some(workspace_table));
        }

        Ok(None)
    }

    /// Get table definition for table-valued functions (CYPHER, GRAPH_TABLE, KNN, etc.)
    fn get_table_valued_function_def(&self, table_name: &str) -> Option<TableDef> {
        let table_upper = table_name.to_uppercase();

        match table_upper.as_str() {
            "FULLTEXT_SEARCH" => Some(Self::fulltext_search_table_def(table_name)),
            "HYBRID_SEARCH" => Some(Self::hybrid_search_table_def(table_name)),
            "CYPHER" => Some(Self::cypher_table_def(table_name)),
            "GRAPH_TABLE" => Some(Self::graph_table_def(table_name)),
            "KNN" => Some(Self::knn_table_def(table_name)),
            "NEIGHBORS" => Some(Self::neighbors_table_def(table_name)),
            _ => None,
        }
    }

    fn fulltext_search_table_def(name: &str) -> TableDef {
        TableDef {
            name: name.to_string(),
            columns: vec![
                ColumnDef::simple("node_id", DataType::Text),
                ColumnDef::simple("workspace_id", DataType::Text),
                ColumnDef::simple("name", DataType::Text),
                ColumnDef::simple("path", DataType::Text),
                ColumnDef::simple("node_type", DataType::Text),
                ColumnDef::simple("score", DataType::Double),
                ColumnDef::simple("revision", DataType::BigInt),
                ColumnDef::simple("properties", DataType::JsonB),
                ColumnDef::nullable("created_at", DataType::Text),
                ColumnDef::nullable("updated_at", DataType::Text),
            ],
            primary_key: vec![],
            indexes: vec![],
        }
    }

    fn hybrid_search_table_def(name: &str) -> TableDef {
        TableDef {
            name: name.to_string(),
            columns: vec![
                ColumnDef::simple("node_id", DataType::Text),
                ColumnDef::simple("workspace_id", DataType::Text),
                ColumnDef::simple("name", DataType::Text),
                ColumnDef::simple("path", DataType::Text),
                ColumnDef::simple("node_type", DataType::Text),
                ColumnDef::simple("score", DataType::Double),
                ColumnDef::nullable("fulltext_rank", DataType::BigInt),
                ColumnDef::nullable("vector_rank", DataType::BigInt),
                ColumnDef::nullable("vector_distance", DataType::Double),
                ColumnDef::simple("revision", DataType::BigInt),
                ColumnDef::simple("properties", DataType::JsonB),
            ],
            primary_key: vec![],
            indexes: vec![],
        }
    }

    fn cypher_table_def(name: &str) -> TableDef {
        // CYPHER returns dynamic columns based on RETURN clause
        TableDef {
            name: name.to_string(),
            columns: vec![],
            primary_key: vec![],
            indexes: vec![],
        }
    }

    fn graph_table_def(name: &str) -> TableDef {
        // GRAPH_TABLE (SQL/PGQ) returns columns based on COLUMNS clause
        TableDef {
            name: name.to_string(),
            columns: vec![
                ColumnDef::simple("id", DataType::Text),
                ColumnDef::simple("path", DataType::Text),
                ColumnDef::simple("name", DataType::Text),
                ColumnDef::simple("node_type", DataType::Text),
                ColumnDef::simple("workspace", DataType::Text),
                ColumnDef::simple("properties", DataType::JsonB),
                ColumnDef::nullable("created_at", DataType::Text),
                ColumnDef::nullable("updated_at", DataType::Text),
                ColumnDef::nullable("type", DataType::Text),
                ColumnDef::nullable("relation_type", DataType::Text),
                ColumnDef::nullable("weight", DataType::Double),
            ],
            primary_key: vec![],
            indexes: vec![],
        }
    }

    fn knn_table_def(name: &str) -> TableDef {
        TableDef {
            name: name.to_string(),
            columns: vec![ColumnDef::nullable("result", DataType::JsonB)],
            primary_key: vec![],
            indexes: vec![],
        }
    }

    fn neighbors_table_def(name: &str) -> TableDef {
        TableDef {
            name: name.to_string(),
            columns: vec![
                ColumnDef::simple("id", DataType::Text),
                ColumnDef::simple("path", DataType::Text),
                ColumnDef::simple("__node_type", DataType::Text),
                ColumnDef::simple("node_type", DataType::Text),
                ColumnDef::simple("name", DataType::Text),
                ColumnDef::nullable("created_at", DataType::Text),
                ColumnDef::nullable("updated_at", DataType::Text),
                ColumnDef::nullable("version", DataType::BigInt),
                ColumnDef::simple("properties", DataType::JsonB),
                ColumnDef::nullable("relation_type", DataType::Text),
                ColumnDef::nullable("weight", DataType::Double),
            ],
            primary_key: vec![],
            indexes: vec![],
        }
    }
}

/// Parse an interval string (e.g., "1 hour", "3 hour 30 minutes") into a
/// chrono::Duration.
///
/// Supports multiple number-unit pairs separated by spaces.
///
/// Note: Due to variable lengths, approximate conversions are used:
/// - 1 year = 365.25 days (accounting for leap years)
/// - 1 month = 30 days
pub(crate) fn parse_interval_string(s: &str) -> Result<chrono::Duration> {
    let s = s.trim();

    let tokens: Vec<&str> = s.split_whitespace().collect();

    if tokens.is_empty() || tokens.len() % 2 != 0 {
        return Err(AnalysisError::UnsupportedExpression(format!(
            "Invalid INTERVAL format '{}'. Expected format: '<number> <unit>' or '<number> <unit> <number> <unit>' (e.g., '1 hour' or '3 hour 30 minutes')",
            s
        )));
    }

    let mut total_duration = chrono::Duration::zero();

    for chunk in tokens.chunks(2) {
        let number_str = chunk[0];
        let unit = chunk[1].to_lowercase();

        let number: f64 = number_str.parse().map_err(|_| {
            AnalysisError::UnsupportedExpression(format!(
                "Invalid INTERVAL number '{}'. Must be a valid number (integer or decimal)",
                number_str
            ))
        })?;

        // Convert to microseconds (chrono's base unit) to handle fractional values
        let microseconds = match unit.as_str() {
            "microsecond" | "microseconds" | "us" => number,
            "millisecond" | "milliseconds" | "ms" => number * 1_000.0,
            "second" | "seconds" | "sec" | "secs" | "s" => number * 1_000_000.0,
            "minute" | "minutes" | "min" | "mins" => number * 60_000_000.0,
            "hour" | "hours" | "hr" | "hrs" | "h" => number * 3_600_000_000.0,
            "day" | "days" | "d" => number * 86_400_000_000.0,
            "week" | "weeks" | "w" => number * 604_800_000_000.0,
            "month" | "months" | "mon" | "mons" => number * 30.0 * 86_400_000_000.0,
            "year" | "years" | "yr" | "yrs" | "y" => number * 365.25 * 86_400_000_000.0,
            _ => {
                return Err(AnalysisError::UnsupportedExpression(format!(
                    "Unsupported INTERVAL unit '{}'. Supported units: microseconds, milliseconds, seconds, minutes, hours, days, weeks, months, years",
                    unit
                )))
            }
        };

        total_duration += chrono::Duration::microseconds(microseconds as i64);
    }

    Ok(total_duration)
}

#[cfg(test)]
mod tests {
    use super::predicates::extract_branch_predicate;
    use crate::analyzer::{
        typed_expr::{BinaryOperator, Expr, Literal, TypedExpr},
        types::DataType,
    };

    /// Test simple branch extraction: WHERE __branch = 'staging'
    #[test]
    fn test_branch_extraction_simple() {
        let branch_col =
            TypedExpr::column("nodes".to_string(), "__branch".to_string(), DataType::Text);
        let branch_val = TypedExpr::literal(Literal::Text("staging".to_string()));
        let filter = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(branch_col),
                op: BinaryOperator::Eq,
                right: Box::new(branch_val),
            },
            DataType::Boolean,
        );

        let (branch, remaining) = extract_branch_predicate(&filter);
        assert_eq!(branch, Some("staging".to_string()));
        assert!(remaining.is_none());
    }

    /// Test branch extraction with other predicates: WHERE __branch = 'dev' AND path = '/content'
    #[test]
    fn test_branch_extraction_with_other_predicates() {
        let branch_col =
            TypedExpr::column("nodes".to_string(), "__branch".to_string(), DataType::Text);
        let branch_val = TypedExpr::literal(Literal::Text("dev".to_string()));
        let branch_pred = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(branch_col),
                op: BinaryOperator::Eq,
                right: Box::new(branch_val),
            },
            DataType::Boolean,
        );

        let path_col = TypedExpr::column("nodes".to_string(), "path".to_string(), DataType::Path);
        let path_val = TypedExpr::literal(Literal::Path("/content".to_string()));
        let path_pred = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(path_col),
                op: BinaryOperator::Eq,
                right: Box::new(path_val),
            },
            DataType::Boolean,
        );

        let combined = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(branch_pred),
                op: BinaryOperator::And,
                right: Box::new(path_pred.clone()),
            },
            DataType::Boolean,
        );

        let (branch, remaining) = extract_branch_predicate(&combined);
        assert_eq!(branch, Some("dev".to_string()));
        assert!(remaining.is_some());

        // Verify remaining predicate is the path filter
        let remaining_expr = remaining.unwrap();
        match &remaining_expr.expr {
            Expr::BinaryOp { left, op, right } => {
                assert!(matches!(op, BinaryOperator::Eq));
                assert!(matches!(&left.expr, Expr::Column { column, .. } if column == "path"));
                assert!(matches!(&right.expr, Expr::Literal(Literal::Path(p)) if p == "/content"));
            }
            _ => panic!("Expected binary operation"),
        }
    }

    /// Test branch extraction: WHERE path = '/content' AND __branch = 'feature-x'
    #[test]
    fn test_branch_extraction_order_reversed() {
        let path_col = TypedExpr::column("nodes".to_string(), "path".to_string(), DataType::Path);
        let path_val = TypedExpr::literal(Literal::Path("/content".to_string()));
        let path_pred = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(path_col),
                op: BinaryOperator::Eq,
                right: Box::new(path_val),
            },
            DataType::Boolean,
        );

        let branch_col =
            TypedExpr::column("nodes".to_string(), "__branch".to_string(), DataType::Text);
        let branch_val = TypedExpr::literal(Literal::Text("feature-x".to_string()));
        let branch_pred = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(branch_col),
                op: BinaryOperator::Eq,
                right: Box::new(branch_val),
            },
            DataType::Boolean,
        );

        let combined = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(path_pred.clone()),
                op: BinaryOperator::And,
                right: Box::new(branch_pred),
            },
            DataType::Boolean,
        );

        let (branch, remaining) = extract_branch_predicate(&combined);
        assert_eq!(branch, Some("feature-x".to_string()));
        assert!(remaining.is_some());
    }

    /// Test no branch predicate: WHERE path = '/content'
    #[test]
    fn test_branch_extraction_no_branch() {
        let path_col = TypedExpr::column("nodes".to_string(), "path".to_string(), DataType::Path);
        let path_val = TypedExpr::literal(Literal::Path("/content".to_string()));
        let filter = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(path_col),
                op: BinaryOperator::Eq,
                right: Box::new(path_val),
            },
            DataType::Boolean,
        );

        let (branch, remaining) = extract_branch_predicate(&filter);
        assert_eq!(branch, None);
        assert!(remaining.is_some());
    }
}
