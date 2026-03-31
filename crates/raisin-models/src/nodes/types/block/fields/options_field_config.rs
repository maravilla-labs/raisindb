//! Field configuration for options fields.
//!
//! This struct defines the configuration options for options fields (dropdown, radio, etc.) in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for an options field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct OptionsFieldConfig {
    /// Available options for selection.
    pub options: Vec<String>,
    /// How options are rendered (dropdown, radio, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_as: Option<OptionsRenderType>,
    /// Allow multiple selections (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_select: Option<bool>,
}

/// How options are rendered in the UI.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub enum OptionsRenderType {
    Dropdown,
    Radio,
    Checkbox,
}
