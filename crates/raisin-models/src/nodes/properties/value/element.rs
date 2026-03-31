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

//! Element and Composite types with custom serde implementations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::property_value::PropertyValue;

/// Typed element within a composite.
///
/// Elements serialize as flat maps for human-readable formats (JSON, YAML):
/// ```json
/// {
///   "element_type": "launchpad:KanbanCard",
///   "uuid": "el-123",
///   "title": "My Task",
///   "note": "Important"
/// }
/// ```
///
/// The internal structure maintains a `content` HashMap, but serialization
/// flattens all fields into a single map for better ergonomics.
#[derive(Debug, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    pub uuid: String,
    pub element_type: String,
    pub content: HashMap<String, PropertyValue>,
}

impl serde::Serialize for Element {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        // Calculate size: element_type + uuid (if not empty) + content fields
        let size = 1 + if self.uuid.is_empty() { 0 } else { 1 } + self.content.len();
        let mut map = serializer.serialize_map(Some(size))?;

        // Always serialize element_type first
        map.serialize_entry("element_type", &self.element_type)?;

        // Serialize uuid only if not empty
        if !self.uuid.is_empty() {
            map.serialize_entry("uuid", &self.uuid)?;
        }

        // Flatten content fields
        for (key, value) in &self.content {
            map.serialize_entry(key, value)?;
        }

        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for Element {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{MapAccess, Visitor};
        use std::fmt;

        struct ElementVisitor;

        impl<'de> Visitor<'de> for ElementVisitor {
            type Value = Element;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map with element_type field")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Element, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut element_type: Option<String> = None;
                let mut uuid: Option<String> = None;
                let mut content = HashMap::new();
                // Track if we see a nested "content" key (old format)
                let mut nested_content: Option<PropertyValue> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "element_type" => {
                            if element_type.is_some() {
                                return Err(serde::de::Error::duplicate_field("element_type"));
                            }
                            element_type = Some(map.next_value()?);
                        }
                        "uuid" => {
                            if uuid.is_some() {
                                return Err(serde::de::Error::duplicate_field("uuid"));
                            }
                            uuid = Some(map.next_value()?);
                        }
                        "content" => {
                            // Could be old nested format or a field actually named "content"
                            // Store it and decide later based on total field count
                            nested_content = Some(map.next_value()?);
                        }
                        _ => {
                            // All other fields go into content
                            content.insert(key, map.next_value()?);
                        }
                    }
                }

                let element_type =
                    element_type.ok_or_else(|| serde::de::Error::missing_field("element_type"))?;
                let uuid = uuid.unwrap_or_default();

                // Handle nested "content" field from old format
                // If we have a nested_content and no other fields, unwrap it
                // If we have other fields alongside content, treat "content" as a regular field
                if let Some(nested) = nested_content {
                    if content.is_empty() {
                        // Old format: {"element_type": "...", "uuid": "...", "content": {...}}
                        // Unwrap the nested content
                        if let PropertyValue::Object(obj) = nested {
                            content = obj;
                        } else {
                            // Content is not an object, treat it as a field named "content"
                            content.insert("content".to_string(), nested);
                        }
                    } else {
                        // Mixed format: other fields exist, so "content" is a regular field
                        content.insert("content".to_string(), nested);
                    }
                }

                Ok(Element {
                    uuid,
                    element_type,
                    content,
                })
            }
        }

        deserializer.deserialize_map(ElementVisitor)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
pub struct Composite {
    pub uuid: String,
    pub items: Vec<Element>,
}
