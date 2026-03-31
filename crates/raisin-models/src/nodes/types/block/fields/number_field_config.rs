//! Field configuration for number fields.
//!
//! This struct defines the configuration options for number fields in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a number field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct NumberFieldConfig {
    /// True for integers, false for decimals (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_integer: Option<bool>,
    /// Minimum value (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value: Option<f64>,
    /// Maximum value (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value: Option<f64>,
}
