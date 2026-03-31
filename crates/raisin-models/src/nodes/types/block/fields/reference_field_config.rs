//! Field configuration for reference fields.
//!
//! This struct defines the configuration options for reference fields in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a reference field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct ReferenceFieldConfig {
    /// Types of referenced entries (optional).
    pub allowed_entry_types: Option<Vec<String>>,
}
