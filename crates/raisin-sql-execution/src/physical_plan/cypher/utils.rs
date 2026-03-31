//! Utility functions for Cypher query execution
//!
//! This module contains helper functions used throughout the Cypher executor,
//! including column name extraction, value hashing, comparison, and node extraction.

use raisin_models::nodes::properties::PropertyValue;

use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Extract column name from return item
///
/// If an alias is provided, it is used directly. Otherwise, a meaningful name
/// is extracted from the expression (e.g., variable names, property names).
#[inline]
pub(crate) fn extract_column_name(item: &raisin_cypher_parser::ReturnItem) -> String {
    use raisin_cypher_parser::Expr;

    if let Some(alias) = &item.alias {
        return alias.clone();
    }

    // Extract meaningful name from expression
    match &item.expr {
        Expr::Variable(name) => name.clone(),
        Expr::Property { expr, property } => {
            if let Expr::Variable(var) = &**expr {
                format!("{}_{}", var, property)
            } else {
                property.clone()
            }
        }
        Expr::FunctionCall { name, .. } => name.to_lowercase(),
        _ => "result".to_string(),
    }
}

/// Compute hash for PropertyValue (for DISTINCT and grouping)
///
/// This function computes a stable hash for any PropertyValue, allowing
/// efficient deduplication and grouping operations.
#[inline]
pub(crate) fn compute_property_value_hash(value: &PropertyValue) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    // Hash based on discriminant and key fields
    match value {
        PropertyValue::String(s) => {
            0u8.hash(&mut hasher);
            s.hash(&mut hasher);
        }
        PropertyValue::Integer(i) => {
            1u8.hash(&mut hasher);
            i.hash(&mut hasher);
        }
        PropertyValue::Float(f) => {
            2u8.hash(&mut hasher);
            f.to_bits().hash(&mut hasher);
        }
        PropertyValue::Boolean(b) => {
            3u8.hash(&mut hasher);
            b.hash(&mut hasher);
        }
        PropertyValue::Date(d) => {
            4u8.hash(&mut hasher);
            d.timestamp().hash(&mut hasher);
            d.timestamp_subsec_nanos().hash(&mut hasher);
        }
        PropertyValue::Url(url) => {
            5u8.hash(&mut hasher);
            url.url.hash(&mut hasher);
        }
        PropertyValue::Reference(r) => {
            6u8.hash(&mut hasher);
            // Hash workspace and id fields
            r.workspace.hash(&mut hasher);
            r.id.hash(&mut hasher);
        }
        PropertyValue::Resource(_) => {
            7u8.hash(&mut hasher);
            // Resource is complex, just hash discriminant
        }
        PropertyValue::Composite(_) => {
            8u8.hash(&mut hasher);
            // Composite is complex, just hash discriminant
        }
        PropertyValue::Element(_) => {
            9u8.hash(&mut hasher);
            // Block is complex, just hash discriminant
        }
        PropertyValue::Array(arr) => {
            10u8.hash(&mut hasher);
            arr.len().hash(&mut hasher);
            // Recursively hash first few elements for better distribution
            for item in arr.iter().take(3) {
                compute_property_value_hash(item).hash(&mut hasher);
            }
        }
        PropertyValue::Object(obj) => {
            11u8.hash(&mut hasher);
            obj.len().hash(&mut hasher);
            // Hash a few keys for distribution
            for key in obj.keys().take(3) {
                key.hash(&mut hasher);
            }
        }
        PropertyValue::Vector(v) => {
            12u8.hash(&mut hasher);
            v.len().hash(&mut hasher);
            // Hash first few elements for distribution
            for val in v.iter().take(3) {
                val.to_bits().hash(&mut hasher);
            }
        }
        PropertyValue::Geometry(geojson) => {
            13u8.hash(&mut hasher);
            // Hash the GeoJSON string representation
            if let Ok(json_str) = serde_json::to_string(geojson) {
                json_str.hash(&mut hasher);
            }
        }
        PropertyValue::Null => {
            14u8.hash(&mut hasher);
        }
        PropertyValue::Decimal(d) => {
            15u8.hash(&mut hasher);
            d.to_string().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Extract number from PropertyValue
///
/// Returns Some(f64) if the value is an Integer or Float, None otherwise.
#[inline]
pub(crate) fn extract_number(value: &PropertyValue) -> Option<f64> {
    match value {
        PropertyValue::Integer(i) => Some(*i as f64),
        PropertyValue::Float(f) => Some(*f),
        _ => None,
    }
}

/// Compare PropertyValues for ordering
///
/// Supports comparison of Integers, Floats, Strings, and Booleans.
/// Returns Equal for incomparable types.
#[inline]
pub(crate) fn compare_property_values(a: &PropertyValue, b: &PropertyValue) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (PropertyValue::Integer(a), PropertyValue::Integer(b)) => a.cmp(b),
        (PropertyValue::Integer(a), PropertyValue::Float(b)) => {
            (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal)
        }
        (PropertyValue::Float(a), PropertyValue::Integer(b)) => {
            a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal)
        }
        (PropertyValue::Float(a), PropertyValue::Float(b)) => {
            a.partial_cmp(b).unwrap_or(Ordering::Equal)
        }
        (PropertyValue::String(a), PropertyValue::String(b)) => a.cmp(b),
        (PropertyValue::Boolean(a), PropertyValue::Boolean(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}
