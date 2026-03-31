//! Field type definitions and schema for block fields.
//!
//! This module defines the `FieldTypeSchema` and `FieldSchema` enums, which represent the configuration and schema for all block field types in RaisinDB.
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
use crate::nodes::types::block::fields::layout::LayoutNode;

/// Represents the schema for a field in a block type.
///
/// Each variant corresponds to a different field type, with its own configuration struct.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(tag = "$type")]
pub enum FieldSchema {
    /// A simple text field.
    TextField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<TextFieldConfig>,
    },
    /// A rich text field supporting formatting.
    RichTextField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<RichTextFieldConfig>,
    },
    /// A numeric field (integer or float).
    NumberField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<NumberFieldConfig>,
    },
    /// A date/time field.
    DateField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<DateFieldConfig>,
    },
    /// A location field (not yet implemented).
    LocationField {
        #[serde(flatten)]
        base: FieldTypeSchema,
    },
    /// A boolean field.
    BooleanField {
        #[serde(flatten)]
        base: FieldTypeSchema,
    },
    /// A media field (image, video, etc.).
    MediaField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<MediaFieldConfig>,
    },
    /// A reference to another entry.
    ReferenceField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<ReferenceFieldConfig>,
    },
    /// A tag field for categorization.
    TagField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<TagFieldConfig>,
    },
    /// An options field (dropdown, radio, etc.).
    OptionsField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<OptionsFieldConfig>,
    },
    /// A JSON object field.
    JsonObjectField {
        #[serde(flatten)]
        base: FieldTypeSchema,
    },
    /// A composite field containing multiple subfields.
    CompositeField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        fields: Vec<FieldSchema>,
        #[serde(skip_serializing_if = "Option::is_none")]
        layout: Option<Vec<LayoutNode>>,
    },
    /// A section field grouping other fields.
    SectionField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        allowed_block_types: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        render_as: Option<String>,
    },
    /// A block field referencing a block type.
    BlockField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        block_type: String,
    },
    /// A listing field for referencing multiple entries.
    ListingField {
        #[serde(flatten)]
        base: FieldTypeSchema,
        #[serde(skip_serializing_if = "Option::is_none")]
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
            | FieldSchema::BlockField { base, .. }
            | FieldSchema::CompositeField { base, .. } => &base.name,
        }
    }
}

impl FieldSchema {
    /// Returns `true` if this field is a `BlockField`.
    pub fn is_block_field(&self) -> bool {
        matches!(self, FieldSchema::BlockField { .. })
    }

    /// If this field is a `BlockField`, returns the `block_type`. Otherwise returns None.
    pub fn block_type_name(&self) -> Option<&str> {
        match self {
            FieldSchema::BlockField { block_type, .. } => Some(block_type),
            _ => None,
        }
    }

    /// Transform a `BlockField` into a `CompositeField` in-place by “rebuilding” self.
    /// You might want to pass in `Vec<FieldSchema>` and `Option<Vec<LayoutNode>>` as the
    /// resolved data from the corresponding `BlockType`.
    pub fn transform_to_composite(
        &mut self,
        new_fields: Vec<FieldSchema>,
        layout: Option<Vec<LayoutNode>>,
    ) {
        if let FieldSchema::BlockField { base, .. } = self {
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
