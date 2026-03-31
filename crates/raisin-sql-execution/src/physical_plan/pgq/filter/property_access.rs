//! Property access evaluation for node and relationship variables.

use std::sync::Arc;

use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;

use super::Result;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::pgq::context::PgqContext;
use crate::physical_plan::pgq::types::{SqlValue, VariableBinding};

/// Evaluate property access: node.property or rel.property
pub(super) async fn evaluate_property_access<S: Storage>(
    variable: &str,
    properties: &[String],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    // Check if it is a node variable
    if let Some(node) = binding.get_node_mut(variable) {
        // If no property specified, return the whole node as JSON
        if properties.is_empty() {
            // Ensure node data is loaded for full export
            node.ensure_loaded(storage, context).await?;

            // Build JSON representation of the node
            let mut node_json = serde_json::Map::new();
            node_json.insert("id".into(), serde_json::json!(node.id.clone()));
            node_json.insert(
                "workspace".into(),
                serde_json::json!(node.workspace.clone()),
            );
            node_json.insert(
                "node_type".into(),
                serde_json::json!(node.node_type.clone()),
            );

            if let Some(path) = node.path() {
                node_json.insert("path".into(), serde_json::json!(path));
            }
            if let Some(name) = node.name() {
                node_json.insert("name".into(), serde_json::json!(name));
            }
            if let Some(props) = node.properties() {
                let props_json: serde_json::Map<String, serde_json::Value> = props
                    .iter()
                    .map(|(k, v)| (k.clone(), property_value_to_json(v)))
                    .collect();
                node_json.insert("properties".into(), serde_json::Value::Object(props_json));
            }

            return Ok(SqlValue::Json(serde_json::Value::Object(node_json)));
        }

        let prop = &properties[0];

        // System fields do not require loading full node data
        match prop.as_str() {
            "id" => return Ok(SqlValue::String(node.id.clone())),
            "workspace" => return Ok(SqlValue::String(node.workspace.clone())),
            "node_type" => return Ok(SqlValue::String(node.node_type.clone())),
            _ => {}
        }

        // For other properties, ensure node data is loaded
        node.ensure_loaded(storage, context).await?;

        match prop.as_str() {
            "path" => Ok(SqlValue::String(
                node.path().unwrap_or_default().to_string(),
            )),
            "name" => Ok(SqlValue::String(
                node.name().unwrap_or_default().to_string(),
            )),
            "created_at" => match node.created_at() {
                Some(dt) => Ok(SqlValue::String(dt.to_rfc3339())),
                None => Ok(SqlValue::Null),
            },
            "updated_at" => match node.updated_at() {
                Some(dt) => Ok(SqlValue::String(dt.to_rfc3339())),
                None => Ok(SqlValue::Null),
            },
            "properties" => {
                // Handle node.properties (entire object) or node.properties.foo (nested access)
                if properties.len() == 1 {
                    // Just "properties" - return entire properties object as JSONB
                    match node.properties() {
                        Some(props) => {
                            let json_map: serde_json::Map<String, serde_json::Value> = props
                                .iter()
                                .map(|(k, v)| (k.clone(), property_value_to_json(v)))
                                .collect();
                            Ok(SqlValue::Json(serde_json::Value::Object(json_map)))
                        }
                        None => Ok(SqlValue::Null),
                    }
                } else {
                    // "properties.foo.bar..." - navigate into properties
                    // Skip "properties" prefix and look up the rest
                    let nested_path = &properties[1..];
                    let mut current_value: Option<&PropertyValue> =
                        node.get_property(&nested_path[0]);

                    // Navigate deeper if there are more path segments
                    for key in nested_path.iter().skip(1) {
                        current_value = match current_value {
                            Some(PropertyValue::Object(map)) => map.get(key),
                            _ => None,
                        };
                    }

                    match current_value {
                        Some(v) => property_value_to_sql_value(v),
                        None => Ok(SqlValue::Null),
                    }
                }
            }
            _ => {
                // Look in properties JSON for direct property access (e.g., node.title)
                match node.get_property(prop) {
                    Some(v) => property_value_to_sql_value(v),
                    None => Ok(SqlValue::Null),
                }
            }
        }
    }
    // Check if it is a relationship variable
    else if let Some(rel) = binding.get_relation(variable) {
        // If no property specified, return the relationship as JSON
        if properties.is_empty() {
            let mut rel_json = serde_json::Map::new();
            rel_json.insert("type".into(), serde_json::json!(rel.relation_type.clone()));
            if let Some(w) = rel.weight {
                rel_json.insert("weight".into(), serde_json::json!(w));
            }
            return Ok(SqlValue::Json(serde_json::Value::Object(rel_json)));
        }

        let prop = &properties[0];
        match prop.as_str() {
            "type" | "relation_type" => Ok(SqlValue::String(rel.relation_type.clone())),
            "weight" => Ok(rel.weight.into()),
            _ => Ok(SqlValue::Null), // Relations do not have other properties yet
        }
    } else {
        Err(ExecutionError::Validation(format!(
            "Unknown variable: {}",
            variable
        )))
    }
}

/// Convert PropertyValue to SqlValue
pub(super) fn property_value_to_sql_value(v: &PropertyValue) -> Result<SqlValue> {
    Ok(match v {
        PropertyValue::Null => SqlValue::Null,
        PropertyValue::Boolean(b) => SqlValue::Boolean(*b),
        PropertyValue::Integer(i) => SqlValue::Integer(*i),
        PropertyValue::Float(f) => SqlValue::Float(*f),
        PropertyValue::String(s) => SqlValue::String(s.clone()),
        PropertyValue::Array(arr) => {
            let converted: Vec<SqlValue> = arr
                .iter()
                .filter_map(|v| property_value_to_sql_value(v).ok())
                .collect();
            SqlValue::Array(converted)
        }
        PropertyValue::Object(map) => {
            // Convert to JSON for complex objects
            let json_map: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), property_value_to_json(v)))
                .collect();
            SqlValue::Json(serde_json::Value::Object(json_map))
        }
        _ => SqlValue::Null, // Handle other property types as null
    })
}

/// Convert PropertyValue to serde_json::Value
pub(super) fn property_value_to_json(v: &PropertyValue) -> serde_json::Value {
    match v {
        PropertyValue::Null => serde_json::Value::Null,
        PropertyValue::Boolean(b) => serde_json::Value::Bool(*b),
        PropertyValue::Integer(i) => serde_json::Value::Number((*i).into()),
        PropertyValue::Float(f) => serde_json::json!(*f),
        PropertyValue::String(s) => serde_json::Value::String(s.clone()),
        PropertyValue::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(property_value_to_json).collect())
        }
        PropertyValue::Object(map) => {
            let json_map: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), property_value_to_json(v)))
                .collect();
            serde_json::Value::Object(json_map)
        }
        _ => serde_json::Value::Null,
    }
}
