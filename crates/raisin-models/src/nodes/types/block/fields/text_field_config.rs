//! Field configuration for text fields.
//!
//! This struct defines the configuration options for text fields in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a text field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct TextFieldConfig {
    /// Maximum length for text (optional).
    pub max_length: Option<usize>,
}
