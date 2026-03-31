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

//! Base field type schema for all block fields.
//!
//! This struct defines the common properties shared by all field types in RaisinDB block schemas.

use crate::nodes::properties::PropertyValue;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The base schema for a field in a block type.
///
/// This struct contains common metadata and configuration for all field types.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema, Default)]
pub struct FieldTypeSchema {
    #[serde(default)]
    /// Unique name of the field.
    pub name: String,
    /// Human-readable title for the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Label for the field (UI display).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether the field is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// Description of the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Help or tooltip text for the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
    /// Default value for the field (PropertyValue).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<PropertyValue>,
    /// Any specific validation rules (as strings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validations: Option<Vec<String>>,
    /// Whether the field is hidden on publish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_hidden: Option<bool>,
    /// Whether multiple values are allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiple: Option<bool>,
    /// Whether the field is a design value field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design_value: Option<bool>,
    /// Whether the field is translatable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translatable: Option<bool>,
}
