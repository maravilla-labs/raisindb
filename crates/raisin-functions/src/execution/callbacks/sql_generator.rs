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

//! SQL statement generators for node operations.
//!
//! This module generates SQL statements that can be executed via QueryEngine,
//! ensuring all node operations go through the unified SQL transaction system.

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use serde_json::Value;
use std::collections::HashMap;

/// Result type for SQL generation - includes the SQL string and parameter values
pub struct SqlStatement {
    /// The SQL statement with $1, $2, etc. placeholders
    pub sql: String,
    /// The parameter values in order
    pub params: Vec<Value>,
}

impl SqlStatement {
    fn new(sql: String, params: Vec<Value>) -> Self {
        Self { sql, params }
    }
}

// ============================================================================
// READ OPERATIONS
// ============================================================================

/// Generate SELECT for getting a node by path
///
/// ```sql
/// SELECT * FROM workspace WHERE path = $1
/// ```
pub fn generate_select_by_path(workspace: &str, path: &str) -> SqlStatement {
    SqlStatement::new(
        format!(
            "SELECT * FROM {} WHERE path = $1",
            escape_identifier(workspace)
        ),
        vec![Value::String(path.to_string())],
    )
}

/// Generate SELECT for getting a node by id
///
/// ```sql
/// SELECT * FROM workspace WHERE id = $1
/// ```
pub fn generate_select_by_id(workspace: &str, id: &str) -> SqlStatement {
    SqlStatement::new(
        format!(
            "SELECT * FROM {} WHERE id = $1",
            escape_identifier(workspace)
        ),
        vec![Value::String(id.to_string())],
    )
}

/// Generate SELECT for getting children of a node
///
/// Uses PATH_STARTS_WITH for proper child matching (direct children only).
///
/// ```sql
/// SELECT * FROM workspace WHERE PATH_STARTS_WITH(path, $1) AND path != $1 LIMIT N
/// ```
pub fn generate_select_children(
    workspace: &str,
    parent_path: &str,
    limit: Option<u32>,
) -> SqlStatement {
    // For direct children, we match paths that:
    // 1. Start with parent_path
    // 2. Have exactly one more depth level (using DEPTH function)
    let sql = if let Some(max) = limit {
        format!(
            "SELECT * FROM {} WHERE PATH_STARTS_WITH(path, $1) AND DEPTH(path) = DEPTH($1) + 1 LIMIT {}",
            escape_identifier(workspace),
            max
        )
    } else {
        format!(
            "SELECT * FROM {} WHERE PATH_STARTS_WITH(path, $1) AND DEPTH(path) = DEPTH($1) + 1",
            escape_identifier(workspace)
        )
    };

    SqlStatement::new(sql, vec![Value::String(parent_path.to_string())])
}

// ============================================================================
// WRITE OPERATIONS
// ============================================================================

/// Generate INSERT for creating a new node
///
/// ```sql
/// INSERT INTO workspace (id, path, node_type, properties) VALUES ($1, $2, $3, $4::JSONB)
/// ```
pub fn generate_insert(workspace: &str, node: &Node) -> SqlStatement {
    let properties_json = properties_to_json(&node.properties);

    SqlStatement::new(
        format!(
            "INSERT INTO {} (id, path, node_type, properties) VALUES ($1, $2, $3, $4::JSONB)",
            escape_identifier(workspace)
        ),
        vec![
            Value::String(node.id.clone()),
            Value::String(node.path.clone()),
            Value::String(node.node_type.clone()),
            properties_json,
        ],
    )
}

/// Generate UPSERT for creating or updating a node
///
/// ```sql
/// UPSERT INTO workspace (id, path, node_type, properties) VALUES ($1, $2, $3, $4::JSONB)
/// ```
pub fn generate_upsert(workspace: &str, node: &Node) -> SqlStatement {
    let properties_json = properties_to_json(&node.properties);

    SqlStatement::new(
        format!(
            "UPSERT INTO {} (id, path, node_type, properties) VALUES ($1, $2, $3, $4::JSONB)",
            escape_identifier(workspace)
        ),
        vec![
            Value::String(node.id.clone()),
            Value::String(node.path.clone()),
            Value::String(node.node_type.clone()),
            properties_json,
        ],
    )
}

