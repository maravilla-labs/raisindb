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

//! Binary encoding for PostgreSQL wire protocol.
//!
//! PostgreSQL JDBC driver switches from text to binary protocol after ~5 prepared
//! statement executions. This module implements binary encoding for all supported types.
//!
//! Key difference: PostgreSQL epoch is 2000-01-01, not Unix 1970-01-01.
//!
//! # Binary Type Formats
//!
//! | Type | Binary Format |
//! |------|--------------|
//! | BOOL | 1 byte (0x00/0x01) |
//! | INT4 | 4 bytes big-endian |
//! | INT8 | 8 bytes big-endian |
//! | FLOAT8 | 8 bytes IEEE 754 big-endian |
//! | TEXT | Raw UTF-8 bytes |
//! | UUID | 16 raw bytes |
//! | TIMESTAMPTZ | 8 bytes (microseconds since 2000-01-01) |
//! | JSONB | 0x01 version byte + JSON text |

use crate::error::PgWireTransportError;
use crate::Result;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use postgres_types::Type;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::StorageTimestamp;
use uuid::Uuid;

/// Offset in seconds from Unix epoch (1970-01-01) to PostgreSQL epoch (2000-01-01).
const PG_EPOCH_OFFSET_SECS: i64 = 946_684_800;

/// Encode a boolean value in PostgreSQL binary format.
/// Format: 1 byte, 0x01 for true, 0x00 for false.
#[inline]
pub fn encode_bool_binary(value: bool) -> Vec<u8> {
    vec![if value { 0x01 } else { 0x00 }]
}

/// Encode a 32-bit integer in PostgreSQL binary format.
/// Format: 4 bytes, big-endian.
#[inline]
pub fn encode_int4_binary(value: i32) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

/// Encode a 64-bit integer in PostgreSQL binary format.
/// Format: 8 bytes, big-endian.
#[inline]
pub fn encode_int8_binary(value: i64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

/// Encode a 64-bit float in PostgreSQL binary format.
/// Format: 8 bytes, IEEE 754 double precision, big-endian.
#[inline]
pub fn encode_float8_binary(value: f64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

/// Encode text in PostgreSQL binary format.
/// Format: Raw UTF-8 bytes (length is handled by pgwire protocol layer).
#[inline]
pub fn encode_text_binary(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}

/// Encode a timestamp with timezone in PostgreSQL binary format.
/// Format: 8 bytes, microseconds since PostgreSQL epoch (2000-01-01 00:00:00 UTC).
///
/// IMPORTANT: PostgreSQL uses 2000-01-01 as epoch, not Unix 1970-01-01.
pub fn encode_timestamp_binary(dt: &StorageTimestamp) -> Vec<u8> {
    let datetime = dt.as_datetime();
    let unix_micros = datetime.timestamp_micros();
    let pg_micros = unix_micros - (PG_EPOCH_OFFSET_SECS * 1_000_000);
    pg_micros.to_be_bytes().to_vec()
}

/// Encode JSONB in PostgreSQL binary format.
/// Format: 1 byte version (0x01) + JSON text as UTF-8.
pub fn encode_jsonb_binary(json: &str) -> Vec<u8> {
    let mut result = Vec::with_capacity(1 + json.len());
    result.push(0x01); // JSONB version 1
    result.extend_from_slice(json.as_bytes());
    result
}

/// Attempts to convert a string to a `StorageTimestamp`.
/// Handles RFC3339 and common SQL timestamp formats.
fn string_to_timestamp(value: &str) -> Result<StorageTimestamp> {
    // Try RFC3339 first (common for JSON)
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Ok(StorageTimestamp::from(dt.with_timezone(&Utc)));
    }

    // Try common SQL timestamp format without timezone
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(StorageTimestamp::from(Utc.from_utc_datetime(&naive)));
    }

    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Ok(StorageTimestamp::from(Utc.from_utc_datetime(&naive)));
    }

    Err(PgWireTransportError::type_conversion(format!(
        "Failed to parse timestamp string '{}' for binary encoding",
        value
    )))
}

/// Converts a `PropertyValue` to a `StorageTimestamp` if possible.
/// This handles `Date` type directly and attempts to parse `String` and `Integer` types.
fn to_timestamp(value: &PropertyValue) -> Result<StorageTimestamp> {
    match value {
        PropertyValue::Date(dt) => Ok(*dt),
        PropertyValue::String(s) => string_to_timestamp(s),
        PropertyValue::Integer(i) => {
            StorageTimestamp::from_epoch_auto_detect(*i).ok_or_else(|| {
                PgWireTransportError::type_conversion(format!(
                    "Failed to convert integer '{}' to timestamp",
                    i
                ))
            })
        }
        _ => Err(PgWireTransportError::type_conversion(format!(
            "Cannot convert {:?} to timestamp",
            value
        ))),
    }
}

