//! Field configuration for date fields.
//!
//! This struct defines the configuration options for date fields in RaisinDB block schemas.

use super::common::DateMode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a date field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct DateFieldConfig {
    /// ISO 8601 or custom formats (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_format: Option<String>,
    /// Determines the picker type: datetime, date, time, timerange (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_mode: Option<DateMode>,
}
