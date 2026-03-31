// SPDX-License-Identifier: BSL-1.1

//! JSON conversion helpers for SQL query results.
//!
//! Converts Row and PropertyValue types from the query engine
//! into serde_json::Value for HTTP responses.

use raisin_models::nodes::properties::PropertyValue;

/// Convert a Row (from QueryEngine) to a JSON object
pub(super) fn row_to_json(row: &raisin_sql_execution::Row) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    for (col_name, prop_value) in &row.columns {
        map.insert(col_name.clone(), property_value_to_json(prop_value));
    }

    serde_json::Value::Object(map)
}

/// Convert a PropertyValue to JSON Value
fn property_value_to_json(value: &PropertyValue) -> serde_json::Value {
    match value {
        PropertyValue::Null => serde_json::Value::Null,
        PropertyValue::String(s) => serde_json::Value::String(s.clone()),
        PropertyValue::Integer(i) => serde_json::json!(i),
        PropertyValue::Float(f) => serde_json::json!(f),
        PropertyValue::Decimal(d) => serde_json::json!(d.to_string()),
        PropertyValue::Boolean(b) => serde_json::Value::Bool(*b),
        PropertyValue::Date(dt) => serde_json::Value::String(dt.to_rfc3339()),
        PropertyValue::Url(url) => serde_json::Value::String(url.url.clone()),
        PropertyValue::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(property_value_to_json).collect())
        }
        PropertyValue::Object(obj) => {
            let mut map = serde_json::Map::new();
            for (k, v) in obj {
                map.insert(k.clone(), property_value_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        // For complex types, serialize using serde_json
        PropertyValue::Reference(r) => serde_json::to_value(r).unwrap_or(serde_json::Value::Null),
        PropertyValue::Resource(res) => {
            serde_json::to_value(res).unwrap_or(serde_json::Value::Null)
        }
        PropertyValue::Composite(bc) => serde_json::to_value(bc).unwrap_or(serde_json::Value::Null),
        PropertyValue::Element(b) => serde_json::to_value(b).unwrap_or(serde_json::Value::Null),
        PropertyValue::Vector(v) => {
            // Convert vector to JSON array of numbers
            serde_json::Value::Array(v.iter().map(|f| serde_json::json!(f)).collect())
        }
        PropertyValue::Geometry(geo) => {
            // Serialize Geometry as GeoJSON
            serde_json::to_value(geo).unwrap_or(serde_json::Value::Null)
        }
    }
}
