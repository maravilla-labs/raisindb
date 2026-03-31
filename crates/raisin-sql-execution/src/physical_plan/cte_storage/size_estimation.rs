//! Memory size estimation for CTE spillage decisions
//!
//! Provides conservative memory footprint estimates for rows and property values.
//! These estimates drive the spill-to-disk decision in [`super::MaterializedCTE`].

use super::Row;
use indexmap::IndexMap;
use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

/// Estimate the memory footprint of a row in bytes
///
/// Provides a conservative estimate including:
/// - IndexMap overhead (~48 bytes baseline)
/// - String keys and values
/// - Nested structures (Objects, Arrays, etc.)
pub fn estimate_row_size(row: &Row) -> usize {
    let mut size = std::mem::size_of::<IndexMap<String, PropertyValue>>();

    for (key, value) in &row.columns {
        size += std::mem::size_of::<String>() + key.len();
        size += estimate_property_value_size(value);
    }

    size
}

/// Estimate memory size of a PropertyValue in bytes
///
/// Recursively calculates size for nested structures.
pub fn estimate_property_value_size(value: &PropertyValue) -> usize {
    match value {
        PropertyValue::Null => std::mem::size_of::<()>(),
        PropertyValue::String(s) => std::mem::size_of::<String>() + s.len(),
        PropertyValue::Integer(_) => std::mem::size_of::<i64>(),
        PropertyValue::Float(_) => std::mem::size_of::<f64>(),
        PropertyValue::Decimal(_) => std::mem::size_of::<rust_decimal::Decimal>(),
        PropertyValue::Boolean(_) => std::mem::size_of::<bool>(),
        PropertyValue::Date(_) => std::mem::size_of::<chrono::DateTime<chrono::Utc>>(),
        PropertyValue::Url(u) => std::mem::size_of::<String>() + u.url.len(),
        PropertyValue::Reference(r) => {
            std::mem::size_of::<String>() * 3 + r.id.len() + r.workspace.len() + r.path.len()
        }
        PropertyValue::Resource(res) => {
            let mut size = std::mem::size_of::<String>() + res.uuid.len();
            size += res
                .name
                .as_ref()
                .map(|s| std::mem::size_of::<String>() + s.len())
                .unwrap_or(0);
            size += res
                .mime_type
                .as_ref()
                .map(|s| std::mem::size_of::<String>() + s.len())
                .unwrap_or(0);
            size += res
                .url
                .as_ref()
                .map(|s| std::mem::size_of::<String>() + s.len())
                .unwrap_or(0);
            size += std::mem::size_of::<i64>() * 2;
            if let Some(metadata) = &res.metadata {
                for (k, v) in metadata {
                    size += std::mem::size_of::<String>() + k.len();
                    size += estimate_property_value_size(v);
                }
            }
            size
        }
        PropertyValue::Element(block) => {
            let mut size =
                std::mem::size_of::<String>() * 2 + block.uuid.len() + block.element_type.len();
            for (k, v) in &block.content {
                size += std::mem::size_of::<String>() + k.len();
                size += estimate_property_value_size(v);
            }
            size
        }
        PropertyValue::Composite(container) => {
            let mut size = std::mem::size_of::<String>() + container.uuid.len();
            for block in &container.items {
                size +=
                    std::mem::size_of::<String>() * 2 + block.uuid.len() + block.element_type.len();
                for (k, v) in &block.content {
                    size += std::mem::size_of::<String>() + k.len();
                    size += estimate_property_value_size(v);
                }
            }
            size
        }
        PropertyValue::Array(arr) => {
            let mut size = std::mem::size_of::<Vec<PropertyValue>>();
            for item in arr {
                size += estimate_property_value_size(item);
            }
            size
        }
        PropertyValue::Object(obj) => {
            let mut size = std::mem::size_of::<HashMap<String, PropertyValue>>();
            for (k, v) in obj {
                size += std::mem::size_of::<String>() + k.len();
                size += estimate_property_value_size(v);
            }
            size
        }
        PropertyValue::Vector(v) => {
            std::mem::size_of::<Vec<f32>>() + (v.len() * std::mem::size_of::<f32>())
        }
        PropertyValue::Geometry(geojson) => {
            let size = serde_json::to_string(geojson)
                .map(|s| s.len())
                .unwrap_or(std::mem::size_of::<raisin_models::nodes::properties::GeoJson>());
            std::mem::size_of::<raisin_models::nodes::properties::GeoJson>() + size
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_row_size() {
        let mut row = Row::new();
        row.insert("id".to_string(), PropertyValue::String("123".to_string()));
        row.insert("count".to_string(), PropertyValue::Integer(42));

        let size = estimate_row_size(&row);
        assert!(size > 100);
    }

    #[test]
    fn test_estimate_property_value_size() {
        let value = PropertyValue::String("hello".to_string());
        let size = estimate_property_value_size(&value);
        assert!(size >= 5);

        let value = PropertyValue::Float(3.14);
        let size = estimate_property_value_size(&value);
        assert_eq!(size, std::mem::size_of::<f64>());

        let value = PropertyValue::Boolean(true);
        let size = estimate_property_value_size(&value);
        assert_eq!(size, std::mem::size_of::<bool>());
    }

    #[test]
    fn test_estimate_nested_object_size() {
        let mut inner = HashMap::new();
        inner.insert(
            "nested".to_string(),
            PropertyValue::String("value".to_string()),
        );
        let value = PropertyValue::Object(inner);

        let size = estimate_property_value_size(&value);
        assert!(size > 50);
    }

    #[test]
    fn test_estimate_array_size() {
        let arr = vec![
            PropertyValue::Float(1.0),
            PropertyValue::Float(2.0),
            PropertyValue::Float(3.0),
        ];
        let value = PropertyValue::Array(arr);

        let size = estimate_property_value_size(&value);
        assert!(size >= std::mem::size_of::<f64>() * 3);
    }
}
