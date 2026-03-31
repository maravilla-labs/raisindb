//! Column utility functions for batch projection
//!
//! Literal broadcasting, PropertyValue text extraction, JSON-to-PropertyValue
//! conversion, and column type inference from PropertyValue vectors.

use std::collections::HashMap;

use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::Literal;

use crate::physical_plan::batch::ColumnArray;
use crate::physical_plan::executor::ExecutionError;

/// Extract text value from PropertyValue for JSON ->> operator
///
/// This handles the conversion from PropertyValue to String for the ->> operator.
pub(crate) fn extract_text_from_property_value(pv: &PropertyValue) -> Option<String> {
    match pv {
        PropertyValue::String(s) => Some(s.clone()),
        PropertyValue::Integer(n) => Some(n.to_string()),
        PropertyValue::Float(n) => Some(n.to_string()),
        PropertyValue::Boolean(b) => Some(b.to_string()),
        PropertyValue::Date(dt) => Some(dt.to_string()),
        // For complex types, return JSON representation
        PropertyValue::Object(obj) => serde_json::to_string(obj).ok(),
        PropertyValue::Array(arr) => serde_json::to_string(arr).ok(),
        PropertyValue::Vector(vec) => serde_json::to_string(vec).ok(),
        _ => None,
    }
}

/// Broadcast a literal value to all rows in the batch
///
/// Creates a column filled with the same literal value.
/// This is the public API -- delegates to `broadcast_literal_impl`.
pub(crate) fn broadcast_literal(lit: &Literal, num_rows: usize) -> ColumnArray {
    broadcast_literal_impl(lit, num_rows)
}

/// Internal implementation for broadcasting a literal to a column
pub(super) fn broadcast_literal_impl(lit: &Literal, num_rows: usize) -> ColumnArray {
    match lit {
        Literal::Null => ColumnArray::String(vec![None; num_rows]),
        Literal::Boolean(b) => ColumnArray::Boolean(vec![Some(*b); num_rows]),
        Literal::Int(i) => ColumnArray::Integer(vec![Some(*i as i64); num_rows]),
        Literal::BigInt(i) => ColumnArray::Integer(vec![Some(*i); num_rows]),
        Literal::Double(d) => ColumnArray::Float(vec![Some(*d); num_rows]),
        Literal::Text(s) => ColumnArray::String(vec![Some(s.clone()); num_rows]),
        Literal::Uuid(s) => ColumnArray::String(vec![Some(s.clone()); num_rows]),
        Literal::Path(s) => ColumnArray::String(vec![Some(s.clone()); num_rows]),
        Literal::JsonB(j) => {
            // Convert JSON to HashMap for Object column
            let obj = match j {
                serde_json::Value::Object(map) => {
                    let mut result = HashMap::new();
                    for (k, v) in map {
                        if let Ok(pv) = json_value_to_property_value(v) {
                            result.insert(k.clone(), pv);
                        }
                    }
                    Some(result)
                }
                _ => None,
            };
            ColumnArray::Object(vec![obj; num_rows])
        }
        Literal::Vector(v) => ColumnArray::Vector(vec![Some(v.clone()); num_rows]),
        Literal::Geometry(geojson) => {
            // Convert GeoJSON to string representation
            ColumnArray::String(vec![Some(geojson.to_string()); num_rows])
        }
        Literal::Timestamp(dt) => {
            // Convert timestamp to RFC3339 string
            ColumnArray::String(vec![Some(dt.to_rfc3339()); num_rows])
        }
        Literal::Interval(_) => {
            // Intervals cannot be broadcast directly - they are used in date arithmetic
            ColumnArray::String(vec![None; num_rows])
        }
        Literal::Parameter(_) => {
            // Parameters are unbound placeholders - cannot be broadcast
            ColumnArray::String(vec![None; num_rows])
        }
    }
}

