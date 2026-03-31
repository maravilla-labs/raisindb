// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Prepared statement types and SQL query parser.
//!
//! Contains [`RaisinStatement`] for holding parsed SQL with parameter types,
//! and [`RaisinQueryParser`] which implements pgwire's `QueryParser` trait.

use async_trait::async_trait;
use pgwire::api::stmt::QueryParser;
use pgwire::api::Type;
use pgwire::error::PgWireResult;
use tracing::debug;

/// Represents a prepared SQL statement with parameter type information.
///
/// This struct stores the original SQL text and the inferred types for
/// each parameter placeholder ($1, $2, etc.) in the statement.
#[derive(Debug, Clone)]
pub struct RaisinStatement {
    /// The original SQL query text with placeholders
    pub sql: String,
    /// PostgreSQL types for each parameter placeholder
    pub param_types: Vec<Type>,
}

impl RaisinStatement {
    /// Create a new RaisinStatement with the given SQL and parameter types
    pub fn new(sql: String, param_types: Vec<Type>) -> Self {
        Self { sql, param_types }
    }
}

/// Query parser that analyzes SQL and infers parameter types.
///
/// This parser examines SQL statements to determine the number and types
/// of parameters. For now, it uses a simple heuristic approach.
#[derive(Debug, Clone)]
pub struct RaisinQueryParser;

impl RaisinQueryParser {
    /// Create a new query parser instance
    pub fn new() -> Self {
        Self
    }

    /// Count the number of parameter placeholders ($1, $2, etc.) in SQL
    ///
    /// This is a simple implementation that counts occurrences of $N patterns.
    /// A more sophisticated implementation would parse the SQL properly.
    pub(crate) fn count_parameters(sql: &str) -> usize {
        let mut max_param = 0;
        let mut chars = sql.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '$' {
                let mut num_str = String::new();
                while let Some(&next_ch) = chars.peek() {
                    if next_ch.is_ascii_digit() {
                        num_str.push(next_ch);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if let Ok(num) = num_str.parse::<usize>() {
                    if num > max_param {
                        max_param = num;
                    }
                }
            }
        }

        max_param
    }
}

impl Default for RaisinQueryParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl QueryParser for RaisinQueryParser {
    type Statement = RaisinStatement;

    /// Parse SQL and determine parameter types.
    ///
    /// This method analyzes the SQL to determine how many parameters it has.
    /// It uses client-provided types when available (from the Parse message),
    /// falling back to TEXT for unspecified parameters.
    async fn parse_sql(&self, sql: &str, types: &[Type]) -> PgWireResult<Self::Statement> {
        debug!("Parsing SQL for extended query: {}", sql);
        debug!("Client-provided parameter types: {:?}", types);

        // Count parameters in the SQL
        let param_count = Self::count_parameters(sql);

        // Use client-provided types when available, default to TEXT for unspecified ones
        // This is important for JDBC drivers that send proper types for LIMIT, OFFSET, etc.
        let param_types: Vec<Type> = (0..param_count)
            .map(|i| types.get(i).cloned().unwrap_or(Type::TEXT))
            .collect();

        debug!(
            "Parsed SQL with {} parameters, types: {:?}",
            param_count, param_types
        );

        Ok(RaisinStatement::new(sql.to_string(), param_types))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_parameters() {
        assert_eq!(
            RaisinQueryParser::count_parameters("SELECT * FROM users"),
            0
        );
        assert_eq!(
            RaisinQueryParser::count_parameters("SELECT * FROM users WHERE id = $1"),
            1
        );
        assert_eq!(
            RaisinQueryParser::count_parameters("SELECT * FROM users WHERE id = $1 AND name = $2"),
            2
        );
        assert_eq!(
            RaisinQueryParser::count_parameters("INSERT INTO users (id, name) VALUES ($1, $2)"),
            2
        );
        // Out of order parameters
        assert_eq!(
            RaisinQueryParser::count_parameters("SELECT $2, $1 FROM users"),
            2
        );
        // Higher numbers
        assert_eq!(
            RaisinQueryParser::count_parameters("SELECT $10 FROM users"),
            10
        );
    }

    #[tokio::test]
    async fn test_parse_sql() {
        let parser = RaisinQueryParser::new();

        // Simple query with one parameter
        let stmt = parser
            .parse_sql("SELECT * FROM users WHERE id = $1", &[])
            .await
            .unwrap();
        assert_eq!(stmt.sql, "SELECT * FROM users WHERE id = $1");
        assert_eq!(stmt.param_types.len(), 1);
        assert_eq!(stmt.param_types[0], Type::TEXT);

        // Query with multiple parameters
        let stmt = parser
            .parse_sql("INSERT INTO users (id, name) VALUES ($1, $2)", &[])
            .await
            .unwrap();
        assert_eq!(stmt.param_types.len(), 2);
    }
}
