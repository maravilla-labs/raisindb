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

//! Type mapping between RaisinDB PropertyValue types and PostgreSQL wire format types.
//!
//! This module provides conversions from RaisinDB's `PropertyValue` enum to PostgreSQL
//! types as defined in the `postgres_types::Type` enum. It handles both type detection
//! and text protocol encoding.
//!
//! # Type Mappings
//!
//! | PropertyValue Variant | PostgreSQL Type | Notes |
//! |-----------------------|-----------------|-------|
//! | `Null`                | `TEXT`          | Nullable handling |
//! | `Boolean`             | `BOOL`          | PostgreSQL boolean |
//! | `Integer`             | `INT8`          | 64-bit integer |
//! | `Float`               | `FLOAT8`        | Double precision |
//! | `Decimal`             | `NUMERIC`       | Arbitrary precision |
//! | `String`              | `TEXT`          | UTF-8 text |
//! | `Date`                | `TIMESTAMPTZ`   | Timestamp with timezone |
//! | `Reference`           | `JSONB`         | Serialized as JSON |
//! | `Url`                 | `JSONB`         | Serialized as JSON |
//! | `Resource`            | `JSONB`         | Serialized as JSON |
//! | `Composite`           | `JSONB`         | Serialized as JSON |
//! | `Element`             | `JSONB`         | Serialized as JSON |
//! | `Vector`              | `FLOAT4_ARRAY`  | pgvector-compatible |
//! | `Array`               | `JSONB`         | Heterogeneous arrays as JSON |
//! | `Object`              | `JSONB`         | Key-value maps as JSON |
//!
//! # Examples
//!
//! ```rust
//! use raisin_models::nodes::properties::PropertyValue;
//! use raisin_transport_pgwire::type_mapping::{to_pg_type, encode_value_text};
//!
//! // Map a RaisinDB value to PostgreSQL type
//! let value = PropertyValue::Integer(42);
//! let pg_type = to_pg_type(&value);
//! assert_eq!(pg_type, postgres_types::Type::INT8);
//!
//! // Encode value as text for PostgreSQL wire protocol
//! let text = encode_value_text(&value).unwrap();
//! assert_eq!(text, "42");
//! ```

use postgres_types::Type;
use raisin_models::nodes::properties::PropertyValue;

use crate::error::PgWireTransportError;
use crate::Result;

/// Maps a RaisinDB `PropertyValue` to the corresponding PostgreSQL `Type`.
///
/// This function determines the appropriate PostgreSQL type for wire protocol
/// communication based on the variant of the `PropertyValue` enum.
///
/// # Arguments
///
/// * `value` - A reference to the PropertyValue to map
///
/// # Returns
///
/// The corresponding PostgreSQL `Type` enum value
///
/// # Examples
///
/// ```rust,no_run
/// use raisin_models::nodes::properties::PropertyValue;
/// use raisin_transport_pgwire::type_mapping::to_pg_type;
/// use postgres_types::Type;
///
/// let value = PropertyValue::String("hello".to_string());
/// assert_eq!(to_pg_type(&value), Type::TEXT);
///
/// let value = PropertyValue::Integer(123);
/// assert_eq!(to_pg_type(&value), Type::INT8);
///
/// let value = PropertyValue::Boolean(true);
/// assert_eq!(to_pg_type(&value), Type::BOOL);
/// ```
pub fn to_pg_type(value: &PropertyValue) -> Type {
    match value {
        PropertyValue::Null => Type::TEXT, // Nullable text - will encode as NULL
        PropertyValue::Boolean(_) => Type::BOOL,
        PropertyValue::Integer(_) => Type::INT8,
        PropertyValue::Float(_) => Type::FLOAT8,
        PropertyValue::Decimal(_) => Type::NUMERIC,
        PropertyValue::String(_) => Type::TEXT,
        PropertyValue::Date(_) => Type::TIMESTAMPTZ,

        // Domain-specific types serialized as JSONB
        PropertyValue::Reference(_) => Type::JSONB,
        PropertyValue::Url(_) => Type::JSONB,
        PropertyValue::Resource(_) => Type::JSONB,
        PropertyValue::Composite(_) => Type::JSONB,
        PropertyValue::Element(_) => Type::JSONB,

        // Collections
        PropertyValue::Vector(_) => Type::FLOAT4_ARRAY, // pgvector-compatible
        PropertyValue::Array(_) => Type::JSONB,         // Heterogeneous arrays as JSON
        PropertyValue::Object(_) => Type::JSONB,        // Maps as JSON

        // Spatial types
        PropertyValue::Geometry(_) => Type::JSONB, // GeoJSON as JSONB
    }
}

