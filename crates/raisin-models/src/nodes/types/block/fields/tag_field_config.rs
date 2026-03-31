//! Field configuration for tag fields.
//!
//! This struct defines the configuration options for tag fields in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a tag field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct TagFieldConfig {
    /// Allowed tags (optional).
    pub allowed_tags: Option<Vec<String>>,
    /// Maximum number of tags (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tags: Option<usize>,
}
