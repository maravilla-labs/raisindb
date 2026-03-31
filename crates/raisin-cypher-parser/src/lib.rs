// SPDX-License-Identifier: BSL-1.1

//! # Cypher Query Parser
//!
//! A parser for the openCypher query language, built with nom 8.0.
//!
//! This crate provides an ergonomic API for parsing openCypher queries into an
//! Abstract Syntax Tree (AST). It's designed to integrate seamlessly with the
//! raisin-sql crate for implementing graph database functionality.
//!
//! ## Features
//!
//! - Complete openCypher query parsing
//! - Partial parsing (expressions, patterns, clauses)
//! - Rich error messages with position information
//! - Zero-copy parsing where possible
//! - Type-safe AST with builder methods
//! - Serde support for AST serialization
//!
//! ## Quick Start
//!
//! ```rust
//! use raisin_cypher_parser::{parse_query, parse_expr};
//!
//! // Parse a complete Cypher query
//! let query = "MATCH (n:Person {name: 'Alice'}) RETURN n.age";
//! let ast = parse_query(query)?;
//!
//! // Parse individual expressions
//! let expr = parse_expr("n.name = 'Alice' AND n.age > 25")?;
//! # Ok::<(), raisin_cypher_parser::ParseError>(())
//! ```
//!
//! ## SQL Integration Example
//!
//! ```rust
//! use raisin_cypher_parser::parse_query;
//!
//! // From SQL: SELECT * FROM cypher('MATCH (n:Person) RETURN n.name')
//! fn execute_cypher_function(query_str: &str) -> Result<Vec<String>, String> {
//!     let query = parse_query(query_str)
//!         .map_err(|e| format!("Invalid Cypher query: {}", e))?;
//!
//!     // Execute the graph query...
//!     // query.clauses contains the parsed AST
//!     Ok(vec![])
//! }
//! # execute_cypher_function("MATCH (n) RETURN n").ok();
//! ```
//!
//! ## Error Handling
//!
//! The parser provides detailed error messages with line and column information:
//!
//! ```rust
//! use raisin_cypher_parser::parse_query;
//!
//! let result = parse_query("MATCH (n RETURN n");
//! assert!(result.is_err());
//! // Error: Syntax error at line 1, column 10: expected ')', found 'RETURN'
//! ```

// TODO(v0.2): Re-enable documentation warnings when docs are complete
// #![warn(missing_docs)]
#![warn(clippy::all)]

pub mod ast;
pub mod error;
mod parser;

// Re-export commonly used types for convenience
pub use ast::{
    BinOp, Clause, Direction, Expr, GraphPattern, Literal, NodePattern, Order, OrderBy,
    PathPattern, PatternElement, Query, RelPattern, ReturnItem, Span, Statement, UnOp,
};
pub use error::{ParseError, Result};

/// Parse a complete Cypher query string.
///
/// This is the main entry point for parsing openCypher queries. It parses
/// a complete query and returns a strongly-typed AST.
///
/// # Arguments
///
/// * `input` - The Cypher query string to parse
///
/// # Returns
///
/// Returns a `Result` containing the parsed `Query` AST or a `ParseError`
/// with detailed position information.
///
/// # Examples
///
/// ```rust
/// use raisin_cypher_parser::parse_query;
///
/// // Simple MATCH query
/// let query = parse_query("MATCH (n:Person) RETURN n.name")?;
/// assert_eq!(query.clauses.len(), 2);
///
/// // Complex query with WHERE and ORDER BY
/// let query = parse_query(
///     "MATCH (p:Person)-[:KNOWS]->(f:Person) \
///      WHERE p.age > 25 \
///      RETURN p.name, f.name \
///      ORDER BY p.name ASC \
///      LIMIT 10"
/// )?;
/// # Ok::<(), raisin_cypher_parser::ParseError>(())
/// ```
///
/// # Errors
///
/// Returns a `ParseError` if the input is not a valid Cypher query. The error
/// includes line and column information to help identify the problem.
pub fn parse_query(input: &str) -> Result<Query> {
    parser::parse_query(input)
}

