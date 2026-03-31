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

//! Helper functions for transaction operations.
//!
//! Provides SQL parameter substitution, JSON/PropertyValue conversion,
//! and node data parsing utilities used by lifecycle and operation callbacks.

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_sql_execution::Row;
use serde_json::Value;

/// Substitute $1, $2, etc. with actual parameter values.
pub(crate) fn substitute_params(sql: &str, params: &[Value]) -> String {
    let mut result = sql.to_string();
    for (i, param) in params.iter().enumerate() {
        let placeholder = format!("${}", i + 1);
        let value_str = match param {
            Value::String(s) => format!("'{}'", s.replace('\'', "''")),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "NULL".to_string(),
            Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(json_value_to_sql).collect();
                format!("ARRAY[{}]", items.join(", "))
            }
            Value::Object(_) => {
                format!("'{}'", param.to_string().replace('\'', "''"))
            }
        };
        result = result.replace(&placeholder, &value_str);
    }
    result
}

fn json_value_to_sql(val: &Value) -> String {
    match val {
        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "NULL".to_string(),
        _ => format!("'{}'", val.to_string().replace('\'', "''")),
    }
}

/// Convert a SQL Row to a JSON object, stripping workspace prefixes from column names.
///
/// SQL results have column names like "workspace.id", "workspace.path", etc.
/// This function strips the prefix to produce clean JSON keys like "id", "path".
pub(crate) fn row_to_json_object(row: Row) -> serde_json::Map<String, Value> {
    let mut obj = serde_json::Map::new();
    for (key, value) in row.columns {
        // Strip workspace prefix from column names (e.g., "workspace.id" -> "id")
        let clean_key = if let Some(pos) = key.find('.') {
            key[pos + 1..].to_string()
        } else {
            key
        };
        obj.insert(clean_key, property_value_to_json(value));
    }
    obj
}

/// Convert PropertyValue to JSON Value
fn property_value_to_json(pv: PropertyValue) -> Value {
    match pv {
        PropertyValue::Null => Value::Null,
        PropertyValue::Boolean(b) => Value::Bool(b),
        PropertyValue::Integer(i) => Value::Number(i.into()),
        PropertyValue::Float(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        PropertyValue::Date(d) => Value::String(d.to_rfc3339()),
        PropertyValue::Decimal(d) => Value::String(d.to_string()),
        PropertyValue::String(s) => Value::String(s),
        PropertyValue::Reference(r) => serde_json::to_value(r).unwrap_or(Value::Null),
        PropertyValue::Url(u) => serde_json::to_value(u).unwrap_or(Value::Null),
        PropertyValue::Resource(r) => serde_json::to_value(r).unwrap_or(Value::Null),
        PropertyValue::Composite(c) => serde_json::to_value(c).unwrap_or(Value::Null),
        PropertyValue::Element(e) => serde_json::to_value(e).unwrap_or(Value::Null),
        PropertyValue::Vector(v) => serde_json::to_value(v).unwrap_or(Value::Null),
        PropertyValue::Geometry(g) => serde_json::to_value(g).unwrap_or(Value::Null),
        PropertyValue::Array(arr) => {
            Value::Array(arr.into_iter().map(property_value_to_json).collect())
        }
        PropertyValue::Object(map) => {
            let obj: serde_json::Map<String, Value> = map
                .into_iter()
                .map(|(k, v)| (k, property_value_to_json(v)))
                .collect();
            Value::Object(obj)
        }
    }
}

/// Parse node creation data from JSON (parent path + data).
pub(crate) fn parse_node_create_data(parent_path: &str, data: Value) -> raisin_error::Result<Node> {
    let name = data
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| raisin_error::Error::Validation("Missing 'name' field".to_string()))?;

    let node_type = data
        .get("node_type")
        .or_else(|| data.get("type"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            raisin_error::Error::Validation("Missing 'node_type' or 'type' field".to_string())
        })?;

    let path = if parent_path == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", parent_path, name)
    };

    let mut node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        path,
        node_type: node_type.to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Default::default()
    };

    if let Some(props) = data.get("properties").and_then(|v| v.as_object()) {
        for (key, value) in props {
            node.properties
                .insert(key.clone(), json_to_property_value(value.clone())?);
        }
    }

    Ok(node)
}

/// Parse full node data from JSON (path required in data).
pub(crate) fn parse_node_full_data(data: Value) -> raisin_error::Result<Node> {
    let path = data
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| raisin_error::Error::Validation("Missing 'path' field".to_string()))?;

    let name = data
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or("").to_string());

    let node_type = data
        .get("node_type")
        .or_else(|| data.get("type"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            raisin_error::Error::Validation("Missing 'node_type' or 'type' field".to_string())
        })?;

    let id = data
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let mut node = Node {
        id,
        name,
        path: path.to_string(),
        node_type: node_type.to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Default::default()
    };

    if let Some(props) = data.get("properties").and_then(|v| v.as_object()) {
        for (key, value) in props {
            node.properties
                .insert(key.clone(), json_to_property_value(value.clone())?);
        }
    }

    Ok(node)
}

/// Apply updates from JSON data to an existing node.
pub(crate) fn apply_node_updates(node: &mut Node, data: Value) -> raisin_error::Result<()> {
    if let Some(props) = data.get("properties").and_then(|v| v.as_object()) {
        for (key, value) in props {
            node.properties
                .insert(key.clone(), json_to_property_value(value.clone())?);
        }
    }
    node.updated_at = Some(chrono::Utc::now());
    Ok(())
}

/// Convert a JSON Value to a PropertyValue.
pub(crate) fn json_to_property_value(value: Value) -> raisin_error::Result<PropertyValue> {
    match value {
        Value::Null => Ok(PropertyValue::Null),
        Value::Bool(b) => Ok(PropertyValue::Boolean(b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(PropertyValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(PropertyValue::Float(f))
            } else {
                Err(raisin_error::Error::Validation(
                    "Invalid number".to_string(),
                ))
            }
        }
        Value::String(s) => Ok(PropertyValue::String(s)),
        Value::Array(arr) => {
            let items: raisin_error::Result<Vec<_>> =
                arr.into_iter().map(json_to_property_value).collect();
            Ok(PropertyValue::Array(items?))
        }
        Value::Object(obj) => {
            let mut map = std::collections::HashMap::new();
            for (k, v) in obj {
                map.insert(k, json_to_property_value(v)?);
            }
            Ok(PropertyValue::Object(map))
        }
    }
}
