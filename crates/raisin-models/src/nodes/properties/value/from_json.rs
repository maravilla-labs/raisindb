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

//! The ONE `serde_json::Value` → [`PropertyValue`] conversion for node writes.
//!
//! # Why this exists
//!
//! SQL DML, the WebSocket node handlers, the function callbacks and the AI job
//! handlers each used to carry a hand-rolled copy of this conversion, and the
//! copies drifted: one knew Reference, one knew Element, one knew Geometry, and
//! **none knew `Resource`**. Once declared property types were enforced, that
//! drift became a hard failure — `UPDATE 'assets' SET properties = properties ||
//! $1::jsonb` round-trips the stored `file` through JSON, the copy handed it back
//! as `Object`, and the write was refused with
//!
//! ```text
//! Property 'file' on NodeType 'raisin:Asset' is declared Resource but the value is Object
//! ```
//!
//! for a patch that never mentioned `file`.
//!
//! # What it does, and the one thing it deliberately does not
//!
//! OBJECT shapes are classified in the canonical ladder's order (Reference, Url,
//! Resource, Composite, Element, Geometry, then Object), delegating to each
//! type's own deserializer where it has one, so "is this a Resource?" is answered
//! by the same code that reads the stored blob.
//!
//! SCALARS are NOT sent through the canonical `#[serde(untagged)]` ladder. That
//! ladder tries `Date` and `Decimal` before `String`, so `"10"` becomes
//! `Decimal(10)`, `"1e3"` becomes `Decimal(1000)` and any RFC3339 string becomes
//! `Date` — measured. A string stays a `String` here; declared `Date`/`Decimal`
//! properties are coerced by the validator's declared-type passes, which know
//! what the property actually is. Arrays likewise stay `Array` (never `Vector`).

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::{Composite, Element, GeoJson, PropertyValue, RaisinReference, RaisinUrl, Resource};

impl PropertyValue {
    /// Convert a JSON value into a `PropertyValue`, recognising every domain
    /// object shape while leaving strings and arrays structurally as they are.
    ///
    /// See the module documentation for why scalars are not interpreted.
    pub fn from_json(value: &Value) -> PropertyValue {
        match value {
            Value::Null => PropertyValue::Null,
            Value::Bool(b) => PropertyValue::Boolean(*b),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    PropertyValue::Integer(i)
                } else {
                    PropertyValue::Float(n.as_f64().unwrap_or(0.0))
                }
            }
            Value::String(s) => PropertyValue::String(s.clone()),
            Value::Array(items) => {
                PropertyValue::Array(items.iter().map(PropertyValue::from_json).collect())
            }
            Value::Object(map) => object_from_json(value, map),
        }
    }
}

fn object_from_json(value: &Value, map: &Map<String, Value>) -> PropertyValue {
    if let Some(reference) = reference_from_json(map) {
        return PropertyValue::Reference(reference);
    }

    // Each typed attempt is gated on a key the type cannot exist without, so an
    // ordinary object does not pay for a clone and a failed deserialization.
    if map.contains_key("raisin:url") {
        if let Ok(url) = serde_json::from_value::<RaisinUrl>(value.clone()) {
            return PropertyValue::Url(url);
        }
    }
    if map.contains_key("uuid") && map.contains_key("created_at") {
        if let Ok(resource) = serde_json::from_value::<Resource>(value.clone()) {
            return PropertyValue::Resource(resource);
        }
    }
    if let Some(composite) = composite_from_json(map) {
        return PropertyValue::Composite(composite);
    }
    if let Some(element) = element_from_json(map) {
        return PropertyValue::Element(element);
    }
    if map.contains_key("type") {
        // Same deserializer the canonical ladder runs, so geometry detection
        // cannot drift; a malformed geometry-ish object falls through to Object.
        if let Ok(geometry) = serde_json::from_value::<GeoJson>(value.clone()) {
            return PropertyValue::Geometry(geometry);
        }
    }

    PropertyValue::Object(plain_map(map))
}

/// `{"raisin:ref": ..., "raisin:workspace"?: ..., "raisin:path"?: ...}`.
///
/// The workspace is optional: an empty workspace means "the referencing node's
/// own workspace", which the reference resolvers and the reference index both
/// honour, and function code has always been allowed to omit it.
fn reference_from_json(map: &Map<String, Value>) -> Option<RaisinReference> {
    let id = map.get("raisin:ref")?.as_str()?;
    let text = |key: &str| {
        map.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Some(RaisinReference {
        id: id.to_string(),
        workspace: text("raisin:workspace"),
        path: text("raisin:path"),
    })
}

/// A flat map carrying `element_type`, mirroring `Element`'s own deserializer —
/// including its legacy `{element_type, uuid, content: {...}}` nesting — but with
/// field values converted by [`PropertyValue::from_json`].
fn element_from_json(map: &Map<String, Value>) -> Option<Element> {
    let element_type = map.get("element_type")?.as_str()?.to_string();
    let uuid = map
        .get("uuid")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let mut content: HashMap<String, PropertyValue> = map
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "element_type" | "uuid" | "content"))
        .map(|(k, v)| (k.clone(), PropertyValue::from_json(v)))
        .collect();

    if let Some(nested) = map.get("content") {
        match (content.is_empty(), nested) {
            (true, Value::Object(inner)) => content = plain_map(inner),
            _ => {
                content.insert("content".to_string(), PropertyValue::from_json(nested));
            }
        }
    }

    Some(Element {
        uuid,
        element_type,
        content,
    })
}

/// `{"uuid": ..., "items": [<element>, ...]}` and nothing else.
///
/// Stricter than `Composite`'s derived deserializer, which ignores unknown keys:
/// an object that merely HAS `uuid` and `items` alongside other fields is a bag,
/// and classifying it as a Composite would silently drop those fields.
fn composite_from_json(map: &Map<String, Value>) -> Option<Composite> {
    if map.len() != 2 {
        return None;
    }
    let uuid = map.get("uuid")?.as_str()?.to_string();
    let items = map
        .get("items")?
        .as_array()?
        .iter()
        .map(|item| element_from_json(item.as_object()?))
        .collect::<Option<Vec<_>>>()?;
    Some(Composite { uuid, items })
}

fn plain_map(map: &Map<String, Value>) -> HashMap<String, PropertyValue> {
    map.iter()
        .map(|(k, v)| (k.clone(), PropertyValue::from_json(v)))
        .collect()
}

#[cfg(test)]
#[path = "from_json_tests.rs"]
mod tests;
