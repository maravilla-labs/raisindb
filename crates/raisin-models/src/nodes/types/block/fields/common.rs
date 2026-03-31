//! Common types for field configuration.
//!
//! This module defines enums and shared types used by multiple field config structs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Mode for date fields (date, time, datetime, timerange).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub enum DateMode {
    /// Date and time picker.
    DateTime,
    /// Date only picker.
    Date,
    /// Time only picker.
    Time,
    /// Time range picker.
    TimeRange,
}