/// Generate UPDATE for updating node properties
///
/// ```sql
/// UPDATE workspace SET properties = $1::JSONB WHERE path = $2
/// ```
pub fn generate_update_properties(
    workspace: &str,
    path: &str,
    properties: &HashMap<String, PropertyValue>,
) -> SqlStatement {
    let properties_json = properties_to_json(properties);

    SqlStatement::new(
        format!(
            "UPDATE {} SET properties = $1::JSONB WHERE path = $2",
            escape_identifier(workspace)
        ),
        vec![properties_json, Value::String(path.to_string())],
    )
}

/// Generate UPDATE for a single property using JSON merge
///
/// ```sql
/// UPDATE workspace SET properties = properties || $1::JSONB WHERE path = $2
/// ```
pub fn generate_update_single_property(
    workspace: &str,
    path: &str,
    property_path: &str,
    value: &PropertyValue,
) -> SqlStatement {
    // Build a nested JSON object for the property path
    // e.g., "metadata.author" -> {"metadata": {"author": value}}
    let property_json = build_nested_property_json(property_path, value);

    SqlStatement::new(
        format!(
            "UPDATE {} SET properties = properties || $1::JSONB WHERE path = $2",
            escape_identifier(workspace)
        ),
        vec![property_json, Value::String(path.to_string())],
    )
}

/// Generate DELETE for removing a node by path
///
/// ```sql
/// DELETE FROM workspace WHERE path = $1
/// ```
pub fn generate_delete_by_path(workspace: &str, path: &str) -> SqlStatement {
    SqlStatement::new(
        format!(
            "DELETE FROM {} WHERE path = $1",
            escape_identifier(workspace)
        ),
        vec![Value::String(path.to_string())],
    )
}

/// Generate DELETE for removing a node and its children (cascade)
///
/// ```sql
/// DELETE FROM workspace WHERE path = $1 OR PATH_STARTS_WITH(path, $1 || '/')
/// ```
pub fn generate_delete_cascade(workspace: &str, path: &str) -> SqlStatement {
    SqlStatement::new(
        format!(
            "DELETE FROM {} WHERE path = $1 OR PATH_STARTS_WITH(path, $1 || '/')",
            escape_identifier(workspace)
        ),
        vec![Value::String(path.to_string())],
    )
}

/// Generate MOVE for moving a node to a new path
///
/// Uses the MOVE statement which handles subtree movement.
///
/// ```sql
/// MOVE workspace SET path = $1 WHERE path = $2
/// ```
pub fn generate_move(workspace: &str, old_path: &str, new_path: &str) -> SqlStatement {
    SqlStatement::new(
        format!(
            "MOVE {} SET path = $1 WHERE path = $2",
            escape_identifier(workspace)
        ),
        vec![
            Value::String(new_path.to_string()),
            Value::String(old_path.to_string()),
        ],
    )
}

// ============================================================================
// TRANSACTION STATEMENTS
// ============================================================================

/// Generate BEGIN statement
pub fn generate_begin() -> SqlStatement {
    SqlStatement::new("BEGIN".to_string(), vec![])
}

/// Generate COMMIT statement
pub fn generate_commit() -> SqlStatement {
    SqlStatement::new("COMMIT".to_string(), vec![])
}

/// Generate COMMIT with message and actor
pub fn generate_commit_with_metadata(message: Option<&str>, actor: Option<&str>) -> SqlStatement {
    let mut sql = "COMMIT".to_string();

    if let Some(msg) = message {
        sql.push_str(&format!(" WITH MESSAGE '{}'", escape_string(msg)));
    }
    if let Some(act) = actor {
        sql.push_str(&format!(" WITH ACTOR '{}'", escape_string(act)));
    }

    SqlStatement::new(sql, vec![])
}

