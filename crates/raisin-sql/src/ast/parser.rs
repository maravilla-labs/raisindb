//! SQL parsing logic for RaisinSQL
//!
//! This module provides the main entry point for parsing RaisinSQL statements
//! and validating them against RaisinDB requirements.
//!
//! # Example
//!
//! ```
//! use raisin_sql::parse_sql;
//!
//! let sql = "SELECT id, name, properties ->> 'title' AS title FROM nodes WHERE PATH_STARTS_WITH(path, '/content/')";
//! let statements = parse_sql(sql).unwrap();
//! assert_eq!(statements.len(), 1);
//! ```

use sqlparser::ast::Statement;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use super::error::{ParseError, Result};
use super::functions::{validate_raisin_functions, validate_table_names_in_query};
use crate::analyzer::catalog::is_schema_table;

/// Main entry point for parsing RaisinSQL statements
///
/// # Arguments
///
/// * `sql` - The SQL string to parse
///
/// # Returns
///
/// A vector of parsed statements or a ParseError
///
/// # Example
///
/// ```
/// use raisin_sql::parse_sql;
///
/// let sql = "SELECT * FROM nodes WHERE id = 'node-123'";
/// let statements = parse_sql(sql).unwrap();
/// assert_eq!(statements.len(), 1);
/// ```
pub fn parse_sql(sql: &str) -> Result<Vec<Statement>> {
    // Use PostgreSQL dialect as base since RaisinSQL is Postgres-style
    let dialect = PostgreSqlDialect {};

    // Parse the SQL
    let statements =
        Parser::parse_sql(&dialect, sql).map_err(|e| ParseError::SqlParserError(e.to_string()))?;

    // Validate RaisinDB-specific constructs
    for stmt in &statements {
        validate_statement(stmt)?;
    }

    Ok(statements)
}

/// Validate that a statement conforms to RaisinDB requirements
pub fn validate_statement(stmt: &Statement) -> Result<()> {
    match stmt {
        Statement::Query(query) => {
            // Validate table names in FROM clause
            validate_table_names_in_query(query)?;
            // Validate RaisinDB-specific functions in the query
            validate_raisin_functions(query)?;
            Ok(())
        }
        Statement::Insert(insert) => {
            // Validate that INSERT targets the 'nodes' table
            if let sqlparser::ast::TableObject::TableName(ref table_name) = insert.table {
                validate_table_name(table_name, "INSERT")?;
            }
            Ok(())
        }
        Statement::Update { table, .. } => {
            // Validate that UPDATE targets the 'nodes' table
            if let Some(table_name) = extract_table_name(&table.relation) {
                validate_table_name(&table_name, "UPDATE")?;
            }
            Ok(())
        }
        Statement::Delete(delete) => {
            // Validate that DELETE targets the 'nodes' table
            for table_name in &delete.tables {
                validate_table_name(table_name, "DELETE")?;
            }
            Ok(())
        }
        _ => {
            // Other statement types are not supported in RaisinSQL
            Err(ParseError::UnsupportedStatement(format!("{:?}", stmt)))
        }
    }
}

/// Extract table name from TableFactor
fn extract_table_name(
    table_factor: &sqlparser::ast::TableFactor,
) -> Option<sqlparser::ast::ObjectName> {
    match table_factor {
        sqlparser::ast::TableFactor::Table { name, .. } => Some(name.clone()),
        _ => None,
    }
}

/// Validate that DML operations target supported tables
///
/// Supported tables:
/// - `nodes` - Main content nodes table
/// - `NodeTypes` - Schema management table for node type definitions
/// - `Archetypes` - Schema management table for archetype definitions
/// - `ElementTypes` - Schema management table for element type definitions
fn validate_table_name(table: &sqlparser::ast::ObjectName, operation: &str) -> Result<()> {
    let table_str = table.to_string();

    // Accept 'nodes' table for DML
    if table_str.eq_ignore_ascii_case("nodes") {
        return Ok(());
    }

    // Accept schema tables (NodeTypes, Archetypes, ElementTypes)
    if is_schema_table(&table_str) {
        return Ok(());
    }

    // Reject unknown tables
    Err(ParseError::InvalidTable {
        operation: operation.to_string(),
        table: table_str,
        expected: "nodes, NodeTypes, Archetypes, or ElementTypes".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_select() {
        let sql = "SELECT * FROM nodes";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_select_with_where() {
        let sql = "SELECT id, name FROM nodes WHERE id = 'node-123'";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_insert_nodes() {
        let sql = r#"INSERT INTO nodes (path, node_type, properties) VALUES ('/content/blog/post1', 'my:Article', '{"title": "Hello"}')"#;
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_update_nodes() {
        let sql =
            r#"UPDATE nodes SET properties = '{"status": "published"}' WHERE id = 'node-123'"#;
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_delete_nodes() {
        let sql = "DELETE FROM nodes WHERE id = 'node-123'";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_table_name() {
        let sql = "SELECT * FROM users";
        let result = parse_sql(sql);
        assert!(result.is_err());
    }

    #[test]
    fn test_json_operators() {
        let sql = "SELECT properties ->> 'title' AS title FROM nodes";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_insert_nodetypes() {
        let sql = r#"INSERT INTO NodeTypes (name, description) VALUES ('Article', 'Blog article')"#;
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_update_nodetypes() {
        let sql =
            r#"UPDATE NodeTypes SET description = 'Updated description' WHERE name = 'Article'"#;
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_delete_nodetypes() {
        let sql = "DELETE FROM NodeTypes WHERE name = 'OldType'";
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_insert_archetypes() {
        let sql =
            r#"INSERT INTO Archetypes (name, title) VALUES ('BlogPost', 'Blog Post Template')"#;
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_insert_elementtypes() {
        let sql = r#"INSERT INTO ElementTypes (name, description) VALUES ('Paragraph', 'Text paragraph element')"#;
        let result = parse_sql(sql);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_table_for_dml() {
        let sql = "INSERT INTO users (name) VALUES ('test')";
        let result = parse_sql(sql);
        assert!(result.is_err());
        if let Err(ParseError::InvalidTable { expected, .. }) = result {
            assert!(expected.contains("NodeTypes"));
            assert!(expected.contains("Archetypes"));
            assert!(expected.contains("ElementTypes"));
        }
    }
}
