//! Field configuration for rich text fields.
//!
//! This struct defines the configuration options for rich text fields in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a rich text field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct RichTextFieldConfig {
    /// Maximum length for rich text (optional).
    pub max_length: Option<usize>,
}
