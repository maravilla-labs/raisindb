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
/// Uses CHILD_OF for proper child matching (direct children only).
///
/// ```sql
/// SELECT * FROM workspace WHERE CHILD_OF($1) LIMIT N
/// ```
pub fn generate_select_children(
    workspace: &str,
    parent_path: &str,
    limit: Option<u32>,
) -> SqlStatement {
    // CHILD_OF matches on PATH SEGMENTS, so it encodes both "under this parent"
    // and "exactly one level deeper" in one predicate.
    //
    // Do NOT reach for `PATH_STARTS_WITH(path, $1) AND DEPTH(path) = DEPTH($1) + 1`
    // here: PATH_STARTS_WITH is a raw string prefix, so a sibling whose name
    // merely EXTENDS the parent's leaks in at the same depth — asking for the
    // children of `/university` also returned every child of `/university-de`.
    // (The other PATH_STARTS_WITH call sites avoid this by passing a trailing
    // slash, e.g. `$1 || '/'` in generate_delete_subtree; this one could not,
    // because appending it would also shift DEPTH($1).)
    let sql = if let Some(max) = limit {
        format!(
            "SELECT * FROM {} WHERE CHILD_OF($1) LIMIT {}",
            escape_identifier(workspace),
            max
        )
    } else {
        format!(
            "SELECT * FROM {} WHERE CHILD_OF($1)",
            escape_identifier(workspace)
        )
    };

    SqlStatement::new(sql, vec![Value::String(parent_path.to_string())])
}

