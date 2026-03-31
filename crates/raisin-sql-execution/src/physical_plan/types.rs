//! Type conversions between SQL types and PropertyValue
//!
//! This module provides bidirectional conversion between RaisinSQL's DataType
//! and raisin-models PropertyValue. This is critical for:
//! - Reading data from RocksDB (PropertyValue → SQL value)
//! - Writing results (SQL value → PropertyValue)
//! - Type checking and validation

use indexmap::IndexMap;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{DataType, Literal};
use std::collections::HashMap;

/// Convert a PropertyValue to a SQL Literal
///
/// This is used when reading data from RocksDB storage and converting it
/// to SQL values for expression evaluation.
///
/// # Errors
///
/// Returns an error if the PropertyValue type cannot be represented as a Literal.
/// Complex types like Block, Composite, and Resource are not directly convertible.
pub fn from_property_value(value: &PropertyValue) -> Result<Literal, String> {
    match value {
        PropertyValue::Null => Ok(Literal::Null),
        PropertyValue::String(s) => Ok(Literal::Text(s.clone())),
        PropertyValue::Integer(n) => Ok(Literal::BigInt(*n)),
        PropertyValue::Float(n) => Ok(Literal::Double(*n)),
        PropertyValue::Decimal(d) => {
            // Convert decimal to double for SQL operations
            use std::str::FromStr;
            let f = f64::from_str(&d.to_string())
                .map_err(|e| format!("Failed to convert Decimal to f64: {}", e))?;
            Ok(Literal::Double(f))
        }
        PropertyValue::Boolean(b) => Ok(Literal::Boolean(*b)),
        PropertyValue::Date(dt) => {
            // Convert PropertyValue::Date to Literal::Timestamp
            Ok(Literal::Timestamp(**dt))
        }
        PropertyValue::Url(url) => Ok(Literal::Text(url.url.clone())),
        PropertyValue::Array(arr) => {
            // Convert array to JSON for storage
            let json = serde_json::to_value(arr)
                .map_err(|e| format!("Failed to serialize array: {}", e))?;
            Ok(Literal::JsonB(json))
        }
        PropertyValue::Object(obj) => {
            // Convert object to JSON
            let json = serde_json::to_value(obj)
                .map_err(|e| format!("Failed to serialize object: {}", e))?;
            Ok(Literal::JsonB(json))
        }
        PropertyValue::Reference(ref_val) => {
            // Convert reference to JSON representation
            let json = serde_json::to_value(ref_val)
                .map_err(|e| format!("Failed to serialize reference: {}", e))?;
            Ok(Literal::JsonB(json))
        }
        PropertyValue::Resource(resource) => {
            // Convert resource to JSON
            let json = serde_json::to_value(resource)
                .map_err(|e| format!("Failed to serialize resource: {}", e))?;
            Ok(Literal::JsonB(json))
        }
        PropertyValue::Composite(container) => {
            // Convert block container to JSON
            let json = serde_json::to_value(container)
                .map_err(|e| format!("Failed to serialize block container: {}", e))?;
            Ok(Literal::JsonB(json))
        }
        PropertyValue::Element(block) => {
            // Convert block to JSON
            let json = serde_json::to_value(block)
                .map_err(|e| format!("Failed to serialize block: {}", e))?;
            Ok(Literal::JsonB(json))
        }
        PropertyValue::Vector(vec) => {
            // Convert vector directly to Literal::Vector
            Ok(Literal::Vector(vec.clone()))
        }
        PropertyValue::Geometry(geo) => {
            // Convert GeoJson to Literal::Geometry as JSON
            let json = serde_json::to_value(geo)
                .map_err(|e| format!("Failed to serialize geometry: {}", e))?;
            Ok(Literal::Geometry(json))
        }
    }
}

/// Convert a SQL Literal to a PropertyValue
///
/// This is used when writing SQL expression results back to PropertyValue format,
/// for example when creating computed columns or materialized views.
///
/// # Errors
///
/// Returns an error if the Literal type cannot be represented as a PropertyValue.
pub fn to_property_value(literal: &Literal) -> Result<PropertyValue, String> {
    match literal {
        Literal::Null => Ok(PropertyValue::Null),
        Literal::Boolean(b) => Ok(PropertyValue::Boolean(*b)),
        Literal::Int(i) => Ok(PropertyValue::Integer(*i as i64)),
        Literal::BigInt(i) => Ok(PropertyValue::Integer(*i)),
        Literal::Double(f) => Ok(PropertyValue::Float(*f)),
        Literal::Text(s) => Ok(PropertyValue::String(s.clone())),
        Literal::Uuid(u) => Ok(PropertyValue::String(u.clone())),
        Literal::Path(p) => Ok(PropertyValue::String(p.clone())),
        Literal::JsonB(json) => {
            // Try to convert JSON back to PropertyValue
            serde_json::from_value(json.clone())
                .map_err(|e| format!("Failed to deserialize JSON to PropertyValue: {}", e))
        }
        Literal::Vector(vec) => {
            // Convert vector directly to PropertyValue::Vector to preserve type
            // This is critical for CTE materialization where vectors need to remain vectors
            Ok(PropertyValue::Vector(vec.clone()))
        }
        Literal::Timestamp(dt) => {
            // DateTimeTimestamp is a type alias for StorageTimestamp
            Ok(PropertyValue::Date((*dt).into()))
        }
        Literal::Interval(_) => Err("Cannot convert INTERVAL to PropertyValue".to_string()),
        Literal::Parameter(p) => Err(format!(
            "Cannot convert unbound parameter {} to PropertyValue",
            p
        )),
        Literal::Geometry(json) => {
            // Convert JSON back to GeoJson
            serde_json::from_value(json.clone())
                .map(PropertyValue::Geometry)
                .map_err(|e| format!("Failed to deserialize geometry: {}", e))
        }
    }
}

