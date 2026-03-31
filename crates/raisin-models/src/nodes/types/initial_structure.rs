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

// InitialStructure and InitialChild

use schemars::JsonSchema;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fmt;
use validator::Validate;

// Minimal stub for PropertyValue
type PropertyValue = serde_json::Value;

/// Initial node structure for workspace initialization.
/// Supports deserialization from either:
/// - Object: `{"children": [...], "properties": {...}}`
/// - Array (legacy): `[{child1}, {child2}]` - treated as children
#[derive(Debug, Serialize, Clone, PartialEq, JsonSchema, Validate)]
pub struct InitialNodeStructure {
    #[serde(default)]
    pub properties: Option<HashMap<String, PropertyValue>>,
    #[serde(default)]
    pub children: Option<Vec<InitialChild>>,
}

impl<'de> Deserialize<'de> for InitialNodeStructure {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct InitialNodeStructureVisitor;

        impl<'de> Visitor<'de> for InitialNodeStructureVisitor {
            type Value = InitialNodeStructure;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an object with children/properties or an array of children")
            }

            // Handle nil/unit values (MessagePack nil, JSON null)
            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(InitialNodeStructure {
                    properties: None,
                    children: None,
                })
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(InitialNodeStructure {
                    properties: None,
                    children: None,
                })
            }

            // Handle array format: [{child1}, {child2}] -> children
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut children = Vec::new();
                while let Some(child) = seq.next_element::<InitialChild>()? {
                    children.push(child);
                }
                Ok(InitialNodeStructure {
                    properties: None,
                    children: if children.is_empty() {
                        None
                    } else {
                        Some(children)
                    },
                })
            }

            // Handle object format: {"children": [...], "properties": {...}}
            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut properties = None;
                let mut children = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "properties" => {
                            // Let Option handle null values properly
                            properties = map.next_value()?;
                        }
                        "children" => {
                            // Let Option handle null values properly
                            children = map.next_value()?;
                        }
                        _ => {
                            // Ignore unknown fields
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                Ok(InitialNodeStructure {
                    properties,
                    children,
                })
            }
        }

        deserializer.deserialize_any(InitialNodeStructureVisitor)
    }
}

/// Initial child node definition.
/// Supports deserialization from multiple formats:
/// - Object: `{"name": "...", "node_type": "..."}`
/// - Tuple (legacy): `["name", "node_type"]` or `["node_type", "name"]` (auto-detected)
/// - Extended tuple: `["name", "node_type", archetype, properties, translations, children]`
#[derive(Debug, Serialize, Clone, PartialEq, JsonSchema, Validate)]
pub struct InitialChild {
    pub name: String,
    #[validate(regex(path = "*crate::nodes::types::utils::URL_FRIENDLY_NAME_REGEX"))]
    pub node_type: String,
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub properties: Option<HashMap<String, PropertyValue>>,
    #[serde(default)]
    pub translations: Option<HashMap<String, PropertyValue>>,
    #[serde(default)]
    pub children: Option<Vec<InitialChild>>,
}

impl<'de> Deserialize<'de> for InitialChild {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct InitialChildVisitor;

        impl<'de> Visitor<'de> for InitialChildVisitor {
            type Value = InitialChild;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter
                    .write_str("an object with name/node_type or a tuple [name, node_type, ...]")
            }

