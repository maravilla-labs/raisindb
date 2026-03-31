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

//! Lenient DateTime deserializers for handling legacy data formats.
//!
//! Supports RFC 3339 strings, Unix timestamps, and sequence formats.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserializer;

/// Visitor for lenient Option<DateTime<Utc>> deserialization
/// Handles both JSON and MessagePack formats, including legacy sequence format
struct OptionalDateTimeLenientVisitor;

impl<'de> serde::de::Visitor<'de> for OptionalDateTimeLenientVisitor {
    type Value = Option<DateTime<Utc>>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an RFC 3339 formatted date and time string, a sequence, or null")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        // Try parsing as RFC 3339
        DateTime::parse_from_rfc3339(v)
            .map(|dt| Some(dt.with_timezone(&Utc)))
            .map_err(|e| E::custom(format!("invalid datetime string '{}': {}", v, e)))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(&v)
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

    /// Handle sequence format: [year, month, day, hour, minute, second, nanosecond]
    /// or [secs, nsecs] timestamp format
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        use serde::de::Error;

        // Collect all elements
        let mut elements: Vec<i64> = Vec::new();
        while let Some(elem) = seq.next_element::<i64>()? {
            elements.push(elem);
        }

        match elements.len() {
            // [nanos] format (Unix timestamp in nanoseconds)
            // This is the current RaisinDB storage format
            1 => {
                let nanos = elements[0];
                let secs = nanos / 1_000_000_000;
                let nsecs = (nanos % 1_000_000_000) as u32;
                DateTime::from_timestamp(secs, nsecs)
                    .map(Some)
                    .ok_or_else(|| Error::custom(format!("invalid timestamp: [{}]", nanos)))
            }
            // [secs, nsecs] format (Unix timestamp with nanoseconds)
            2 => {
                let secs = elements[0];
                let nsecs = elements[1] as u32;
                DateTime::from_timestamp(secs, nsecs)
                    .map(Some)
                    .ok_or_else(|| {
                        Error::custom(format!("invalid timestamp: [{}, {}]", secs, nsecs))
                    })
            }
            // [year, month, day, hour, minute, second] format
            6 => {
                let year = elements[0] as i32;
                let month = elements[1] as u32;
                let day = elements[2] as u32;
                let hour = elements[3] as u32;
                let minute = elements[4] as u32;
                let second = elements[5] as u32;

                NaiveDateTime::new(
                    chrono::NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| {
                        Error::custom(format!("invalid date: {}-{}-{}", year, month, day))
                    })?,
                    chrono::NaiveTime::from_hms_opt(hour, minute, second).ok_or_else(|| {
                        Error::custom(format!("invalid time: {}:{}:{}", hour, minute, second))
                    })?,
                )
                .and_utc()
                .pipe(|dt| Ok(Some(dt)))
            }
            // [year, month, day, hour, minute, second, nanosecond] format
            7 => {
                let year = elements[0] as i32;
                let month = elements[1] as u32;
                let day = elements[2] as u32;
                let hour = elements[3] as u32;
                let minute = elements[4] as u32;
                let second = elements[5] as u32;
                let nano = elements[6] as u32;

                NaiveDateTime::new(
                    chrono::NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| {
                        Error::custom(format!("invalid date: {}-{}-{}", year, month, day))
                    })?,
                    chrono::NaiveTime::from_hms_nano_opt(hour, minute, second, nano).ok_or_else(
                        || {
                            Error::custom(format!(
                                "invalid time: {}:{}:{}.{}",
                                hour, minute, second, nano
                            ))
                        },
                    )?,
                )
                .and_utc()
                .pipe(|dt| Ok(Some(dt)))
            }
            n => Err(Error::custom(format!(
                "invalid datetime sequence length: {} (expected 1, 2, 6, or 7)",
                n
            ))),
        }
    }
}

/// Helper trait for pipe syntax
trait Pipe: Sized {
    fn pipe<F, R>(self, f: F) -> R
    where
        F: FnOnce(Self) -> R,
    {
        f(self)
    }
}

impl<T> Pipe for T {}

/// MessagePack-compatible lenient deserializer for Option<DateTime<Utc>> fields
///
/// Handles legacy data with different formats:
/// - RFC 3339 strings -> Some(DateTime)
/// - Sequence [secs, nsecs] -> Some(DateTime) (Unix timestamp)
/// - Sequence [year, month, day, hour, minute, second] -> Some(DateTime)
/// - Sequence [year, month, day, hour, minute, second, nanosecond] -> Some(DateTime)
/// - null/unit -> None
///
/// Works with both JSON and MessagePack serialization formats.
pub fn deserialize_optional_datetime_lenient_msgpack<'de, D>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(OptionalDateTimeLenientVisitor)
}
