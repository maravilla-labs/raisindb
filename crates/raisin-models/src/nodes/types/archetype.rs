// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use chrono::{DateTime, Utc};
use std::collections::HashMap;

use nanoid::nanoid;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::errors::RaisinModelError;
use crate::nodes::element::FieldSchema;
use crate::nodes::properties::PropertyValue;
use crate::nodes::types::element::fields::layout::LayoutNode;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema, Validate)]
pub struct Archetype {
    #[serde(default = "default_uuid")]
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    // #[validate(regex = "URL_FRIENDLY_NAME_REGEX")] // Remove or fix if not used
    pub base_node_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<FieldSchema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_content: Option<InitialArchetypeStructure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Vec<LayoutNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, PropertyValue>>,
    #[serde(default)]
    pub version: Option<i32>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub published_by: Option<String>,
    #[serde(default)]
    pub publishable: Option<bool>,
    /// If true, no undefined properties are allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
    #[serde(default)]
    pub previous_version: Option<String>,
}

fn default_uuid() -> String {
    nanoid!(16)
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct InitialArchetypeStructure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<HashMap<String, PropertyValue>>>,
}

impl Archetype {
    /// Validates the entire Archetype, including initial_content and fields.
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
        serde_json::to_value(schemars::schema_for!(Archetype))
            .map_err(RaisinModelError::from_serde)
            .expect("Failed to convert archetype schema to JSON value")
    }
}
