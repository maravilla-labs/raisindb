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

//! What a value becomes when it is rendered to JSON and read back.
//!
//! Every SQL value that travels as a JSON document — a projected `properties`
//! map, a RESOLVE() result — is `PropertyValue::from_json(&to_value(v))` by
//! definition: that is what the JSON round trip the projection used to run
//! returns. [`PropertyValue::into_json_round_trip`] returns EXACTLY that,
//! without serializing the parts whose answer is provable (plan Phase 13d):
//!
//! - scalars come back as themselves (a non-finite float as `null`); a vector
//!   as an array of floats;
//! - a reference comes back as itself (`from_json` reads the three keys it
//!   writes);
//! - an object, an element and a composite come back as themselves with their
//!   children round-tripped — when no key in them would make `from_json`
//!   classify the rendering differently (`raisin:ref`, `raisin:url`, `uuid`,
//!   `element_type`, `type`, …);
//! - everything else (dates, decimals, URLs, resources, geometries, and any
//!   container a classifying key makes ambiguous) takes the real round trip —
//!   for that subtree only.
//!
//! The equivalence test pins the fast answer to the round trip.

use super::PropertyValue;
use std::collections::HashMap;

/// Keys `from_json` classifies a plain object by.
const OBJECT_CLASSIFIERS: &[&str] = &["raisin:ref", "raisin:url", "uuid", "element_type", "type"];

/// Content keys that make an element's rendering ambiguous: they collide
/// with its own `element_type` / `uuid`, are read by another shape first
/// (reference, URL, resource), or are the legacy `content` nesting.
const ELEMENT_AMBIGUOUS: &[&str] = &[
    "raisin:ref",
    "raisin:url",
    "created_at",
    "items",
    "uuid",
    "element_type",
    "content",
];

impl PropertyValue {
    /// `PropertyValue::from_json(&serde_json::to_value(&self))`, without the
    /// serialization where its answer is known. See the module docs.
    pub fn into_json_round_trip(mut self) -> PropertyValue {
        self.json_round_trip_in_place();
        self
    }

    /// [`Self::into_json_round_trip`] in place: a container whose answer is
    /// itself keeps its allocation (no map is rebuilt or re-hashed).
    pub fn json_round_trip_in_place(&mut self) {
        match self {
            PropertyValue::Null
            | PropertyValue::Boolean(_)
            | PropertyValue::Integer(_)
            | PropertyValue::String(_)
            | PropertyValue::Reference(_) => {}
            PropertyValue::Float(f) => {
                if !f.is_finite() {
                    *self = PropertyValue::Null;
                }
            }
            PropertyValue::Vector(floats) => {
                *self = PropertyValue::Array(
                    floats
                        .iter()
                        .map(|f| {
                            if f.is_finite() {
                                PropertyValue::Float(f64::from(*f))
                            } else {
                                PropertyValue::Null
                            }
                        })
                        .collect(),
                );
            }
            PropertyValue::Array(items) => items
                .iter_mut()
                .for_each(PropertyValue::json_round_trip_in_place),
            PropertyValue::Object(map) if !has_any(map, OBJECT_CLASSIFIERS) => map
                .values_mut()
                .for_each(PropertyValue::json_round_trip_in_place),
            PropertyValue::Element(element) if !has_any(&element.content, ELEMENT_AMBIGUOUS) => {
                element
                    .content
                    .values_mut()
                    .for_each(PropertyValue::json_round_trip_in_place)
            }
            PropertyValue::Composite(composite)
                if composite
                    .items
                    .iter()
                    .all(|item| !has_any(&item.content, &["uuid", "element_type", "content"])) =>
            {
                composite
                    .items
                    .iter_mut()
                    .flat_map(|item| item.content.values_mut())
                    .for_each(PropertyValue::json_round_trip_in_place)
            }
            other => *other = slow(other),
        }
    }
}

fn has_any(map: &HashMap<String, PropertyValue>, keys: &[&str]) -> bool {
    keys.iter().any(|k| map.contains_key(*k))
}

/// The real round trip.
fn slow(value: &PropertyValue) -> PropertyValue {
    serde_json::to_value(value)
        .map(|json| PropertyValue::from_json(&json))
        .unwrap_or(PropertyValue::Null)
}

#[cfg(test)]
#[path = "json_round_trip_tests.rs"]
mod tests;
