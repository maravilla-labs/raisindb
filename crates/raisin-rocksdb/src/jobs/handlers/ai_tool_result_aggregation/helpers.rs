//! Standalone helper functions for PropertyValue/JSON conversion.

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use std::collections::HashMap;

/// Get an integer property from a node
pub(super) fn get_int_property(node: &Node, key: &str) -> Result<i64> {
    node.properties
        .get(key)
        .and_then(|v| match v {
            PropertyValue::Integer(i) => Some(*i),
            _ => None,
        })
        .ok_or_else(|| Error::Validation(format!("Missing or invalid {} property", key)))
}

/// Convert PropertyValue to JSON Value
pub(super) fn property_value_to_json(pv: &PropertyValue) -> serde_json::Value {
    match pv {
        PropertyValue::Null => serde_json::Value::Null,
        PropertyValue::Boolean(b) => serde_json::Value::Bool(*b),
        PropertyValue::Integer(i) => serde_json::json!(i),
        PropertyValue::Float(f) => serde_json::json!(f),
        PropertyValue::Decimal(d) => serde_json::json!(d.to_string()),
        PropertyValue::String(s) => serde_json::Value::String(s.clone()),
        PropertyValue::Date(dt) => serde_json::Value::String(dt.to_string()),
        PropertyValue::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(property_value_to_json).collect())
        }
        PropertyValue::Object(obj) => {
            let map: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .map(|(k, v)| (k.clone(), property_value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        PropertyValue::Reference(r) => serde_json::json!({
            "raisin:ref": r.id,
            "raisin:path": r.path,
            "raisin:workspace": r.workspace
        }),
        PropertyValue::Url(u) => serde_json::json!({
            "raisin:url": u.url
        }),
        PropertyValue::Resource(r) => serde_json::to_value(r).unwrap_or(serde_json::Value::Null),
        PropertyValue::Composite(c) => serde_json::to_value(c).unwrap_or(serde_json::Value::Null),
        PropertyValue::Element(e) => serde_json::to_value(e).unwrap_or(serde_json::Value::Null),
        PropertyValue::Vector(v) => serde_json::json!(v),
        PropertyValue::Geometry(g) => serde_json::to_value(g).unwrap_or(serde_json::Value::Null),
    }
}

/// Convert JSON Value to PropertyValue
pub(super) fn json_to_property_value(value: serde_json::Value) -> Result<PropertyValue> {
    match value {
        serde_json::Value::Null => Ok(PropertyValue::Null),
        serde_json::Value::Bool(b) => Ok(PropertyValue::Boolean(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(PropertyValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(PropertyValue::Float(f))
            } else {
                Err(Error::Validation("Invalid number".to_string()))
            }
        }
        serde_json::Value::String(s) => Ok(PropertyValue::String(s)),
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<_>> = arr.into_iter().map(json_to_property_value).collect();
            Ok(PropertyValue::Array(items?))
        }
        serde_json::Value::Object(obj) => {
            let mut map = HashMap::new();
            for (k, v) in obj {
                map.insert(k, json_to_property_value(v)?);
            }
            Ok(PropertyValue::Object(map))
        }
    }
}