/// Generate SELECT for `raisin.nodes.query(workspace, query)`.
///
/// The `query` argument is a small declarative FILTER OBJECT, not SQL — a
/// function that needs the full language calls `raisin.sql.query()`. Every
/// user-supplied value becomes a bound parameter; nothing is interpolated into
/// the statement except the workspace identifier and the sort column, which is
/// checked against a fixed list.
///
/// Recognised keys (all optional, `camelCase` or `snake_case`):
///
/// | key | effect |
/// |---|---|
/// | `path` | `path = $n` |
/// | `id` | `id = $n` |
/// | `nodeType` | `node_type = $n` |
/// | `childOf` | `CHILD_OF($n)` — direct children only |
/// | `descendantOf` | `DESCENDANT_OF($n)` — the whole subtree |
/// | `properties` | one `properties->>'k'::String = $n` per entry |
/// | `orderBy` + `order` | `ORDER BY <col> ASC\|DESC` |
/// | `limit`, `offset` | `LIMIT` / `OFFSET` |
///
/// The property predicate uses the `::String` KEY-CAST form deliberately: it
/// evaluates as a verbatim row-level filter, so it stays correct alongside a
/// `path` or `node_type` equality and on a workspace with compound indexes,
/// where the bare form can be routed to an index that is unbuilt or stale and
/// return zero rows. `->>` yields text, so a non-string value is compared as
/// its JSON text (`true` -> `'true'`, `0` -> `'0'`).
///
/// # Errors
///
/// [`raisin_error::Error::Validation`] when `query` is not an object, when
/// `orderBy` names a column that is not sortable, or when a property filter
/// value is a container.
pub fn generate_node_query(workspace: &str, query: &Value) -> raisin_error::Result<SqlStatement> {
    let obj = query.as_object().ok_or_else(|| {
        raisin_error::Error::Validation(
            "raisin.nodes.query expects a filter object, e.g. { nodeType: 'article', limit: 10 }"
                .to_string(),
        )
    })?;

    let get = |a: &str, b: &str| obj.get(a).or_else(|| obj.get(b));

    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    let mut push_str =
        |clauses: &mut Vec<String>, params: &mut Vec<Value>, template: &str, value: &Value| {
            if let Some(text) = value.as_str() {
                params.push(Value::String(text.to_string()));
                clauses.push(template.replace("$n", &format!("${}", params.len())));
            }
        };

    if let Some(v) = obj.get("path") {
        push_str(&mut clauses, &mut params, "path = $n", v);
    }
    if let Some(v) = obj.get("id") {
        push_str(&mut clauses, &mut params, "id = $n", v);
    }
    if let Some(v) = get("nodeType", "node_type") {
        push_str(&mut clauses, &mut params, "node_type = $n", v);
    }
    if let Some(v) = get("childOf", "child_of") {
        push_str(&mut clauses, &mut params, "CHILD_OF($n)", v);
    }
    if let Some(v) = get("descendantOf", "descendant_of") {
        push_str(&mut clauses, &mut params, "DESCENDANT_OF($n)", v);
    }

    if let Some(props) = obj.get("properties") {
        let props = props.as_object().ok_or_else(|| {
            raisin_error::Error::Validation(
                "raisin.nodes.query: `properties` must be an object of key/value filters"
                    .to_string(),
            )
        })?;
        // BTreeMap ordering: serde_json is built with `preserve_order` here, so
        // iterate as given and keep the statement stable for one input.
        for (key, value) in props {
            if key.contains('\'') {
                return Err(raisin_error::Error::Validation(format!(
                    "raisin.nodes.query: property name '{key}' may not contain a quote"
                )));
            }
            let as_text = match value {
                Value::String(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                Value::Null => {
                    return Err(raisin_error::Error::Validation(format!(
                        "raisin.nodes.query: property filter '{key}' is null; \
                         use raisin.sql.query() to test for absence"
                    )))
                }
                _ => {
                    return Err(raisin_error::Error::Validation(format!(
                        "raisin.nodes.query: property filter '{key}' must be a string, \
                         number or boolean"
                    )))
                }
            };
            params.push(Value::String(as_text));
            clauses.push(format!(
                "properties->>'{}'::String = ${}",
                key,
                params.len()
            ));
        }
    }

    let mut sql = format!("SELECT * FROM {}", escape_identifier(workspace));
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }

    if let Some(order_by) = get("orderBy", "order_by").and_then(Value::as_str) {
        // An allow-list, because this is the ONE part of the statement that is
        // interpolated rather than bound. `__order` and `__tree_order` are the
        // editorial-ordering columns; they are opaque sortable text and are the
        // only correct way to reproduce drag-and-drop order (`path` sorts
        // siblings alphabetically instead).
        const SORTABLE: [&str; 8] = [
            "path",
            "name",
            "node_type",
            "created_at",
            "updated_at",
            "revision",
            "__order",
            "__tree_order",
        ];
        if !SORTABLE.contains(&order_by) {
            return Err(raisin_error::Error::Validation(format!(
                "raisin.nodes.query: cannot order by '{order_by}'; \
                 allowed: {}. Use raisin.sql.query() for anything else.",
                SORTABLE.join(", ")
            )));
        }
        let descending = get("order", "direction")
            .and_then(Value::as_str)
            .is_some_and(|d| d.eq_ignore_ascii_case("desc"));
        sql.push_str(&format!(
            " ORDER BY {} {}",
            order_by,
            if descending { "DESC" } else { "ASC" }
        ));
    }

    if let Some(limit) = obj.get("limit").and_then(Value::as_u64) {
        sql.push_str(&format!(" LIMIT {}", limit));
    }
    if let Some(offset) = obj.get("offset").and_then(Value::as_u64) {
        sql.push_str(&format!(" OFFSET {}", offset));
    }

    Ok(SqlStatement::new(sql, params))
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
    // ------------------------------------------------------------------
    // raisin.nodes.query
    // ------------------------------------------------------------------

    mod node_query {
        use super::super::generate_node_query;
        use serde_json::json;

        #[test]
        fn an_empty_filter_selects_the_whole_workspace() {
            let stmt = generate_node_query("content", &json!({})).unwrap();
            assert_eq!(stmt.sql, "SELECT * FROM content");
            assert!(stmt.params.is_empty());
        }

        #[test]
        fn every_value_is_bound_never_interpolated() {
            // Regression guard for the shape of the fix: this used to be a stub
            // returning "Node query not yet implemented", and the obvious
            // re-implementation formats values into the string.
            let stmt = generate_node_query(
                "content",
                &json!({ "nodeType": "article'; DROP TABLE x --" }),
            )
            .unwrap();
            assert_eq!(stmt.sql, "SELECT * FROM content WHERE node_type = $1");
            assert_eq!(stmt.params, vec![json!("article'; DROP TABLE x --")]);
        }

        #[test]
        fn predicates_compose_in_a_stable_order() {
            let stmt = generate_node_query(
                "content",
                &json!({
                    "nodeType": "article",
                    "descendantOf": "/blog",
                    "limit": 20,
                    "offset": 40,
                }),
            )
            .unwrap();

            assert_eq!(
                stmt.sql,
                "SELECT * FROM content WHERE node_type = $1 AND DESCENDANT_OF($2) \
                 LIMIT 20 OFFSET 40"
                    .replace("\\\n                 ", " ")
            );
            assert_eq!(stmt.params, vec![json!("article"), json!("/blog")]);
        }

        #[test]
        fn snake_case_keys_are_accepted_too() {
            let camel =
                generate_node_query("ws", &json!({ "nodeType": "a", "childOf": "/x" })).unwrap();
            let snake =
                generate_node_query("ws", &json!({ "node_type": "a", "child_of": "/x" })).unwrap();
            assert_eq!(camel.sql, snake.sql);
            assert_eq!(camel.params, snake.params);
        }

        #[test]
        fn a_property_filter_uses_the_string_cast_form() {
            // The `::String` KEY-CAST form evaluates as a verbatim row filter,
            // so it stays correct beside a node_type equality and on a
            // workspace whose compound index is unbuilt. The bare form can be
            // routed to that index and return zero rows.
            let stmt = generate_node_query(
                "content",
                &json!({ "nodeType": "article", "properties": { "slug": "hello" } }),
            )
            .unwrap();

            assert!(stmt.sql.contains("properties->>'slug'::String = $2"));
            assert_eq!(stmt.params, vec![json!("article"), json!("hello")]);
        }

        #[test]
        fn a_non_string_property_is_compared_as_its_json_text() {
            // `->>` yields text, so `seq: 0` must become '0' and not 0.
            let stmt =
                generate_node_query("ws", &json!({ "properties": { "seq": 0, "live": true } }))
                    .unwrap();
            assert!(stmt.params.contains(&json!("0")));
            assert!(stmt.params.contains(&json!("true")));
        }

        #[test]
        fn ordering_is_restricted_to_a_known_column() {
            let stmt = generate_node_query("ws", &json!({ "orderBy": "__order", "order": "desc" }))
                .unwrap();
            assert_eq!(stmt.sql, "SELECT * FROM ws ORDER BY __order DESC");

            // The sort column is the one part that is interpolated, so an
            // unknown name is an error rather than a formatted string.
            let err = generate_node_query("ws", &json!({ "orderBy": "1; DROP TABLE x" }))
                .unwrap_err()
                .to_string();
            assert!(err.contains("cannot order by"), "unexpected: {err}");
        }

        #[test]
        fn a_non_object_filter_is_refused_with_an_example() {
            let err = generate_node_query("ws", &json!("SELECT * FROM ws"))
                .unwrap_err()
                .to_string();
            assert!(err.contains("filter object"), "unexpected: {err}");
        }

        #[test]
        fn a_property_name_carrying_a_quote_is_refused() {
            // The property NAME is interpolated into the `->>` operand, so it
            // cannot be allowed to close the literal.
            let err = generate_node_query("ws", &json!({ "properties": { "a'b": "x" } }))
                .unwrap_err()
                .to_string();
            assert!(err.contains("quote"), "unexpected: {err}");
        }

        #[test]
        fn a_container_or_null_property_value_is_refused() {
            assert!(generate_node_query("ws", &json!({ "properties": { "a": [1] } })).is_err());
            assert!(generate_node_query("ws", &json!({ "properties": { "a": null } })).is_err());
        }
    }

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
