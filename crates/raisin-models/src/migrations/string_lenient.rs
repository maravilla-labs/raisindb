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

//! Lenient string deserializers for handling legacy data with type mismatches.
//!
//! Includes both legacy JSON-only versions and MessagePack-compatible versions.

use serde::{Deserialize, Deserializer};

/// Lenient deserializer for Option<String> fields that may contain:
/// - String values -> Some(String)
/// - boolean false -> None
/// - null -> None
/// - Any other type -> Try to convert to string
///
/// **LEGACY VERSION**: Uses serde_json::Value, only works with JSON
#[deprecated(note = "Use deserialize_optional_string_lenient_msgpack for MessagePack support")]
pub fn deserialize_optional_string_lenient<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    use serde_json::Value;

    let value: Value = Value::deserialize(deserializer)?;

    match value {
        Value::String(s) => Ok(Some(s)),
        Value::Bool(false) => Ok(None),
        Value::Null => Ok(None),
        Value::Bool(true) => Ok(Some("true".to_string())),
        Value::Number(n) => Ok(Some(n.to_string())),
        Value::Array(_) | Value::Object(_) => {
            Err(Error::custom("Cannot convert array/object to string"))
        }
    }
}

/// Lenient deserializer for required String fields that may contain:
/// - String values -> String
/// - boolean false -> "" (empty string)
/// - boolean true -> "true"
/// - null -> "" (empty string)
/// - numbers -> string representation
///
/// **LEGACY VERSION**: Uses serde_json::Value, only works with JSON
#[deprecated(note = "Use deserialize_string_lenient_msgpack for MessagePack support")]
pub fn deserialize_string_lenient<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    use serde_json::Value;

    let value: Value = Value::deserialize(deserializer)?;

    match value {
        Value::String(s) => Ok(s),
        Value::Bool(false) => Ok(String::new()),
        Value::Bool(true) => Ok("true".to_string()),
        Value::Null => Ok(String::new()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Array(_) | Value::Object(_) => {
            Err(Error::custom("Cannot convert array/object to string"))
        }
    }
}

// ============================================================================
// MessagePack-Compatible Lenient Deserializers
// ============================================================================

/// Visitor for lenient Option<String> deserialization
/// Handles both JSON and MessagePack formats
struct OptionalStringLenientVisitor;

impl<'de> serde::de::Visitor<'de> for OptionalStringLenientVisitor {
    type Value = Option<String>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a string, boolean, null, or number")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        // Boolean false -> None, true -> Some("true")
        Ok(if v { Some("true".to_string()) } else { None })
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Some(v.to_string()))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Some(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Some(v.to_string()))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Some(v.to_string()))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Some(v.to_string()))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(None)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(None)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

/// MessagePack-compatible lenient deserializer for Option<String> fields
///
/// Handles legacy data with type mismatches:
/// - String values -> Some(String)
/// - boolean false -> None
/// - boolean true -> Some("true")
/// - null/unit -> None
/// - numbers -> Some(string representation)
///
/// Works with both JSON and MessagePack serialization formats.
pub fn deserialize_optional_string_lenient_msgpack<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(OptionalStringLenientVisitor)
}

/// Visitor for lenient String deserialization
/// Handles both JSON and MessagePack formats
struct StringLenientVisitor;

impl<'de> serde::de::Visitor<'de> for StringLenientVisitor {
    type Value = String;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a string, boolean, null, or number")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        // Boolean false -> "", true -> "true"
        Ok(if v { "true".to_string() } else { String::new() })
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v.to_string())
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v)
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v.to_string())
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v.to_string())
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v.to_string())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(String::new())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(String::new())
    }
}

/// MessagePack-compatible lenient deserializer for required String fields
///
/// Handles legacy data with type mismatches:
/// - String values -> String
/// - boolean false -> "" (empty string)
/// - boolean true -> "true"
/// - null/unit -> "" (empty string)
/// - numbers -> string representation
///
/// Works with both JSON and MessagePack serialization formats.
pub fn deserialize_string_lenient_msgpack<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(StringLenientVisitor)
}