/// Encodes a RaisinDB `PropertyValue` as a text string for PostgreSQL wire protocol.
///
/// This function converts a PropertyValue to its PostgreSQL text representation,
/// suitable for use in text-based query results. Complex types (Reference, Url,
/// Resource, etc.) are serialized as JSON.
///
/// # Arguments
///
/// * `value` - A reference to the PropertyValue to encode
///
/// # Returns
///
/// A `Result` containing the encoded string, or a `PgWireError` if serialization fails
///
/// # Errors
///
/// Returns `PgWireTransportError::TypeConversion` if JSON serialization fails for complex types
///
/// # Examples
///
/// ```rust,no_run
/// use raisin_models::nodes::properties::PropertyValue;
/// use raisin_transport_pgwire::type_mapping::encode_value_text;
///
/// // Primitive types
/// assert_eq!(encode_value_text(&PropertyValue::Integer(42)).unwrap(), "42");
/// assert_eq!(encode_value_text(&PropertyValue::Boolean(true)).unwrap(), "t");
/// assert_eq!(encode_value_text(&PropertyValue::String("hello".to_string())).unwrap(), "hello");
///
/// // Null values
/// assert_eq!(encode_value_text(&PropertyValue::Null).unwrap(), "");
/// ```
pub fn encode_value_text(value: &PropertyValue) -> Result<String> {
    match value {
        PropertyValue::Null => Ok(String::new()), // Empty string for NULL

        PropertyValue::Boolean(b) => {
            // PostgreSQL boolean text format: 't' for true, 'f' for false
            Ok(if *b { "t".to_string() } else { "f".to_string() })
        }

        PropertyValue::Integer(i) => Ok(i.to_string()),

        PropertyValue::Float(f) => {
            // Handle special float values
            if f.is_nan() {
                Ok("NaN".to_string())
            } else if f.is_infinite() {
                if *f > 0.0 {
                    Ok("Infinity".to_string())
                } else {
                    Ok("-Infinity".to_string())
                }
            } else {
                Ok(f.to_string())
            }
        }

        PropertyValue::Decimal(d) => Ok(d.to_string()),

        PropertyValue::String(s) => Ok(s.clone()),

        PropertyValue::Date(dt) => {
            // Format as PostgreSQL TIMESTAMPTZ: 'YYYY-MM-DD HH:MM:SS.ffffff+TZ'
            Ok(dt
                .as_datetime()
                .format("%Y-%m-%d %H:%M:%S%.6f%z")
                .to_string())
        }

        // Domain-specific types - serialize as JSON
        PropertyValue::Reference(r) => serde_json::to_string(r).map_err(|e| {
            PgWireTransportError::type_conversion(format!("Failed to serialize Reference: {}", e))
        }),

        PropertyValue::Url(u) => serde_json::to_string(u).map_err(|e| {
            PgWireTransportError::type_conversion(format!("Failed to serialize Url: {}", e))
        }),

        PropertyValue::Resource(res) => serde_json::to_string(res).map_err(|e| {
            PgWireTransportError::type_conversion(format!("Failed to serialize Resource: {}", e))
        }),

        PropertyValue::Composite(comp) => serde_json::to_string(comp).map_err(|e| {
            PgWireTransportError::type_conversion(format!("Failed to serialize Composite: {}", e))
        }),

        PropertyValue::Element(elem) => serde_json::to_string(elem).map_err(|e| {
            PgWireTransportError::type_conversion(format!("Failed to serialize Element: {}", e))
        }),

        // Vector - encode as PostgreSQL array format: {v1,v2,v3,...}
        PropertyValue::Vector(vec) => {
            if vec.is_empty() {
                Ok("{}".to_string())
            } else {
                let elements: Vec<String> = vec.iter().map(|v| v.to_string()).collect();
                Ok(format!("{{{}}}", elements.join(",")))
            }
        }

        // Heterogeneous arrays - serialize as JSON
        PropertyValue::Array(arr) => serde_json::to_string(arr).map_err(|e| {
            PgWireTransportError::type_conversion(format!("Failed to serialize Array: {}", e))
        }),

        // Objects - serialize as JSON
        PropertyValue::Object(obj) => serde_json::to_string(obj).map_err(|e| {
            PgWireTransportError::type_conversion(format!("Failed to serialize Object: {}", e))
        }),

        // Geometry - serialize as GeoJSON
        PropertyValue::Geometry(geo) => serde_json::to_string(geo).map_err(|e| {
            PgWireTransportError::type_conversion(format!("Failed to serialize Geometry: {}", e))
        }),
    }
}

