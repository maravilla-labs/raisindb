//! Helper functions for completion provider
//!
//! Shared utilities for formatting data types and checking type compatibility.

use crate::analyzer::types::DataType;

/// Format DataType for display
pub(super) fn format_data_type(dt: &DataType) -> String {
    match dt {
        DataType::Int => "Int".to_string(),
        DataType::BigInt => "BigInt".to_string(),
        DataType::Double => "Double".to_string(),
        DataType::Boolean => "Boolean".to_string(),
        DataType::Text => "Text".to_string(),
        DataType::Uuid => "Uuid".to_string(),
        DataType::TimestampTz => "Timestamp".to_string(),
        DataType::Interval => "Interval".to_string(),
        DataType::Path => "Path".to_string(),
        DataType::JsonB => "JsonB".to_string(),
        DataType::Vector(n) => format!("Vector({})", n),
        DataType::Geometry => "Geometry".to_string(),
        DataType::Array(inner) => format!("Array<{}>", format_data_type(inner)),
        DataType::Nullable(inner) => format!("{}?", format_data_type(inner)),
        DataType::TSVector => "TSVector".to_string(),
        DataType::TSQuery => "TSQuery".to_string(),
        DataType::Unknown => "Any".to_string(),
    }
}

/// Check if column type is compatible with expected type
pub(super) fn is_type_compatible(col_type: &DataType, expected: &DataType) -> bool {
    // Unknown matches anything
    if matches!(expected, DataType::Unknown) {
        return true;
    }

    // Exact match
    if col_type == expected {
        return true;
    }

    // Check coercion
    col_type.can_coerce_to(expected)
}
