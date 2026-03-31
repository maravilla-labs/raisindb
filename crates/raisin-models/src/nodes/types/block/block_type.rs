//! Block type module for RaisinDB.
//!
//! This module defines the `BlockType` struct and related logic for block types in RaisinDB.

use super::view::View;
use super::FieldSchema;
use crate::errors::RaisinModelError;
use crate::nodes::types::block::fields::layout::LayoutNode;
use crate::nodes::types::content_type::InititalContentStructure;
use nanoid::nanoid;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Represents a block type in RaisinDB.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct BlockType {
    /// Unique identifier for the block type.
    #[serde(default = "default_uuid")]
    pub id: String,
    /// Name of the block type.
    pub name: String,
    /// Icon for the block type (optional).
    pub icon: Option<String>,
    /// Description of the block type (optional).
    pub description: Option<String>,
    /// Fields for the block type.
    pub fields: Vec<FieldSchema>,
    /// Initial content structure (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_content: Option<InititalContentStructure>,
    /// Layout nodes (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<Vec<LayoutNode>>,
    /// View configuration (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<View>,
}

impl BlockType {
    /// Generate a JSON schema for the `BlockType` struct using the Schemars API.
    ///
    /// # Returns
    /// A `serde_json::Value` representing the JSON schema for `BlockType`.
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(BlockType))
            .map_err(RaisinModelError::from_serde)
            .expect("Failed to convert schema to JSON value")
    }
}

fn default_uuid() -> String {
    nanoid!(16)
}