/// Infer the SQL DataType from a PropertyValue
///
/// This is useful for schema inference and validation when reading data.
pub fn infer_data_type(value: &PropertyValue) -> DataType {
    match value {
        PropertyValue::Null => DataType::Text, // Default to Text for null
        PropertyValue::String(_) => DataType::Text,
        PropertyValue::Integer(_) => DataType::BigInt,
        PropertyValue::Float(_) => DataType::Double,
        PropertyValue::Decimal(_) => DataType::Double, // Map Decimal to Double for SQL
        PropertyValue::Boolean(_) => DataType::Boolean,
        PropertyValue::Date(_) => DataType::TimestampTz,
        PropertyValue::Url(_) => DataType::Text,
        PropertyValue::Vector(v) => DataType::Vector(v.len()),
        PropertyValue::Geometry(_) => DataType::Geometry,
        PropertyValue::Array(_) | PropertyValue::Object(_) => DataType::JsonB,
        PropertyValue::Reference(_)
        | PropertyValue::Resource(_)
        | PropertyValue::Composite(_)
        | PropertyValue::Element(_) => DataType::JsonB,
    }
}

/// Check if a PropertyValue can be coerced to the target DataType
///
/// This is used during query planning to validate type compatibility.
pub fn can_coerce(value: &PropertyValue, target_type: &DataType) -> bool {
    let inferred_type = infer_data_type(value);
    inferred_type.can_coerce_to(target_type)
}

/// Convert a PropertyValue to match a target DataType
///
/// This performs runtime type coercion, for example converting a string to a number.
///
/// # Errors
///
/// Returns an error if the conversion is not possible.
pub fn coerce_value(
    value: &PropertyValue,
    target_type: &DataType,
) -> Result<PropertyValue, String> {
    let current_type = infer_data_type(value);

    // If types already match, no conversion needed
    if current_type.can_coerce_to(target_type) && current_type == *target_type.base_type() {
        return Ok(value.clone());
    }

    // Handle specific conversions
    match (value, target_type.base_type()) {
        // String to number
        (PropertyValue::String(s), DataType::Double) => {
            let num: f64 = s
                .parse()
                .map_err(|_| format!("Cannot parse '{}' as number", s))?;
            Ok(PropertyValue::Float(num))
        }
        (PropertyValue::String(s), DataType::Int | DataType::BigInt) => {
            let num: i64 = s
                .parse()
                .map_err(|_| format!("Cannot parse '{}' as integer", s))?;
            Ok(PropertyValue::Integer(num))
        }

        // Integer/Float to string
        (PropertyValue::Integer(n), DataType::Text) => Ok(PropertyValue::String(n.to_string())),
        (PropertyValue::Float(n), DataType::Text) => Ok(PropertyValue::String(n.to_string())),

        // Integer to Float
        (PropertyValue::Integer(n), DataType::Double) => Ok(PropertyValue::Float(*n as f64)),

        // Float to Integer (truncates)
        (PropertyValue::Float(n), DataType::Int | DataType::BigInt) => {
            Ok(PropertyValue::Integer(*n as i64))
        }

        // Bool to string
        (PropertyValue::Boolean(b), DataType::Text) => Ok(PropertyValue::String(b.to_string())),

        // Anything to JSON
        (v, DataType::JsonB) => Ok(v.clone()), // PropertyValue is already JSON-compatible

        _ => Err(format!(
            "Cannot coerce {:?} to {}",
            infer_data_type(value),
            target_type
        )),
    }
}

/// Convert selected PropertyValues to a row-friendly map preserving column order
///
/// This extracts specific columns from a Node's properties for query execution.
pub fn extract_columns(
    properties: &HashMap<String, PropertyValue>,
    columns: &[String],
) -> IndexMap<String, PropertyValue> {
    let mut result = IndexMap::new();
    for col in columns {
        if let Some(value) = properties.get(col) {
            result.insert(col.clone(), value.clone());
        }
    }
    result
}