/// Convert serde_json::Value to PropertyValue
pub(super) fn json_value_to_property_value(
    value: &serde_json::Value,
) -> Result<PropertyValue, Error> {
    match value {
        serde_json::Value::Null => Err(Error::Validation("Cannot convert null".to_string())),
        serde_json::Value::Bool(b) => Ok(PropertyValue::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(PropertyValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(PropertyValue::Float(f))
            } else {
                Err(Error::Validation("Invalid number".to_string()))
            }
        }
        serde_json::Value::String(s) => Ok(PropertyValue::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let mut result = Vec::new();
            for item in arr {
                result.push(json_value_to_property_value(item)?);
            }
            Ok(PropertyValue::Array(result))
        }
        serde_json::Value::Object(map) => {
            let mut result = HashMap::new();
            for (k, v) in map {
                result.insert(k.clone(), json_value_to_property_value(v)?);
            }
            Ok(PropertyValue::Object(result))
        }
    }
}

/// Convert Vec<Option<PropertyValue>> to ColumnArray
///
/// This inspects the values to determine the appropriate column type.
pub(crate) fn property_values_to_column_array(
    values: Vec<Option<PropertyValue>>,
) -> Result<ColumnArray, ExecutionError> {
    // Determine column type from first non-null value
    let sample = values.iter().find_map(|v| v.as_ref());

    match sample {
        Some(PropertyValue::String(_)) => {
            let result: Vec<Option<String>> = values
                .into_iter()
                .map(|v| {
                    v.and_then(|pv| match pv {
                        PropertyValue::String(s) => Some(s),
                        _ => None,
                    })
                })
                .collect();
            Ok(ColumnArray::String(result))
        }
        Some(PropertyValue::Integer(_)) => {
            let result: Vec<Option<i64>> = values
                .into_iter()
                .map(|v| {
                    v.and_then(|pv| match pv {
                        PropertyValue::Integer(n) => Some(n),
                        _ => None,
                    })
                })
                .collect();
            Ok(ColumnArray::Integer(result))
        }
        Some(PropertyValue::Float(_)) => {
            let result: Vec<Option<f64>> = values
                .into_iter()
                .map(|v| {
                    v.and_then(|pv| match pv {
                        PropertyValue::Float(n) => Some(n),
                        _ => None,
                    })
                })
                .collect();
            Ok(ColumnArray::Float(result))
        }
        Some(PropertyValue::Boolean(_)) => {
            let result: Vec<Option<bool>> = values
                .into_iter()
                .map(|v| {
                    v.and_then(|pv| match pv {
                        PropertyValue::Boolean(b) => Some(b),
                        _ => None,
                    })
                })
                .collect();
            Ok(ColumnArray::Boolean(result))
        }
        Some(PropertyValue::Object(_)) => {
            let result: Vec<Option<HashMap<String, PropertyValue>>> = values
                .into_iter()
                .map(|v| {
                    v.and_then(|pv| match pv {
                        PropertyValue::Object(obj) => Some(obj),
                        _ => None,
                    })
                })
                .collect();
            Ok(ColumnArray::Object(result))
        }
        // Default to string column for mixed types
        _ => Ok(ColumnArray::String(vec![None; values.len()])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broadcast_literal_string() {
        let lit = Literal::Text("hello".to_string());
        let col = broadcast_literal(&lit, 3);

        if let ColumnArray::String(values) = col {
            assert_eq!(values.len(), 3);
            assert_eq!(values[0], Some("hello".to_string()));
            assert_eq!(values[1], Some("hello".to_string()));
            assert_eq!(values[2], Some("hello".to_string()));
        } else {
            panic!("Expected String column");
        }
    }

    #[test]
    fn test_broadcast_literal_number() {
        let lit = Literal::Double(42.5);
        let col = broadcast_literal(&lit, 2);

        if let ColumnArray::Float(values) = col {
            assert_eq!(values.len(), 2);
            assert_eq!(values[0], Some(42.5));
            assert_eq!(values[1], Some(42.5));
        } else {
            panic!("Expected Float column");
        }
    }

    #[test]
    fn test_extract_text_from_property_value() {
        assert_eq!(
            extract_text_from_property_value(&PropertyValue::String("test".to_string())),
            Some("test".to_string())
        );
        assert_eq!(
            extract_text_from_property_value(&PropertyValue::Integer(42)),
            Some("42".to_string())
        );
        assert_eq!(
            extract_text_from_property_value(&PropertyValue::Float(42.5)),
            Some("42.5".to_string())
        );
        assert_eq!(
            extract_text_from_property_value(&PropertyValue::Boolean(true)),
            Some("true".to_string())
        );
    }
}
