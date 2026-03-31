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

//! Field type definitions and schema for element fields.
//!
//! This module defines the `FieldTypeSchema` and `FieldSchema` enums, which represent the configuration and schema for all element field types in RaisinDB.
//!
//! # Schemars 1.0 Migration
//!
//! - All schema types are now imported from the root of `schemars`.
//! - Use the new `json_schema!` macro for manual schema construction.
//! - See the migration guide for more details.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::fields::base_field::FieldTypeSchema;
use super::fields::date_field_config::DateFieldConfig;
use super::fields::listing_field_config::ListingFieldConfig;
use super::fields::media_field_config::MediaFieldConfig;
use super::fields::number_field_config::NumberFieldConfig;
use super::fields::options_field_config::OptionsFieldConfig;
use super::fields::reference_field_config::ReferenceFieldConfig;
use super::fields::rich_text_field_config::RichTextFieldConfig;
use super::fields::tag_field_config::TagFieldConfig;
use super::fields::text_field_config::TextFieldConfig;
use crate::nodes::types::element::fields::layout::LayoutNode;

/// Represents the schema for a field in an element type.
///
/// Each variant corresponds to a different field type, with its own configuration struct.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(tag = "$type")]
pub enum FieldSchema {
    /// A simple text field.
    TextField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<TextFieldConfig>,
    },
    /// A rich text field supporting formatting.
    RichTextField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<RichTextFieldConfig>,
    },
    /// A numeric field (integer or float).
    NumberField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<NumberFieldConfig>,
    },
    /// A date/time field.
    DateField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<DateFieldConfig>,
    },
    /// A location field (not yet implemented).
    LocationField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
    },
    /// A boolean field.
    BooleanField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
    },
    /// A media field (image, video, etc.).
    MediaField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<MediaFieldConfig>,
    },
    /// A reference to another entry.
    ReferenceField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<ReferenceFieldConfig>,
    },
    /// A tag field for categorization.
    TagField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<TagFieldConfig>,
    },
    /// An options field (dropdown, radio, etc.).
    OptionsField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<OptionsFieldConfig>,
    },
    /// A JSON object field.
    JsonObjectField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
    },
    /// A composite field containing multiple subfields.
    CompositeField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default)]
        fields: Vec<FieldSchema>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout: Option<Vec<LayoutNode>>,
    },
    /// A section field grouping other fields.
    SectionField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default)]
        allowed_element_types: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        render_as: Option<String>,
    },
    /// An element field referencing an element type.
    ElementField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        element_type: String,
    },
    /// A listing field for referencing multiple entries.
    ListingField {
        #[serde(flatten, default)]
        base: FieldTypeSchema,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<ListingFieldConfig>,
    },
}

/// Trait for extracting the base name from a field schema.
pub trait FieldSchemaBase {
    fn base_name(&self) -> &String;
}

impl FieldSchemaBase for FieldSchema {
    fn base_name(&self) -> &String {
        match self {
            FieldSchema::TextField { base, .. }
            | FieldSchema::RichTextField { base, .. }
            | FieldSchema::NumberField { base, .. }
            | FieldSchema::DateField { base, .. }
            | FieldSchema::LocationField { base, .. }
            | FieldSchema::BooleanField { base, .. }
            | FieldSchema::MediaField { base, .. }
            | FieldSchema::ReferenceField { base, .. }
            | FieldSchema::OptionsField { base, .. }
            | FieldSchema::JsonObjectField { base }
            | FieldSchema::SectionField { base, .. }
            | FieldSchema::ListingField { base, .. }
            | FieldSchema::TagField { base, .. }
            | FieldSchema::ElementField { base, .. }
            | FieldSchema::CompositeField { base, .. } => &base.name,
        }
    }
}

impl FieldSchema {
    /// Returns `true` if this field is an `ElementField`.
    pub fn is_element_field(&self) -> bool {
        matches!(self, FieldSchema::ElementField { .. })
    }

    /// If this field is an `ElementField`, returns the `element_type`. Otherwise returns None.
    pub fn element_type_name(&self) -> Option<&str> {
        match self {
            FieldSchema::ElementField { element_type, .. } => Some(element_type),
            _ => None,
        }
    }

    /// Transform an `ElementField` into a `CompositeField` in-place by “rebuilding” self.
    /// You might want to pass in `Vec<FieldSchema>` and `Option<Vec<LayoutNode>>` as the
    /// resolved data from the corresponding `ElementType`.
    pub fn transform_to_composite(
        &mut self,
        new_fields: Vec<FieldSchema>,
        layout: Option<Vec<LayoutNode>>,
    ) {
        if let FieldSchema::ElementField { base, .. } = self {
            // Grab the old `FieldTypeSchema` because we’ll carry it over to the new variant.
            let old_base = base.clone();
            *self = FieldSchema::CompositeField {
                base: old_base,
                fields: new_fields,
                layout,
            };
        }
    }
}
#[cfg(test)]
mod tests {
    use super::FieldSchema;

    #[test]
    fn test_field_schema_msgpack_roundtrip() {
        let json_data = serde_json::json!({
            "$type": "TextField",
            "name": "title",
            "description": "Article title"
        });

        // Deserialize from JSON
        let field: FieldSchema = serde_json::from_value(json_data).expect("Failed to parse JSON");
        println!("✅ Deserialized from JSON: {:?}", field);

        // Serialize to MessagePack
        let msgpack_bytes =
            rmp_serde::to_vec_named(&field).expect("Failed to serialize to MessagePack");
        println!(
            "✅ Serialized to MessagePack: {} bytes",
            msgpack_bytes.len()
        );

        // Deserialize from MessagePack
        let field2: FieldSchema =
            rmp_serde::from_slice(&msgpack_bytes).expect("Failed to deserialize from MessagePack");
        println!("✅ Deserialized from MessagePack: {:?}", field2);

        assert_eq!(field, field2);
        println!("✅ Round-trip successful!");
    }
}
