//! Base field type schema for all block fields.
//!
//! This struct defines the common properties shared by all field types in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The base schema for a field in a block type.
///
/// This struct contains common metadata and configuration for all field types.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct FieldTypeSchema {
    /// Unique name of the field.
    pub name: String,
    /// Human-readable title for the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Label for the field (UI display).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether the field is required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// Description of the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Help or tooltip text for the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
    /// Default value for the field (generic JSON value).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<serde_json::Value>,
    /// Any specific validation rules (as strings).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validations: Option<Vec<String>>,
    /// Whether the field is hidden on publish.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_hidden: Option<bool>,
    /// Whether multiple values are allowed (also accepts `repeatable` in YAML).
    #[serde(skip_serializing_if = "Option::is_none", alias = "repeatable")]
    pub multiple: Option<bool>,
    /// Whether the field is a design value field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub design_value: Option<bool>,
    /// Whether the field is translatable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translatable: Option<bool>,
}