            // Handle nil/unit values (MessagePack nil, JSON null)
            // InitialChild requires name and node_type, so nil is an error
            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Err(de::Error::custom(
                    "InitialChild cannot be null - it requires name and node_type fields",
                ))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Err(de::Error::custom(
                    "InitialChild cannot be null - it requires name and node_type fields",
                ))
            }

            // Handle tuple/array format: ["name", "node_type"] or extended
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                // First two elements are name and node_type (order auto-detected)
                let first: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &"at least 2 elements"))?;
                let second: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &"at least 2 elements"))?;

                // Auto-detect which is name vs node_type based on ":" presence
                // node_type typically has format "namespace:Type" (e.g., "raisin:AclFolder")
                let (name, node_type) = if first.contains(':') && !second.contains(':') {
                    // First looks like node_type, second is name
                    (second, first)
                } else {
                    // Default: first is name, second is node_type
                    (first, second)
                };

                // Optional extended tuple fields: [name, node_type, archetype, properties, translations, children]
                let archetype: Option<String> = seq.next_element()?.flatten();
                let properties: Option<HashMap<String, PropertyValue>> =
                    seq.next_element()?.flatten();
                let translations: Option<HashMap<String, PropertyValue>> =
                    seq.next_element()?.flatten();
                let children: Option<Vec<InitialChild>> = seq.next_element()?.flatten();

                Ok(InitialChild {
                    name,
                    node_type,
                    archetype,
                    properties,
                    translations,
                    children,
                })
            }

            // Handle object format: {"name": "...", "node_type": "..."}
            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut name: Option<String> = None;
                let mut node_type: Option<String> = None;
                let mut archetype: Option<String> = None;
                let mut properties: Option<HashMap<String, PropertyValue>> = None;
                let mut translations: Option<HashMap<String, PropertyValue>> = None;
                let mut children: Option<Vec<InitialChild>> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "name" => name = map.next_value()?,
                        "node_type" => node_type = map.next_value()?,
                        "archetype" => archetype = map.next_value()?,
                        "properties" => properties = map.next_value()?,
                        "translations" => translations = map.next_value()?,
                        "children" => children = map.next_value()?,
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                Ok(InitialChild {
                    name: name.ok_or_else(|| de::Error::missing_field("name"))?,
                    node_type: node_type.ok_or_else(|| de::Error::missing_field("node_type"))?,
                    archetype,
                    properties,
                    translations,
                    children,
                })
            }
        }

        deserializer.deserialize_any(InitialChildVisitor)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct InitialArchetypeStructure {
    #[serde(default)]
    pub content: Option<Vec<HashMap<String, String>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserializes_initial_child_from_legacy_tuple_name_first() {
        let value = json!(["Users", "raisin:AclFolder"]);
        let child: InitialChild = serde_json::from_value(value).expect("should deserialize");

        assert_eq!(child.name, "Users");
        assert_eq!(child.node_type, "raisin:AclFolder");
    }

    #[test]
    fn deserializes_initial_child_from_legacy_tuple_type_first() {
        let value = json!(["raisin:AclFolder", "Groups"]);
        let child: InitialChild = serde_json::from_value(value).expect("should deserialize");

        assert_eq!(child.name, "Groups");
        assert_eq!(child.node_type, "raisin:AclFolder");
    }

    #[test]
    fn deserializes_initial_child_from_map() {
        let value = json!({
            "name": "Users",
            "node_type": "raisin:AclFolder",
            "archetype": "raisin:DefaultFolder"
        });
        let child: InitialChild = serde_json::from_value(value).expect("should deserialize");

        assert_eq!(child.name, "Users");
        assert_eq!(child.node_type, "raisin:AclFolder");
        assert_eq!(child.archetype.as_deref(), Some("raisin:DefaultFolder"));
    }

    #[test]
    fn deserializes_initial_child_from_legacy_extended_tuple() {
        let value = json!([
            "Users",
            "raisin:AclFolder",
            null,
            {"title": "Users"},
            null,
            [
                ["Admins", "raisin:AclFolder"]
            ]
        ]);

        let child: InitialChild =
            serde_json::from_value(value).expect("extended tuple should deserialize");

        assert_eq!(child.name, "Users");
        assert_eq!(child.node_type, "raisin:AclFolder");
        assert!(child.properties.is_some());
        let nested = child.children.expect("nested children expected");
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].name, "Admins");
    }

    #[test]
    fn deserializes_initial_node_structure_from_array() {
        let value = json!([
            {"name": "Users", "node_type": "raisin:AclFolder"},
            {"name": "Roles", "node_type": "raisin:AclFolder"}
        ]);

        let structure: InitialNodeStructure =
            serde_json::from_value(value).expect("array should deserialize");

        let children = structure.children.expect("children expected");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "Users");
        assert_eq!(children[1].name, "Roles");
    }

    #[test]
    fn deserializes_initial_node_structure_from_map() {
        let value = json!({
            "children": [
                {"name": "Users", "node_type": "raisin:AclFolder"}
            ],
            "properties": {
                "title": "Access Control"
            }
        });

        let structure: InitialNodeStructure =
            serde_json::from_value(value).expect("map should deserialize");

        assert!(structure.properties.is_some());
        let children = structure.children.expect("children expected");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "Users");
    }

    #[test]
    fn deserializes_initial_child_with_null_fields() {
        // Test that null values for optional fields are handled correctly
        let value = json!({
            "name": "Users",
            "node_type": "raisin:AclFolder",
            "archetype": null,
            "properties": null,
            "translations": null,
            "children": null
        });
        let child: InitialChild = serde_json::from_value(value).expect("should deserialize");

        assert_eq!(child.name, "Users");
        assert_eq!(child.node_type, "raisin:AclFolder");
        assert!(child.archetype.is_none());
        assert!(child.properties.is_none());
        assert!(child.translations.is_none());
        assert!(child.children.is_none());
    }

    #[test]
    fn deserializes_initial_node_structure_with_null_fields() {
        // Test that null values for optional fields are handled correctly
        let value = json!({
            "properties": null,
            "children": null
        });
        let structure: InitialNodeStructure =
            serde_json::from_value(value).expect("should deserialize");

        assert!(structure.properties.is_none());
        assert!(structure.children.is_none());
    }

    #[test]
    fn deserializes_nested_children_with_null_fields() {
        // Test nested children with null fields (matches stored MessagePack format)
        let value = json!({
            "properties": null,
            "children": [
                {
                    "name": "private",
                    "node_type": "raisin:ProfileData",
                    "archetype": null,
                    "properties": null,
                    "translations": null,
                    "children": null
                },
                {
                    "name": "public",
                    "node_type": "raisin:ProfileData",
                    "archetype": null,
                    "properties": null,
                    "translations": null,
                    "children": null
                }
            ]
        });
        let structure: InitialNodeStructure =
            serde_json::from_value(value).expect("should deserialize");

        assert!(structure.properties.is_none());
        let children = structure.children.expect("children expected");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "private");
        assert_eq!(children[1].name, "public");
        assert!(children[0].children.is_none());
        assert!(children[1].children.is_none());
    }

    #[test]
    fn messagepack_roundtrip_initial_node_structure() {
        // Test MessagePack serialization/deserialization round-trip
        // Using to_vec_named because custom deserializers expect named fields
        let structure = InitialNodeStructure {
            properties: Some([("title".to_string(), json!("Test"))].into_iter().collect()),
            children: Some(vec![
                InitialChild {
                    name: "users".to_string(),
                    node_type: "raisin:AclFolder".to_string(),
                    archetype: None,
                    properties: Some([("color".to_string(), json!("red"))].into_iter().collect()),
                    translations: None,
                    children: Some(vec![InitialChild {
                        name: "system".to_string(),
                        node_type: "raisin:AclFolder".to_string(),
                        archetype: None,
                        properties: None,
                        translations: None,
                        children: None,
                    }]),
                },
                InitialChild {
                    name: "roles".to_string(),
                    node_type: "raisin:AclFolder".to_string(),
                    archetype: None,
                    properties: None,
                    translations: None,
                    children: None, // No nested children
                },
            ]),
        };

        // Serialize to MessagePack with named fields (required for custom deserializers)
        let msgpack = rmp_serde::to_vec_named(&structure).expect("should serialize to MessagePack");

        // Deserialize from MessagePack
        let roundtrip: InitialNodeStructure =
            rmp_serde::from_slice(&msgpack).expect("should deserialize from MessagePack");

        assert_eq!(structure, roundtrip);
    }

    #[test]
    fn messagepack_roundtrip_initial_child_with_none_fields() {
        // Test that None fields roundtrip correctly through MessagePack
        // Using to_vec_named because custom deserializers expect named fields
        let child = InitialChild {
            name: "test".to_string(),
            node_type: "raisin:Folder".to_string(),
            archetype: None,
            properties: None,
            translations: None,
            children: None,
        };

        let msgpack = rmp_serde::to_vec_named(&child).expect("should serialize");
        let roundtrip: InitialChild = rmp_serde::from_slice(&msgpack).expect("should deserialize");

        assert_eq!(child, roundtrip);
    }

    #[test]
    fn messagepack_null_initial_node_structure() {
        // Test deserializing null/nil as InitialNodeStructure
        // This simulates what happens when Option<InitialNodeStructure> contains None
        // and is serialized as nil
        let nil_msgpack = rmp_serde::to_vec(&()).expect("should serialize nil");
        let result: InitialNodeStructure =
            rmp_serde::from_slice(&nil_msgpack).expect("should handle nil");

        assert!(result.properties.is_none());
        assert!(result.children.is_none());
    }
}
