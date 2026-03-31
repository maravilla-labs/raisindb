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

//! Storage-optimized timestamp type with format-aware serialization.
//!
//! `StorageTimestamp` stores timestamps as compact i64 nanoseconds in binary formats
//! (MessagePack/RocksDB) while returning RFC3339 strings in JSON API responses.

use chrono::{DateTime, TimeZone, Utc};
use schemars::JsonSchema;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// A timestamp optimized for storage efficiency.
///
/// - Binary formats (MessagePack): Serialized as i64 nanoseconds (~9 bytes)
/// - Human-readable formats (JSON): Serialized as RFC3339 string
///
/// Supports auto-detection of epoch precision (seconds, millis, micros, nanos)
/// when deserializing from integer values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StorageTimestamp(DateTime<Utc>);

impl StorageTimestamp {
    /// Creates a new StorageTimestamp from the current time.
    pub fn now() -> Self {
        StorageTimestamp(Utc::now())
    }

    /// Creates a StorageTimestamp from nanoseconds since Unix epoch.
    pub fn from_nanos(nanos: i64) -> Option<Self> {
        let secs = nanos / 1_000_000_000;
        let nsecs = (nanos % 1_000_000_000) as u32;
        Utc.timestamp_opt(secs, nsecs)
            .single()
            .map(StorageTimestamp)
    }

    /// Returns the timestamp as nanoseconds since Unix epoch.
    pub fn timestamp_nanos(&self) -> i64 {
        self.0.timestamp_nanos_opt().unwrap_or(i64::MAX)
    }

    /// Returns the inner DateTime<Utc>.
    pub fn into_inner(self) -> DateTime<Utc> {
        self.0
    }

    /// Returns a reference to the inner DateTime<Utc>.
    pub fn as_datetime(&self) -> &DateTime<Utc> {
        &self.0
    }

    /// Auto-detects epoch precision and creates a StorageTimestamp.
    ///
    /// Detection thresholds (all correspond to approximately year 5138):
    /// - `< 100_000_000_000` → seconds
    /// - `< 100_000_000_000_000` → milliseconds
    /// - `< 100_000_000_000_000_000` → microseconds
    /// - Otherwise → nanoseconds
    pub fn from_epoch_auto_detect(value: i64) -> Option<Self> {
        const THRESHOLD_SECONDS: i64 = 100_000_000_000; // ~5138 AD
        const THRESHOLD_MILLIS: i64 = 100_000_000_000_000; // ~5138 AD
        const THRESHOLD_MICROS: i64 = 100_000_000_000_000_000; // ~5138 AD

        let (secs, nanos) = if value < THRESHOLD_SECONDS {
            (value, 0u32)
        } else if value < THRESHOLD_MILLIS {
            (value / 1000, ((value % 1000) * 1_000_000) as u32)
        } else if value < THRESHOLD_MICROS {
            (value / 1_000_000, ((value % 1_000_000) * 1000) as u32)
        } else {
            (value / 1_000_000_000, (value % 1_000_000_000) as u32)
        };

        Utc.timestamp_opt(secs, nanos)
            .single()
            .map(StorageTimestamp)
    }
}

impl From<DateTime<Utc>> for StorageTimestamp {
    fn from(dt: DateTime<Utc>) -> Self {
        StorageTimestamp(dt)
    }
}

impl From<StorageTimestamp> for DateTime<Utc> {
    fn from(ts: StorageTimestamp) -> Self {
        ts.0
    }
}

impl std::ops::Deref for StorageTimestamp {
    type Target = DateTime<Utc>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for StorageTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.to_rfc3339())
    }
}

impl Serialize for StorageTimestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            // JSON: RFC3339 string "2024-11-28T16:00:00.123456789Z"
            serializer.serialize_str(&self.0.to_rfc3339())
        } else {
            // MessagePack: Wrap i64 nanoseconds in a newtype to distinguish from plain numbers
            // This ensures unambiguous deserialization with PropertyValue's untagged enum
            // Format: [nanoseconds] (single-element tuple/array)
            use serde::ser::SerializeTuple;
            let mut tuple = serializer.serialize_tuple(1)?;
            tuple.serialize_element(&self.timestamp_nanos())?;
            tuple.end()
        }
    }
}

impl<'de> Deserialize<'de> for StorageTimestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use deserialize_any to handle all formats:
        // - JSON: RFC3339 strings only (integers rejected to avoid ambiguity with Number)
        // - MessagePack: Single-element tuple [nanoseconds] (distinguishable from plain numbers)
        deserializer.deserialize_any(StorageTimestampVisitor)
    }
}

struct StorageTimestampVisitor;

impl<'de> Visitor<'de> for StorageTimestampVisitor {
    type Value = StorageTimestamp;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an RFC3339 datetime string or a [nanoseconds] tuple")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        // JSON: RFC3339 strings
        DateTime::parse_from_rfc3339(value)
            .map(|dt| StorageTimestamp(dt.with_timezone(&Utc)))
            .map_err(de::Error::custom)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        // MessagePack: Single-element tuple [nanoseconds]
        let nanos: i64 = seq
            .next_element()?
            .ok_or_else(|| de::Error::custom("expected nanoseconds in tuple"))?;

        // Verify no extra elements
        if seq.next_element::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom("expected single-element tuple"));
        }

        StorageTimestamp::from_nanos(nanos)
            .ok_or_else(|| de::Error::custom("Invalid nanosecond timestamp"))
    }

    // Note: visit_i64 is intentionally NOT implemented.
    // This ensures plain integers don't match StorageTimestamp, allowing
    // PropertyValue's untagged enum to correctly fall through to Number.
}

