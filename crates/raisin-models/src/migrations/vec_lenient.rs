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

//! Lenient Vec deserializers that handle SQL NULL values gracefully.

use serde::{Deserialize, Deserializer};

/// Visitor for lenient Vec<String> deserialization
/// Handles both JSON and MessagePack formats, treating null as empty vec
struct VecStringLenientVisitor;

impl<'de> serde::de::Visitor<'de> for VecStringLenientVisitor {
    type Value = Vec<String>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a sequence of strings or null")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Vec::new())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Vec::new())
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut vec = Vec::new();
        while let Some(s) = seq.next_element::<String>()? {
            vec.push(s);
        }
        Ok(vec)
    }
}

/// MessagePack-compatible lenient deserializer for Vec<String> fields
///
/// Handles SQL NULL values gracefully:
/// - null/unit -> empty Vec<String>
/// - sequence -> Vec<String> (normal case)
///
/// This is needed because SQL returns NULL for empty array columns,
/// and `#[serde(default)]` only works when the field is missing from JSON,
/// not when it's present with an explicit null value.
pub fn deserialize_vec_string_lenient<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(VecStringLenientVisitor)
}

/// Generic lenient deserializer for Vec<T> fields
///
/// Handles SQL NULL values gracefully:
/// - null/unit -> empty Vec<T>
/// - sequence -> Vec<T> (normal case)
///
/// Uses Option<Vec<T>> internally to handle null, then unwraps to empty vec.
pub fn deserialize_vec_lenient<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    // Use Option<Vec<T>> to handle null, then convert None to empty vec
    let opt: Option<Vec<T>> = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}