/// Parse a Cypher statement (currently only queries are supported).
///
/// This function parses a complete Cypher statement. In the future, this will
/// support DDL statements (CREATE INDEX, etc.) in addition to queries.
///
/// # Arguments
///
/// * `input` - The Cypher statement to parse
///
/// # Examples
///
/// ```rust
/// use raisin_cypher_parser::{parse_statement, Statement};
///
/// let stmt = parse_statement("MATCH (n) RETURN n")?;
/// match stmt {
///     Statement::Query(query) => {
///         assert_eq!(query.clauses.len(), 2);
///     }
/// }
/// # Ok::<(), raisin_cypher_parser::ParseError>(())
/// ```
pub fn parse_statement(input: &str) -> Result<Statement> {
    parser::parse_statement(input)
}

/// Parse a Cypher expression.
///
/// This function parses individual expressions without requiring a complete query.
/// Useful for parsing property values, WHERE conditions, or RETURN expressions.
///
/// # Arguments
///
/// * `input` - The expression string to parse
///
/// # Examples
///
/// ```rust
/// use raisin_cypher_parser::{parse_expr, Expr};
///
/// // Property access
/// let expr = parse_expr("person.name")?;
///
/// // Boolean expression
/// let expr = parse_expr("age > 18 AND active = true")?;
///
/// // Function call
/// let expr = parse_expr("toUpper(name)")?;
///
/// // List construction
/// let expr = parse_expr("[1, 2, 3, 4]")?;
///
/// // Map construction
/// let expr = parse_expr("{name: 'Alice', age: 30}")?;
/// # Ok::<(), raisin_cypher_parser::ParseError>(())
/// ```
pub fn parse_expr(input: &str) -> Result<Expr> {
    parser::parse_expr(input)
}

/// Parse a graph pattern.
///
/// This function parses graph pattern syntax, which is used in MATCH, CREATE,
/// and MERGE clauses. Useful for parsing pattern fragments independently.
///
/// # Arguments
///
/// * `input` - The pattern string to parse
///
/// # Examples
///
/// ```rust
/// use raisin_cypher_parser::parse_pattern;
///
/// // Simple node pattern
/// let pattern = parse_pattern("(n:Person)")?;
///
/// // Relationship pattern
/// let pattern = parse_pattern("(a)-[:KNOWS]->(b)")?;
///
/// // Complex path with properties
/// let pattern = parse_pattern(
///     "(alice:Person {name: 'Alice'})-[:KNOWS*1..3]->(friend:Person)"
/// )?;
///
/// // Multiple patterns
/// let pattern = parse_pattern("(a)-[:KNOWS]->(b), (b)-[:WORKS_AT]->(c)")?;
/// # Ok::<(), raisin_cypher_parser::ParseError>(())
/// ```
pub fn parse_pattern(input: &str) -> Result<GraphPattern> {
    parser::parse_pattern(input)
}

/// Parse a single graph pattern path.
///
/// This function parses a single path pattern (node-relationship-node sequence)
/// without supporting multiple comma-separated patterns.
///
/// # Arguments
///
/// * `input` - The path pattern string to parse
///
/// # Examples
///
/// ```rust
/// use raisin_cypher_parser::parse_path;
///
/// // Simple path
/// let path = parse_path("(a)-[:KNOWS]->(b)")?;
/// assert_eq!(path.elements.len(), 3); // node, rel, node
///
/// // Named path
/// let path = parse_path("p = (a)-[:KNOWS]->(b)")?;
/// assert_eq!(path.variable, Some("p".to_string()));
/// # Ok::<(), raisin_cypher_parser::ParseError>(())
/// ```
pub fn parse_path(input: &str) -> Result<PathPattern> {
    parser::parse_path(input)
}

/// Configuration options for the parser.
///
/// This struct allows customizing parser behavior. Currently minimal,
/// but can be extended in the future for features like:
/// - Strict vs. lenient mode
/// - openCypher version compatibility
/// - Custom extension support
///
/// # Examples
///
/// ```rust
/// use raisin_cypher_parser::{ParserConfig, Parser};
///
/// let config = ParserConfig::default();
/// let parser = Parser::new(config);
///
/// let query = parser.parse_query("MATCH (n) RETURN n")?;
/// # Ok::<(), raisin_cypher_parser::ParseError>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct ParserConfig {
    _private: (),
}