/// Generate ROLLBACK statement
pub fn generate_rollback() -> SqlStatement {
    SqlStatement::new("ROLLBACK".to_string(), vec![])
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Escape a SQL identifier (table/workspace name) to prevent injection
fn escape_identifier(name: &str) -> String {
    // Validate the identifier contains only allowed characters
    // RaisinDB workspaces use alphanumeric + underscore
    if name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        name.to_string()
    } else {
        // Quote the identifier if it contains special characters
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// Escape a string value for SQL (single quotes)
fn escape_string(s: &str) -> String {
    s.replace('\'', "''")
}

/// Convert properties HashMap to JSON Value
fn properties_to_json(properties: &HashMap<String, PropertyValue>) -> Value {
    let mut obj = serde_json::Map::new();
    for (key, value) in properties {
        obj.insert(key.clone(), property_value_to_json(value));
    }
    Value::Object(obj)
}

/// Convert a PropertyValue to JSON Value
fn property_value_to_json(pv: &PropertyValue) -> Value {
    match pv {
        PropertyValue::Null => Value::Null,
        PropertyValue::Boolean(b) => Value::Bool(*b),
        PropertyValue::Integer(i) => Value::Number((*i).into()),
        PropertyValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        PropertyValue::Date(d) => Value::String(d.to_rfc3339()),
        PropertyValue::Decimal(d) => Value::String(d.to_string()),
        PropertyValue::String(s) => Value::String(s.clone()),
        PropertyValue::Reference(r) => serde_json::to_value(r).unwrap_or(Value::Null),
        PropertyValue::Url(u) => serde_json::to_value(u).unwrap_or(Value::Null),
        PropertyValue::Resource(r) => serde_json::to_value(r).unwrap_or(Value::Null),
        PropertyValue::Composite(c) => serde_json::to_value(c).unwrap_or(Value::Null),
        PropertyValue::Element(e) => serde_json::to_value(e).unwrap_or(Value::Null),
        PropertyValue::Vector(v) => serde_json::to_value(v).unwrap_or(Value::Null),
        PropertyValue::Geometry(g) => serde_json::to_value(g).unwrap_or(Value::Null),
        PropertyValue::Array(arr) => Value::Array(arr.iter().map(property_value_to_json).collect()),
        PropertyValue::Object(map) => {
            let obj: serde_json::Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), property_value_to_json(v)))
                .collect();
            Value::Object(obj)
        }
    }
}

/// Build a nested JSON object for a property path
///
/// e.g., "metadata.author" with value "John" -> {"metadata": {"author": "John"}}
fn build_nested_property_json(property_path: &str, value: &PropertyValue) -> Value {
    let parts: Vec<&str> = property_path.split('.').collect();
    let json_value = property_value_to_json(value);

    // Build from innermost to outermost
    parts.iter().rev().fold(json_value, |inner, part| {
        let mut obj = serde_json::Map::new();
        obj.insert((*part).to_string(), inner);
        Value::Object(obj)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_select_by_path() {
        let stmt = generate_select_by_path("content", "/articles/post1");
        assert_eq!(stmt.sql, "SELECT * FROM content WHERE path = $1");
        assert_eq!(stmt.params.len(), 1);
        assert_eq!(stmt.params[0], Value::String("/articles/post1".to_string()));
    }

    #[test]
    fn test_generate_select_by_id() {
        let stmt = generate_select_by_id("content", "abc123");
        assert_eq!(stmt.sql, "SELECT * FROM content WHERE id = $1");
        assert_eq!(stmt.params[0], Value::String("abc123".to_string()));
    }

    #[test]
    fn test_generate_insert() {
        let mut properties = HashMap::new();
        properties.insert(
            "title".to_string(),
            PropertyValue::String("Hello".to_string()),
        );

        let node = Node {
            id: "node123".to_string(),
            name: "post1".to_string(),
            path: "/articles/post1".to_string(),
            node_type: "Article".to_string(),
            properties,
            ..Default::default()
        };

        let stmt = generate_insert("content", &node);
        assert!(stmt.sql.contains("INSERT INTO content"));
        assert!(stmt.sql.contains("VALUES ($1, $2, $3, $4::JSONB)"));
        assert_eq!(stmt.params.len(), 4);
    }

    #[test]
    fn test_generate_delete_by_path() {
        let stmt = generate_delete_by_path("content", "/articles/post1");
        assert_eq!(stmt.sql, "DELETE FROM content WHERE path = $1");
        assert_eq!(stmt.params[0], Value::String("/articles/post1".to_string()));
    }

    #[test]
    fn test_generate_move() {
        let stmt = generate_move("content", "/old/path", "/new/path");
        assert_eq!(stmt.sql, "MOVE content SET path = $1 WHERE path = $2");
        assert_eq!(stmt.params[0], Value::String("/new/path".to_string()));
        assert_eq!(stmt.params[1], Value::String("/old/path".to_string()));
    }

    #[test]
    fn test_escape_identifier() {
        assert_eq!(escape_identifier("content"), "content");
        assert_eq!(escape_identifier("my_workspace"), "my_workspace");
        assert_eq!(escape_identifier("test-ws"), "test-ws");
    }

    #[test]
    fn test_build_nested_property_json() {
        let value = PropertyValue::String("John".to_string());
        let result = build_nested_property_json("metadata.author", &value);

        let expected = serde_json::json!({
            "metadata": {
                "author": "John"
            }
        });
        assert_eq!(result, expected);
    }
}
