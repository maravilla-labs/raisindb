//! Field configuration for media fields.
//!
//! This struct defines the configuration options for media fields in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a media field (e.g., image, video).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct MediaFieldConfig {
    /// Allowed media types (e.g., ["image", "video"]).
    pub allowed_types: Option<Vec<String>>,
}
