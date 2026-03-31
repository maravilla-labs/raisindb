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

//! Element type module for RaisinDB.
//!
//! This module defines the `ElementType` struct and related logic for element types in RaisinDB.

use super::FieldSchema;
use crate::errors::RaisinModelError;
use crate::nodes::properties::PropertyValue;
use crate::nodes::types::archetype::InitialArchetypeStructure;
use crate::nodes::types::element::fields::layout::LayoutNode;
use chrono::{DateTime, Utc};
use nanoid::nanoid;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents an element type in RaisinDB.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct ElementType {
    /// Unique identifier for the element type.
    #[serde(default = "default_uuid")]
    pub id: String,
    /// Name of the element type.
    pub name: String,
    /// Parent element type to inherit fields from (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    /// Human-readable title for the element type (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Icon for the element type (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Description of the element type (optional).
    #[serde(default)]
    pub description: Option<String>,
    /// Fields for the element type.
    #[serde(default)]
    pub fields: Vec<FieldSchema>,
    /// Initial content structure (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_content: Option<InitialArchetypeStructure>,
    /// Layout nodes (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Vec<LayoutNode>>,
    /// Meta properties for additional configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, PropertyValue>>,
    /// Version of the element type.
    #[serde(default)]
    pub version: Option<i32>,
    /// Creation timestamp.
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    /// Last update timestamp.
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    /// Publication timestamp (if published).
    #[serde(default)]
    pub published_at: Option<DateTime<Utc>>,
    /// Actor who published this element type.
    #[serde(default)]
    pub published_by: Option<String>,
    /// Whether this element type is currently publishable.
    #[serde(default)]
    pub publishable: Option<bool>,
    /// If true, no undefined properties are allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
    /// Previous version identifier if versioned.
    #[serde(default)]
    pub previous_version: Option<String>,
}

impl ElementType {
    /// Generate a JSON schema for the `ElementType` struct using the Schemars API.
    ///
    /// # Returns
    /// A `serde_json::Value` representing the JSON schema for `ElementType`.
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(ElementType))
            .map_err(RaisinModelError::from_serde)
            .expect("Failed to convert schema to JSON value")
    }
}

fn default_uuid() -> String {
    nanoid!(16)
}
