use std::collections::HashMap;

use nanoid::nanoid;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::errors::RaisinModelError;
use crate::nodes::block::FieldSchema;
use crate::nodes::properties::PropertyValue;
use crate::nodes::types::block::view::View;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema, Validate)]
pub struct ContentType {
    #[serde(default = "default_uuid")]
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    // #[validate(regex = "URL_FRIENDLY_NAME_REGEX")] // Remove or fix if not used
    pub base_node_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<FieldSchema>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_content: Option<InititalContentStructure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<View>,
}

fn default_uuid() -> String {
    nanoid!(16)
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct InititalContentStructure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<HashMap<String, PropertyValue>>>,
}

impl ContentType {
    /// Validates the entire ContentType, including initial_content and fields.
    ///
    /// Similar to `NodeType::validate_full`, this runs all necessary checks:
    /// - Basic validation using `validator`.
    /// - (Optional) Add any field-level or initial_content checks here.
    ///
    /// # Future Enhancements
    ///
    /// This method currently only performs basic validation. In the future,
    /// you may want to add:
    /// - Field-level validation for ContentType-specific fields
    /// - initial_content validation
    /// - Cross-field validation rules
    pub fn validate_full(&self, _context: &std::sync::Arc<()>) -> Result<(), RaisinModelError> {
        self.validate()?;
        Ok(())
    }

    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(ContentType))
            .map_err(RaisinModelError::from_serde)
            .expect("Failed to convert content type schema to JSON value")
    }
}
