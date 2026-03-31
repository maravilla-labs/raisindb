// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Common utility functions for DML execution.
//!
//! Contains expression evaluation, type conversion, column extraction,
//! and other shared helpers used across DML operations.

use crate::physical_plan::executor::Row;
use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};

/// Extract the 'name' value from a WHERE clause filter.
///
/// Currently supports simple patterns like: WHERE name = 'value'
/// Returns error if filter is missing, complex, or doesn't reference 'name'.
pub(super) fn extract_name_from_filter(filter: &Option<TypedExpr>) -> Result<String, Error> {
    let filter_expr = filter.as_ref().ok_or_else(|| {
        Error::Validation(
            "UPDATE/DELETE requires a WHERE clause to identify the target row (e.g., WHERE name = 'YourType')"
                .to_string(),
        )
    })?;

    // Try to extract: name = 'literal'
    if let Expr::BinaryOp {
        left,
        op: raisin_sql::analyzer::BinaryOperator::Eq,
        right,
    } = &filter_expr.expr
    {
        // Check if left side is a column named 'name'
        if let Expr::Column { column, .. } = &left.expr {
            if column == "name" {
                // Extract literal value from right side
                if let Expr::Literal(Literal::Text(name_value)) = &right.expr {
                    return Ok(name_value.clone());
                }
            }
        }
    }

    Err(Error::Validation(
        "UPDATE/DELETE WHERE clause must be a simple equality: WHERE name = 'value'".to_string(),
    ))
}

/// Evaluate a TypedExpr to a PropertyValue.
///
/// Handles literal expressions and simple casts.
/// Returns error for complex expressions that can't be evaluated at plan time.
pub(super) fn eval_expr_to_property_value(expr: &TypedExpr) -> Result<PropertyValue, Error> {
    match &expr.expr {
        Expr::Literal(lit) => literal_to_property_value(lit),
        Expr::Cast { expr, target_type } => {
            // Evaluate the inner expression first
            let inner_value = eval_expr_to_property_value(expr)?;

            // Apply JSONB cast if needed - parse string as JSON
            if matches!(target_type, DataType::JsonB) {
                if let PropertyValue::String(s) = inner_value {
                    // Parse JSON string to serde_json::Value, then convert to PropertyValue
                    let json: serde_json::Value = serde_json::from_str(&s).map_err(|e| {
                        Error::Validation(format!("Cannot cast '{}' to JSONB: {}", s, e))
                    })?;
                    return literal_to_property_value(&Literal::JsonB(json));
                }
            }

            // For other casts, return the inner value as-is
            Ok(inner_value)
        }
        _ => Err(Error::InvalidState(format!(
            "Complex expressions in DML VALUES are not yet supported: {:?}",
            expr.expr
        ))),
    }
}

/// Evaluate a TypedExpr to a PropertyValue using row context.
///
/// Handles complex expressions including JSONB operators, function calls,
/// and column references. Used for workspace UPDATE operations.
pub(super) fn eval_expr_with_row_to_property_value(
    expr: &TypedExpr,
    row: &Row,
) -> Result<PropertyValue, Error> {
    use crate::physical_plan::eval::core::eval_expr;
    let literal = eval_expr(expr, row)?;
    literal_to_property_value(&literal)
}

/// Convert a Node to a Row for expression evaluation.
///
/// Creates a Row with all node fields as qualified columns (workspace.column)
/// so that expressions like `properties || '{...}'` can reference the existing values.
pub(super) fn node_to_row(node: &raisin_models::nodes::Node, workspace: &str) -> Row {
    let mut row = Row::new();

    // Add standard node columns with workspace qualifier
    row.insert(
        format!("{}.id", workspace),
        PropertyValue::String(node.id.clone()),
    );
    row.insert(
        format!("{}.name", workspace),
        PropertyValue::String(node.name.clone()),
    );
    row.insert(
        format!("{}.path", workspace),
        PropertyValue::String(node.path.clone()),
    );
    row.insert(
        format!("{}.node_type", workspace),
        PropertyValue::String(node.node_type.clone()),
    );
    if let Some(archetype) = &node.archetype {
        row.insert(
            format!("{}.archetype", workspace),
            PropertyValue::String(archetype.clone()),
        );
    }
    row.insert(
        format!("{}.version", workspace),
        PropertyValue::Integer(node.version as i64),
    );

    // Add properties as JSONB object
    row.insert(
        format!("{}.properties", workspace),
        PropertyValue::Object(node.properties.clone()),
    );

    // Also add individual properties with workspace qualifier for direct column access
    for (key, value) in &node.properties {
        row.insert(format!("{}.{}", workspace, key), value.clone());
    }

    row
}