impl ParserConfig {
    /// Create a new parser configuration with default settings.
    pub fn new() -> Self {
        Self::default()
    }
}

/// A stateful parser instance with configuration.
///
/// While the module-level functions (`parse_query`, etc.) are more convenient
/// for most use cases, this struct allows you to create a configured parser
/// instance that can be reused.
///
/// # Examples
///
/// ```rust
/// use raisin_cypher_parser::{Parser, ParserConfig};
///
/// let parser = Parser::new(ParserConfig::default());
///
/// // Parse multiple queries with the same configuration
/// let q1 = parser.parse_query("MATCH (n) RETURN n")?;
/// let q2 = parser.parse_query("CREATE (n:Person {name: 'Bob'})")?;
/// # Ok::<(), raisin_cypher_parser::ParseError>(())
/// ```
#[derive(Debug, Clone)]
pub struct Parser {
    #[allow(dead_code)]
    config: ParserConfig,
}

impl Parser {
    /// Create a new parser with the given configuration.
    pub fn new(config: ParserConfig) -> Self {
        Self { config }
    }

    /// Parse a complete Cypher query.
    ///
    /// See [`parse_query`] for examples and documentation.
    pub fn parse_query(&self, input: &str) -> Result<Query> {
        parser::parse_query(input)
    }

    /// Parse a Cypher statement.
    ///
    /// See [`parse_statement`] for examples and documentation.
    pub fn parse_statement(&self, input: &str) -> Result<Statement> {
        parser::parse_statement(input)
    }

    /// Parse a Cypher expression.
    ///
    /// See [`parse_expr`] for examples and documentation.
    pub fn parse_expr(&self, input: &str) -> Result<Expr> {
        parser::parse_expr(input)
    }

    /// Parse a graph pattern.
    ///
    /// See [`parse_pattern`] for examples and documentation.
    pub fn parse_pattern(&self, input: &str) -> Result<GraphPattern> {
        parser::parse_pattern(input)
    }

    /// Parse a single path pattern.
    ///
    /// See [`parse_path`] for examples and documentation.
    pub fn parse_path(&self, input: &str) -> Result<PathPattern> {
        parser::parse_path(input)
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new(ParserConfig::default())
    }
}

/// Extension trait for parsing Cypher from string types.
///
/// This trait provides a convenient `.parse_cypher()` method on string types,
/// similar to the standard library's `.parse()` method.
///
/// # Examples
///
/// ```rust
/// use raisin_cypher_parser::CypherParse;
///
/// let query = "MATCH (n:Person) RETURN n".parse_cypher()?;
/// assert_eq!(query.clauses.len(), 2);
/// # Ok::<(), raisin_cypher_parser::ParseError>(())
/// ```
pub trait CypherParse {
    /// Parse this string as a Cypher query.
    fn parse_cypher(&self) -> Result<Query>;
}

impl CypherParse for str {
    fn parse_cypher(&self) -> Result<Query> {
        parse_query(self)
    }
}

impl CypherParse for String {
    fn parse_cypher(&self) -> Result<Query> {
        parse_query(self)
    }
}

impl CypherParse for &str {
    fn parse_cypher(&self) -> Result<Query> {
        parse_query(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_match() {
        let result = parse_query("MATCH (n) RETURN n");
        assert!(result.is_ok());
        let query = result.unwrap();
        assert_eq!(query.clauses.len(), 2);
    }

    #[test]
    fn test_parse_with_labels() {
        let result = parse_query("MATCH (p:Person) RETURN p.name");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parser_instance() {
        let parser = Parser::default();
        let result = parser.parse_query("MATCH (n) RETURN n");
        assert!(result.is_ok());
    }

    #[test]
    fn test_cypher_parse_trait() {
        let query_str = "MATCH (n) RETURN n";
        let result = query_str.parse_cypher();
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_query_returns_error() {
        let result = parse_query("INVALID SYNTAX HERE");
        assert!(result.is_err());
    }
}