/// Checks if a PropertyValue represents a NULL value.
///
/// This is a utility function to determine if a value should be encoded
/// as NULL in the PostgreSQL wire protocol.
///
/// # Arguments
///
/// * `value` - A reference to the PropertyValue to check
///
/// # Returns
///
/// `true` if the value is `PropertyValue::Null`, `false` otherwise
#[inline]
pub fn is_null(value: &PropertyValue) -> bool {
    matches!(value, PropertyValue::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use raisin_models::nodes::properties::value::{RaisinReference, RaisinUrl};
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::str::FromStr;

    #[test]
    fn test_to_pg_type_primitives() {
        assert_eq!(to_pg_type(&PropertyValue::Null), Type::TEXT);
        assert_eq!(to_pg_type(&PropertyValue::Boolean(true)), Type::BOOL);
        assert_eq!(to_pg_type(&PropertyValue::Integer(42)), Type::INT8);
        assert_eq!(to_pg_type(&PropertyValue::Float(3.14)), Type::FLOAT8);
        assert_eq!(
            to_pg_type(&PropertyValue::Decimal(
                Decimal::from_str("123.45").unwrap()
            )),
            Type::NUMERIC
        );
        assert_eq!(
            to_pg_type(&PropertyValue::String("test".to_string())),
            Type::TEXT
        );
    }

    #[test]
    fn test_to_pg_type_date() {
        let timestamp = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let value = PropertyValue::Date(timestamp.into());
        assert_eq!(to_pg_type(&value), Type::TIMESTAMPTZ);
    }

    #[test]
    fn test_to_pg_type_collections() {
        assert_eq!(
            to_pg_type(&PropertyValue::Vector(vec![1.0, 2.0, 3.0])),
            Type::FLOAT4_ARRAY
        );
        assert_eq!(
            to_pg_type(&PropertyValue::Array(vec![PropertyValue::Integer(1)])),
            Type::JSONB
        );
        assert_eq!(
            to_pg_type(&PropertyValue::Object(HashMap::new())),
            Type::JSONB
        );
    }

    #[test]
    fn test_to_pg_type_domain_types() {
        let reference = RaisinReference {
            id: "test-id".to_string(),
            workspace: "test-ws".to_string(),
            path: "/test/path".to_string(),
        };
        assert_eq!(
            to_pg_type(&PropertyValue::Reference(reference)),
            Type::JSONB
        );

        let url = RaisinUrl::new("https://example.com");
        assert_eq!(to_pg_type(&PropertyValue::Url(url)), Type::JSONB);
    }

    #[test]
    fn test_encode_value_text_primitives() {
        assert_eq!(encode_value_text(&PropertyValue::Null).unwrap(), "");
        assert_eq!(
            encode_value_text(&PropertyValue::Boolean(true)).unwrap(),
            "t"
        );
        assert_eq!(
            encode_value_text(&PropertyValue::Boolean(false)).unwrap(),
            "f"
        );
        assert_eq!(
            encode_value_text(&PropertyValue::Integer(42)).unwrap(),
            "42"
        );
        assert_eq!(
            encode_value_text(&PropertyValue::Integer(-123)).unwrap(),
            "-123"
        );
        assert_eq!(
            encode_value_text(&PropertyValue::Float(3.14)).unwrap(),
            "3.14"
        );
        assert_eq!(
            encode_value_text(&PropertyValue::String("hello".to_string())).unwrap(),
            "hello"
        );
    }

    #[test]
    fn test_encode_value_text_special_floats() {
        assert_eq!(
            encode_value_text(&PropertyValue::Float(f64::NAN)).unwrap(),
            "NaN"
        );
        assert_eq!(
            encode_value_text(&PropertyValue::Float(f64::INFINITY)).unwrap(),
            "Infinity"
        );
        assert_eq!(
            encode_value_text(&PropertyValue::Float(f64::NEG_INFINITY)).unwrap(),
            "-Infinity"
        );
    }

    #[test]
    fn test_encode_value_text_decimal() {
        let decimal = Decimal::from_str("123.456789").unwrap();
        assert_eq!(
            encode_value_text(&PropertyValue::Decimal(decimal)).unwrap(),
            "123.456789"
        );
    }

    #[test]
    fn test_encode_value_text_date() {
        let timestamp = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let value = PropertyValue::Date(timestamp.into());
        let encoded = encode_value_text(&value).unwrap();
        // Should be RFC3339 format
        assert!(encoded.contains("2023-11-14"));
    }

    #[test]
    fn test_encode_value_text_vector() {
        let vector = vec![1.0, 2.0, 3.5, -4.2];
        let encoded = encode_value_text(&PropertyValue::Vector(vector)).unwrap();
        assert_eq!(encoded, "{1,2,3.5,-4.2}");

        // Empty vector
        let empty_vector: Vec<f32> = vec![];
        let encoded = encode_value_text(&PropertyValue::Vector(empty_vector)).unwrap();
        assert_eq!(encoded, "{}");
    }

    #[test]
    fn test_encode_value_text_array() {
        let array = vec![
            PropertyValue::Integer(1),
            PropertyValue::String("test".to_string()),
            PropertyValue::Boolean(true),
        ];
        let encoded = encode_value_text(&PropertyValue::Array(array)).unwrap();
        // Should be valid JSON
        assert!(serde_json::from_str::<serde_json::Value>(&encoded).is_ok());
    }

    #[test]
    fn test_encode_value_text_object() {
        let mut obj = HashMap::new();
        obj.insert("key1".to_string(), PropertyValue::Integer(42));
        obj.insert(
            "key2".to_string(),
            PropertyValue::String("value".to_string()),
        );
        let encoded = encode_value_text(&PropertyValue::Object(obj)).unwrap();
        // Should be valid JSON
        assert!(serde_json::from_str::<serde_json::Value>(&encoded).is_ok());
    }

    #[test]
    fn test_encode_value_text_reference() {
        let reference = RaisinReference {
            id: "node-123".to_string(),
            workspace: "my-workspace".to_string(),
            path: "/documents/readme".to_string(),
        };
        let encoded = encode_value_text(&PropertyValue::Reference(reference)).unwrap();
        // Should be valid JSON with raisin:ref structure
        let json: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(json["raisin:ref"], "node-123");
        assert_eq!(json["raisin:workspace"], "my-workspace");
        assert_eq!(json["raisin:path"], "/documents/readme");
    }

    #[test]
    fn test_encode_value_text_url() {
        let url = RaisinUrl::new("https://example.com")
            .with_title("Example Site")
            .with_description("An example website");
        let encoded = encode_value_text(&PropertyValue::Url(url)).unwrap();
        // Should be valid JSON
        let json: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(json["raisin:url"], "https://example.com");
        assert_eq!(json["raisin:title"], "Example Site");
        assert_eq!(json["raisin:description"], "An example website");
    }

    #[test]
    fn test_is_null() {
        assert!(is_null(&PropertyValue::Null));
        assert!(!is_null(&PropertyValue::Integer(0)));
        assert!(!is_null(&PropertyValue::String(String::new())));
        assert!(!is_null(&PropertyValue::Boolean(false)));
    }
}