/// Convert a PropertyValue (Array of Numbers) to a vector of f32
///
/// This is used when extracting vector embeddings from node properties
/// for distance calculations and HNSW search.
///
/// # Errors
///
/// Returns an error if the PropertyValue is not an array or if array
/// elements cannot be converted to f32.
pub fn property_value_to_vector(pv: &PropertyValue) -> Result<Vec<f32>, String> {
    match pv {
        PropertyValue::Array(arr) => arr
            .iter()
            .map(|v| match v {
                PropertyValue::Integer(n) => Ok(*n as f32),
                PropertyValue::Float(n) => Ok(*n as f32),
                _ => Err(format!("Vector array must contain numbers, found: {:?}", v)),
            })
            .collect(),
        PropertyValue::Vector(vec) => Ok(vec.clone()),
        _ => Err(format!("Expected array for vector type, found: {:?}", pv)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_property_value_string() {
        let pv = PropertyValue::String("hello".to_string());
        let literal = from_property_value(&pv).unwrap();
        assert_eq!(literal, Literal::Text("hello".to_string()));
    }

    #[test]
    fn test_from_property_value_integer() {
        let pv = PropertyValue::Integer(42);
        let literal = from_property_value(&pv).unwrap();
        assert_eq!(literal, Literal::BigInt(42));
    }

    #[test]
    fn test_from_property_value_float() {
        let pv = PropertyValue::Float(42.5);
        let literal = from_property_value(&pv).unwrap();
        assert_eq!(literal, Literal::Double(42.5));
    }

    #[test]
    fn test_from_property_value_boolean() {
        let pv = PropertyValue::Boolean(true);
        let literal = from_property_value(&pv).unwrap();
        assert_eq!(literal, Literal::Boolean(true));
    }

    #[test]
    fn test_to_property_value_text() {
        let literal = Literal::Text("world".to_string());
        let pv = to_property_value(&literal).unwrap();
        assert_eq!(pv, PropertyValue::String("world".to_string()));
    }

    #[test]
    fn test_to_property_value_integer() {
        let literal = Literal::Int(42);
        let pv = to_property_value(&literal).unwrap();
        assert_eq!(pv, PropertyValue::Integer(42));
    }

    #[test]
    fn test_to_property_value_bigint() {
        let literal = Literal::BigInt(9007199254740993); // Beyond f64 safe range
        let pv = to_property_value(&literal).unwrap();
        assert_eq!(pv, PropertyValue::Integer(9007199254740993));
    }

    #[test]
    fn test_to_property_value_null() {
        let literal = Literal::Null;
        let result = to_property_value(&literal).unwrap();
        assert_eq!(result, PropertyValue::Null);
    }

    #[test]
    fn test_infer_data_type() {
        assert_eq!(
            infer_data_type(&PropertyValue::String("test".to_string())),
            DataType::Text
        );
        assert_eq!(
            infer_data_type(&PropertyValue::Integer(42)),
            DataType::BigInt
        );
        assert_eq!(
            infer_data_type(&PropertyValue::Float(42.0)),
            DataType::Double
        );
        assert_eq!(
            infer_data_type(&PropertyValue::Boolean(true)),
            DataType::Boolean
        );
    }

    #[test]
    fn test_can_coerce() {
        let pv = PropertyValue::String("test".to_string());
        assert!(can_coerce(&pv, &DataType::Text));
        assert!(!can_coerce(&pv, &DataType::Int));
    }

    #[test]
    fn test_coerce_string_to_float() {
        let pv = PropertyValue::String("42.5".to_string());
        let coerced = coerce_value(&pv, &DataType::Double).unwrap();
        assert_eq!(coerced, PropertyValue::Float(42.5));
    }

    #[test]
    fn test_coerce_string_to_integer() {
        let pv = PropertyValue::String("42".to_string());
        let coerced = coerce_value(&pv, &DataType::BigInt).unwrap();
        assert_eq!(coerced, PropertyValue::Integer(42));
    }

    #[test]
    fn test_coerce_integer_to_string() {
        let pv = PropertyValue::Integer(42);
        let coerced = coerce_value(&pv, &DataType::Text).unwrap();
        assert_eq!(coerced, PropertyValue::String("42".to_string()));
    }

    #[test]
    fn test_coerce_float_to_string() {
        let pv = PropertyValue::Float(42.5);
        let coerced = coerce_value(&pv, &DataType::Text).unwrap();
        assert_eq!(coerced, PropertyValue::String("42.5".to_string()));
    }

    #[test]
    fn test_extract_columns() {
        let mut props = HashMap::new();
        props.insert("id".to_string(), PropertyValue::String("123".to_string()));
        props.insert(
            "name".to_string(),
            PropertyValue::String("test".to_string()),
        );
        props.insert("count".to_string(), PropertyValue::Integer(42));

        let columns = vec!["id".to_string(), "name".to_string()];
        let extracted = extract_columns(&props, &columns);

        assert_eq!(extracted.len(), 2);
        assert_eq!(
            extracted.get("id"),
            Some(&PropertyValue::String("123".to_string()))
        );
        assert_eq!(
            extracted.get("name"),
            Some(&PropertyValue::String("test".to_string()))
        );
        assert_eq!(extracted.get("count"), None);

        let ordered_keys: Vec<_> = extracted.keys().cloned().collect();
        assert_eq!(ordered_keys, vec!["id".to_string(), "name".to_string()]);
    }
}