/// Encode a PropertyValue in PostgreSQL binary format based on the target type.
///
/// Returns the binary representation suitable for the pgwire protocol.
pub fn encode_value_binary(value: &PropertyValue, pg_type: &Type) -> Result<Vec<u8>> {
    match pg_type {
        // Handle explicit timestamp types first
        &Type::TIMESTAMP | &Type::TIMESTAMPTZ => {
            let ts = to_timestamp(value)?;
            return Ok(encode_timestamp_binary(&ts));
        }
        _ => {}
    }

    match value {
        PropertyValue::Null => {
            // NULL is handled at the protocol level, not here
            Ok(Vec::new())
        }

        PropertyValue::Boolean(b) => Ok(encode_bool_binary(*b)),

        PropertyValue::Integer(i) => {
            match pg_type {
                &Type::INT4 => Ok(encode_int4_binary(*i as i32)),
                _ => Ok(encode_int8_binary(*i)), // Default to INT8
            }
        }

        PropertyValue::Float(f) => Ok(encode_float8_binary(*f)),

        PropertyValue::Decimal(d) => {
            // For NUMERIC/DECIMAL, encode as text since binary NUMERIC is complex
            Ok(encode_text_binary(&d.to_string()))
        }

        PropertyValue::String(s) => match pg_type {
            &Type::UUID => {
                let uuid = Uuid::parse_str(s).map_err(|e| {
                    PgWireTransportError::type_conversion(format!(
                        "Failed to parse UUID '{}' for binary encoding: {}",
                        s, e
                    ))
                })?;
                Ok(uuid.as_bytes().to_vec())
            }
            _ => Ok(encode_text_binary(s)),
        },

        PropertyValue::Date(dt) => {
            // Fallback for Date when pg_type wasn't timestamp
            Ok(encode_text_binary(&dt.to_rfc3339()))
        }

        // Domain-specific types - serialize as JSONB
        PropertyValue::Reference(r) => {
            let json_str = serde_json::to_string(r).map_err(|e| {
                PgWireTransportError::type_conversion(format!(
                    "Failed to serialize Reference for binary: {}",
                    e
                ))
            })?;
            Ok(encode_jsonb_binary(&json_str))
        }

        PropertyValue::Url(u) => {
            let json_str = serde_json::to_string(u).map_err(|e| {
                PgWireTransportError::type_conversion(format!(
                    "Failed to serialize Url for binary: {}",
                    e
                ))
            })?;
            Ok(encode_jsonb_binary(&json_str))
        }

        PropertyValue::Resource(res) => {
            let json_str = serde_json::to_string(res).map_err(|e| {
                PgWireTransportError::type_conversion(format!(
                    "Failed to serialize Resource for binary: {}",
                    e
                ))
            })?;
            Ok(encode_jsonb_binary(&json_str))
        }

        PropertyValue::Composite(comp) => {
            let json_str = serde_json::to_string(comp).map_err(|e| {
                PgWireTransportError::type_conversion(format!(
                    "Failed to serialize Composite for binary: {}",
                    e
                ))
            })?;
            Ok(encode_jsonb_binary(&json_str))
        }

        PropertyValue::Element(elem) => {
            let json_str = serde_json::to_string(elem).map_err(|e| {
                PgWireTransportError::type_conversion(format!(
                    "Failed to serialize Element for binary: {}",
                    e
                ))
            })?;
            Ok(encode_jsonb_binary(&json_str))
        }

        PropertyValue::Vector(vec) => {
            // Vector as PostgreSQL FLOAT4 array binary format
            // Binary array format: dimensions, flags, element_oid, dim1_size, dim1_lbound, elements...
            // For simplicity, encode as JSONB instead (compatible and simpler)
            let json_str = serde_json::to_string(vec).map_err(|e| {
                PgWireTransportError::type_conversion(format!(
                    "Failed to serialize Vector for binary: {}",
                    e
                ))
            })?;
            Ok(encode_jsonb_binary(&json_str))
        }

        PropertyValue::Array(arr) => {
            let json_str = serde_json::to_string(arr).map_err(|e| {
                PgWireTransportError::type_conversion(format!(
                    "Failed to serialize Array for binary: {}",
                    e
                ))
            })?;
            Ok(encode_jsonb_binary(&json_str))
        }

        PropertyValue::Object(obj) => {
            let json_str = serde_json::to_string(obj).map_err(|e| {
                PgWireTransportError::type_conversion(format!(
                    "Failed to serialize Object for binary: {}",
                    e
                ))
            })?;
            Ok(encode_jsonb_binary(&json_str))
        }

        PropertyValue::Geometry(geo) => {
            let json_str = serde_json::to_string(geo).map_err(|e| {
                PgWireTransportError::type_conversion(format!(
                    "Failed to serialize Geometry for binary: {}",
                    e
                ))
            })?;
            Ok(encode_jsonb_binary(&json_str))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn test_encode_bool_binary() {
        assert_eq!(encode_bool_binary(true), vec![0x01]);
        assert_eq!(encode_bool_binary(false), vec![0x00]);
    }

    #[test]
    fn test_encode_int4_binary() {
        assert_eq!(encode_int4_binary(0), vec![0x00, 0x00, 0x00, 0x00]);
        assert_eq!(encode_int4_binary(1), vec![0x00, 0x00, 0x00, 0x01]);
        assert_eq!(encode_int4_binary(256), vec![0x00, 0x00, 0x01, 0x00]);
        assert_eq!(encode_int4_binary(-1), vec![0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn test_encode_int8_binary() {
        assert_eq!(
            encode_int8_binary(0),
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            encode_int8_binary(1),
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]
        );
        assert_eq!(
            encode_int8_binary(-1),
            vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
        );
    }

    #[test]
    fn test_encode_float8_binary() {
        let bytes = encode_float8_binary(1.5);
        assert_eq!(bytes.len(), 8);
        // IEEE 754 double precision for 1.5
        assert_eq!(bytes, vec![0x3F, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_encode_text_binary() {
        assert_eq!(encode_text_binary("hello"), b"hello".to_vec());
        assert_eq!(encode_text_binary(""), Vec::<u8>::new());
    }

    #[test]
    fn test_encode_timestamp_binary() {
        // PostgreSQL epoch: 2000-01-01 00:00:00 UTC
        let pg_epoch = Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap();
        let storage_ts: StorageTimestamp = pg_epoch.into();
        let bytes = encode_timestamp_binary(&storage_ts);
        assert_eq!(bytes.len(), 8);
        // At PostgreSQL epoch, microseconds should be 0
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        // One second after PostgreSQL epoch
        let one_sec_after = Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 1).unwrap();
        let storage_ts: StorageTimestamp = one_sec_after.into();
        let bytes = encode_timestamp_binary(&storage_ts);
        // 1 second = 1,000,000 microseconds
        let expected: i64 = 1_000_000;
        assert_eq!(bytes, expected.to_be_bytes().to_vec());
    }

    #[test]
    fn test_encode_jsonb_binary() {
        let bytes = encode_jsonb_binary(r#"{"key": "value"}"#);
        assert_eq!(bytes[0], 0x01); // JSONB version
        assert_eq!(&bytes[1..], br#"{"key": "value"}"#);
    }

    #[test]
    fn test_encode_value_binary_boolean() {
        let value = PropertyValue::Boolean(true);
        let bytes = encode_value_binary(&value, &Type::BOOL).unwrap();
        assert_eq!(bytes, vec![0x01]);
    }

    #[test]
    fn test_encode_value_binary_integer() {
        let value = PropertyValue::Integer(42);
        let bytes = encode_value_binary(&value, &Type::INT4).unwrap();
        assert_eq!(bytes, encode_int4_binary(42));

        let bytes = encode_value_binary(&value, &Type::INT8).unwrap();
        assert_eq!(bytes, encode_int8_binary(42));
    }

    #[test]
    fn test_encode_value_binary_float() {
        let value = PropertyValue::Float(3.14);
        let bytes = encode_value_binary(&value, &Type::FLOAT8).unwrap();
        assert_eq!(bytes, encode_float8_binary(3.14));
    }

    #[test]
    fn test_encode_value_binary_string() {
        let value = PropertyValue::String("hello world".to_string());
        let bytes = encode_value_binary(&value, &Type::TEXT).unwrap();
        assert_eq!(bytes, b"hello world".to_vec());
    }

    #[test]
    fn test_encode_value_binary_date() {
        let timestamp = Utc.with_ymd_and_hms(2023, 11, 14, 12, 30, 0).unwrap();
        let value = PropertyValue::Date(timestamp.into());
        let bytes = encode_value_binary(&value, &Type::TIMESTAMPTZ).unwrap();
        assert_eq!(bytes.len(), 8);
        // Should be positive (after PG epoch 2000-01-01)
        let micros = i64::from_be_bytes(bytes.try_into().unwrap());
        assert!(micros > 0);
    }

    #[test]
    fn test_encode_value_binary_date_from_string() {
        let value = PropertyValue::String("2000-01-01T00:00:00Z".to_string());
        let bytes = encode_value_binary(&value, &Type::TIMESTAMPTZ).unwrap();
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }
}