/// Convert a Literal to a PropertyValue.
pub(super) fn literal_to_property_value(lit: &Literal) -> Result<PropertyValue, Error> {
    match lit {
        Literal::Boolean(b) => Ok(PropertyValue::Boolean(*b)),
        Literal::Int(i) => Ok(PropertyValue::Integer(*i as i64)),
        Literal::BigInt(i) => Ok(PropertyValue::Integer(*i)),
        Literal::Double(f) => Ok(PropertyValue::Float(*f)),
        Literal::Text(s) => Ok(PropertyValue::String(s.clone())),
        Literal::JsonB(j) => {
            // Convert JSON to PropertyValue::Object if it's an object
            if let serde_json::Value::Object(map) = j {
                let mut prop_map = std::collections::HashMap::new();
                for (k, v) in map {
                    prop_map.insert(k.clone(), json_value_to_property_value(v)?);
                }
                Ok(PropertyValue::Object(prop_map))
            } else {
                Err(Error::Validation(
                    "JSONB values must be objects for PropertyValue conversion".to_string(),
                ))
            }
        }
        Literal::Null => Ok(PropertyValue::Null),
        _ => Err(Error::Validation(format!(
            "Cannot convert literal {:?} to PropertyValue",
            lit
        ))),
    }
}

/// Convert serde_json::Value to PropertyValue.
pub(super) fn json_value_to_property_value(v: &serde_json::Value) -> Result<PropertyValue, Error> {
    match v {
        serde_json::Value::Bool(b) => Ok(PropertyValue::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(PropertyValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(PropertyValue::Float(f))
            } else {
                Err(Error::Validation("Invalid number in JSON".to_string()))
            }
        }
        serde_json::Value::String(s) => Ok(PropertyValue::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let mut values = Vec::new();
            for item in arr {
                values.push(json_value_to_property_value(item)?);
            }
            Ok(PropertyValue::Array(values))
        }
        serde_json::Value::Object(map) => {
            // Check if this is a RaisinReference
            if let (Some(ref_val), Some(ws_val)) =
                (map.get("raisin:ref"), map.get("raisin:workspace"))
            {
                if let (serde_json::Value::String(ref_str), serde_json::Value::String(ws_str)) =
                    (ref_val, ws_val)
                {
                    let path_str = map
                        .get("raisin:path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("")
                        .to_string();

                    return Ok(PropertyValue::Reference(RaisinReference {
                        id: ref_str.clone(),
                        workspace: ws_str.clone(),
                        path: path_str,
                    }));
                }
            }

            // Fallback: regular object
            let mut prop_map = std::collections::HashMap::new();
            for (k, v) in map {
                prop_map.insert(k.clone(), json_value_to_property_value(v)?);
            }
            Ok(PropertyValue::Object(prop_map))
        }
        serde_json::Value::Null => Ok(PropertyValue::Null),
    }
}

// =============================================================================
// Column Extraction Helpers
// =============================================================================

pub(super) fn extract_string_column(
    col_map: &IndexMap<String, PropertyValue>,
    col_name: &str,
) -> Result<String, Error> {
    match col_map.get(col_name) {
        Some(PropertyValue::String(s)) => Ok(s.clone()),
        Some(_) => Err(Error::Validation(format!(
            "Column '{}' must be a string",
            col_name
        ))),
        None => Err(Error::Validation(format!(
            "Required column '{}' is missing",
            col_name
        ))),
    }
}

pub(super) fn extract_optional_string_column(
    col_map: &IndexMap<String, PropertyValue>,
    col_name: &str,
) -> Option<String> {
    match col_map.get(col_name) {
        Some(PropertyValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

pub(super) fn extract_optional_boolean_column(
    col_map: &IndexMap<String, PropertyValue>,
    col_name: &str,
) -> Option<bool> {
    match col_map.get(col_name) {
        Some(PropertyValue::Boolean(b)) => Some(*b),
        _ => None,
    }
}

pub(super) fn extract_number_column(
    col_map: &IndexMap<String, PropertyValue>,
    col_name: &str,
) -> Result<f64, Error> {
    match col_map.get(col_name) {
        Some(PropertyValue::Integer(n)) => Ok(*n as f64),
        Some(PropertyValue::Float(n)) => Ok(*n),
        Some(_) => Err(Error::Validation(format!(
            "Column '{}' must be a number",
            col_name
        ))),
        None => Err(Error::Validation(format!(
            "Required column '{}' is missing",
            col_name
        ))),
    }
}

pub(super) fn extract_string_value(value: &PropertyValue) -> Result<String, Error> {
    match value {
        PropertyValue::String(s) => Ok(s.clone()),
        _ => Err(Error::Validation("Expected string value".to_string())),
    }
}

pub(super) fn extract_number_value(value: &PropertyValue) -> Result<f64, Error> {
    match value {
        PropertyValue::Integer(n) => Ok(*n as f64),
        PropertyValue::Float(n) => Ok(*n),
        _ => Err(Error::Validation("Expected number value".to_string())),
    }
}

/// Extract boolean value from PropertyValue.
pub(super) fn extract_boolean_value(value: &PropertyValue) -> Result<bool, Error> {
    match value {
        PropertyValue::Boolean(b) => Ok(*b),
        _ => Err(Error::Validation("Expected boolean value".to_string())),
    }
}

/// Extract Vec<String> from PropertyValue::Array.
pub(super) fn extract_string_array(value: &PropertyValue) -> Result<Vec<String>, Error> {
    match value {
        PropertyValue::Array(arr) => {
            let mut result = Vec::new();
            for item in arr {
                if let PropertyValue::String(s) = item {
                    result.push(s.clone());
                } else {
                    return Err(Error::Validation(
                        "Array must contain only strings".to_string(),
                    ));
                }
            }
            Ok(result)
        }
        _ => Err(Error::Validation("Expected array value".to_string())),
    }
}

/// Convert PropertyValue to serde_json::Value.
pub(super) fn property_value_to_json(value: &PropertyValue) -> Result<serde_json::Value, Error> {
    match value {
        PropertyValue::Null => Ok(serde_json::Value::Null),
        PropertyValue::String(s) => Ok(serde_json::Value::String(s.clone())),
        PropertyValue::Integer(n) => Ok(serde_json::Value::Number((*n).into())),
        PropertyValue::Float(n) => Ok(serde_json::Value::Number(
            serde_json::Number::from_f64(*n)
                .ok_or_else(|| Error::Validation("Invalid number".to_string()))?,
        )),
        PropertyValue::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        PropertyValue::Array(arr) => {
            let values: Result<Vec<_>, _> = arr.iter().map(property_value_to_json).collect();
            Ok(serde_json::Value::Array(values?))
        }
        PropertyValue::Object(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                obj.insert(k.clone(), property_value_to_json(v)?);
            }
            Ok(serde_json::Value::Object(obj))
        }
        // For other types, serialize them via serde_json
        other => serde_json::to_value(other)
            .map_err(|e| Error::Validation(format!("Failed to convert property value: {}", e))),
    }
}

/// Convert PropertyValue to target type via JSON serialization.
///
/// Leverages serde to convert from generic PropertyValue
/// to specific Rust types like Vec<PropertyValueSchema>, Vec<String>, etc.
pub(super) fn convert_property_value<T: serde::de::DeserializeOwned>(
    value: &PropertyValue,
    column_name: &str,
) -> Result<T, Error> {
    let json_value = property_value_to_json(value)?;
    serde_json::from_value(json_value).map_err(|e| {
        Error::Validation(format!(
            "Failed to convert column '{}' value: {}",
            column_name, e
        ))
    })
}
