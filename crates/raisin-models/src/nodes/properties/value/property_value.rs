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

//! The core `PropertyValue` enum and `DateTimeTimestamp` type alias.

use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::nodes::properties::utils::{deserialize_raisin_reference, deserialize_raisin_url};
use crate::timestamp::StorageTimestamp;

use super::domain_types::{RaisinReference, RaisinUrl, Resource};
use super::element::{Composite, Element};
use super::geojson::GeoJson;

/// Timestamp type optimized for storage efficiency.
///
/// - Binary formats (MessagePack): Serialized as i64 nanoseconds (~9 bytes)
/// - Human-readable formats (JSON): Serialized as RFC3339 string
///
/// Supports auto-detection of epoch precision (seconds, millis, micros, nanos)
/// when deserializing from integer values.
pub type DateTimeTimestamp = StorageTimestamp;

/// The core value type for node properties.
///
/// ## Primitive Types
/// - `Null` - Explicit null value
/// - `Boolean` - true/false
/// - `Integer` - Exact 64-bit integers (no precision loss)
/// - `Float` - IEEE 754 double-precision floating point
/// - `Decimal` - 128-bit decimal for financial/exact calculations
/// - `String` - UTF-8 text
/// - `Date` - RFC3339 timestamps
///
/// ## Domain-Specific Types
/// - `Reference` - Cross-node references (raisin:ref pattern)
/// - `Url` - Rich URL with metadata (raisin:url pattern)
/// - `Resource` - File/media attachments
/// - `Composite` - Structured composite types
/// - `Element` - Typed elements within composites
///
/// ## Collection Types
/// - `Vector` - f32 arrays for embeddings/similarity search
/// - `Geometry` - GeoJSON geometries for geospatial queries
/// - `Array` - Heterogeneous arrays
/// - `Object` - Key-value maps
///
/// ## Deserialization Order
/// Order matters for `#[serde(untagged)]`! Variants are tried in order:
/// 1. Null, Boolean - JSON primitives
/// 2. Integer - JSON integers (no decimal point)
/// 3. Float - JSON numbers with decimal
/// 4. Date - RFC3339 strings (tuple in MessagePack)
/// 5. Decimal - String-encoded decimals "123.456"
/// 6. String - Plain strings
/// 7. Reference, Url - Objects with raisin:* keys
/// 8. Resource, Composite, Element - Domain objects
/// 9. Geometry - Objects with "type": "Point|LineString|Polygon|..."
/// 10. Vector, Array - Arrays
/// 11. Object - Fallback for any object
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(untagged)]
pub enum PropertyValue {
    // === PRIMITIVES (order critical for untagged deserialization) ===
    /// Explicit null value
    Null,

    /// Boolean true/false
    Boolean(bool),

    /// Exact 64-bit integer (no precision loss for large numbers)
    Integer(i64),

    /// IEEE 754 double-precision floating point
    Float(f64),

    /// RFC3339 timestamp (tuple format in MessagePack for disambiguation)
    Date(DateTimeTimestamp),

    /// 128-bit decimal for financial/exact calculations
    /// Serialized as string "123.456789" to preserve precision
    Decimal(Decimal),

    /// UTF-8 string
    String(String),

    // === DOMAIN-SPECIFIC TYPES (detected by raisin:* keys) ===
    /// Cross-node reference with workspace and path context
    #[serde(deserialize_with = "deserialize_raisin_reference")]
    Reference(RaisinReference),

    /// Rich URL with optional metadata (title, image, embed, etc.)
    #[serde(deserialize_with = "deserialize_raisin_url")]
    Url(RaisinUrl),

    /// File or media resource attachment
    Resource(Resource),

    /// Structured composite containing multiple elements
    Composite(Composite),

    /// Typed element within a composite
    Element(Element),

    // === COLLECTIONS ===
    /// Vector embedding (array of f32 values)
    /// Used for vector similarity search with pgvector-compatible queries
    Vector(Vec<f32>),

    /// GeoJSON geometry (Point, LineString, Polygon, etc.)
    /// Used for geospatial queries with PostGIS-compatible ST_* functions
    Geometry(GeoJson),

    /// Heterogeneous array of property values
    Array(Vec<PropertyValue>),

    /// Key-value object (fallback for unrecognized objects)
    Object(HashMap<String, PropertyValue>),
}