impl JsonSchema for StorageTimestamp {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("StorageTimestamp")
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        // In JSON, we represent as RFC3339 string
        String::json_schema(generator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone, Timelike};

    #[test]
    fn test_json_serialization_rfc3339() {
        let dt = Utc.with_ymd_and_hms(2024, 11, 28, 16, 30, 45).unwrap();
        let ts = StorageTimestamp::from(dt);

        let json = serde_json::to_string(&ts).expect("should serialize");
        assert_eq!(json, r#""2024-11-28T16:30:45+00:00""#);
    }

    #[test]
    fn test_json_deserialization_rfc3339() {
        let json = r#""2024-11-28T16:30:45Z""#;
        let ts: StorageTimestamp = serde_json::from_str(json).expect("should deserialize");

        let expected = Utc.with_ymd_and_hms(2024, 11, 28, 16, 30, 45).unwrap();
        assert_eq!(ts.into_inner(), expected);
    }

    #[test]
    fn test_json_rejects_integers() {
        // JSON deserialization should reject integers to avoid ambiguity
        // with PropertyValue's Number variant (untagged enum)
        let json = "1732813845";
        let result: Result<StorageTimestamp, _> = serde_json::from_str(json);
        assert!(result.is_err(), "Should reject integer in JSON");

        // Small numbers like 8 should also be rejected
        let json = "8";
        let result: Result<StorageTimestamp, _> = serde_json::from_str(json);
        assert!(result.is_err(), "Should reject small integer in JSON");

        // Large numbers should also be rejected
        let json = "23433453453453";
        let result: Result<StorageTimestamp, _> = serde_json::from_str(json);
        assert!(result.is_err(), "Should reject large integer in JSON");
    }

    #[test]
    fn test_json_accepts_rfc3339_with_subsecond() {
        // Should accept RFC3339 with subsecond precision
        let json = r#""2024-11-28T17:10:45.123456789Z""#;
        let ts: StorageTimestamp = serde_json::from_str(json).expect("should deserialize");

        assert_eq!(ts.timestamp(), 1732813845);
        assert_eq!(ts.timestamp_subsec_nanos(), 123456789);
    }

    #[test]
    fn test_messagepack_serialization_tuple_nanos() {
        let dt = Utc.with_ymd_and_hms(2024, 11, 28, 16, 30, 45).unwrap();
        let ts = StorageTimestamp::from(dt);

        let bytes = rmp_serde::to_vec(&ts).expect("should serialize");

        // MessagePack serializes as single-element tuple [nanoseconds] for unambiguous
        // deserialization with PropertyValue's untagged enum
        // Format: 1 byte tuple marker + 9 bytes i64 = 10 bytes (still much smaller than RFC3339)
        assert!(
            bytes.len() <= 10,
            "MessagePack should be compact: {} bytes",
            bytes.len()
        );

        // Deserialize and verify
        let deserialized: StorageTimestamp =
            rmp_serde::from_slice(&bytes).expect("should deserialize");
        assert_eq!(deserialized, ts);
    }

    #[test]
    fn test_messagepack_roundtrip_with_nanos() {
        let dt = Utc
            .with_ymd_and_hms(2024, 11, 28, 16, 30, 45)
            .unwrap()
            .with_nanosecond(123456789)
            .unwrap();
        let ts = StorageTimestamp::from(dt);

        let bytes = rmp_serde::to_vec(&ts).expect("should serialize");
        let deserialized: StorageTimestamp =
            rmp_serde::from_slice(&bytes).expect("should deserialize");

        assert_eq!(deserialized.timestamp_nanos(), ts.timestamp_nanos());
        assert_eq!(deserialized.timestamp_subsec_nanos(), 123456789);
    }

    #[test]
    fn test_now() {
        let before = Utc::now();
        let ts = StorageTimestamp::now();
        let after = Utc::now();

        assert!(ts.into_inner() >= before);
        assert!(ts.into_inner() <= after);
    }

    #[test]
    fn test_from_nanos() {
        let nanos = 1732813845_123456789_i64;
        let ts = StorageTimestamp::from_nanos(nanos).expect("should create from nanos");

        assert_eq!(ts.timestamp(), 1732813845);
        assert_eq!(ts.timestamp_subsec_nanos(), 123456789);
    }

    #[test]
    fn test_ordering() {
        let ts1 = StorageTimestamp::from_nanos(1000000000_000000000).unwrap();
        let ts2 = StorageTimestamp::from_nanos(2000000000_000000000).unwrap();

        assert!(ts1 < ts2);
        assert!(ts2 > ts1);
    }

    #[test]
    fn test_display() {
        let dt = Utc.with_ymd_and_hms(2024, 11, 28, 16, 30, 45).unwrap();
        let ts = StorageTimestamp::from(dt);

        let display = format!("{}", ts);
        assert!(display.contains("2024-11-28"));
        assert!(display.contains("16:30:45"));
    }

    #[test]
    fn test_deref() {
        let dt = Utc.with_ymd_and_hms(2024, 11, 28, 16, 30, 45).unwrap();
        let ts = StorageTimestamp::from(dt);

        // Can use DateTime methods directly via Deref
        assert_eq!(ts.year(), 2024);
        assert_eq!(ts.month(), 11);
        assert_eq!(ts.day(), 28);
    }
}
