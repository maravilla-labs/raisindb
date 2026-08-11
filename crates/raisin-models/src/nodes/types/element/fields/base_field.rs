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

use crate::nodes::properties::schema::IndexType;
use crate::nodes::properties::PropertyValue;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Which indexes include this field's value (Fulltext / Vector / Property).
    ///
    /// Mirrors `PropertyValueSchema.index` on NodeType properties so element and
    /// archetype fields are configurable the same way. `None`/empty means the
    /// field value is NOT indexed — the element/archetype identity is always
    /// searchable regardless (see shape-driven full-text indexing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<Vec<IndexType>>,
    /// Arbitrary, free-form metadata attached to the field.
    ///
    /// Not interpreted by the database; round-tripped as-is so editors and
    /// integrations can attach their own configuration to a field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, PropertyValue>>,
    /// The value is a secret: it is moved into the secret store on write and the
    /// stored field holds a `secret://…` reference instead of the plaintext.
    ///
    /// Mirrors `PropertyValueSchema.encrypted` so element and archetype fields
    /// are declarable the same way as NodeType properties. Enforced by the
    /// server at the write layer; reads return the reference and never resolve
    /// it. The legacy `meta.secret: true` spelling is still honoured on read —
    /// see [`is_secret`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted: Option<bool>,
}

/// Whether a field schema declares its value a secret.
///
/// Checks the first-class [`FieldTypeSchema::encrypted`] field, then falls back
/// to the legacy `meta.secret: true` convention. Mirrors
/// [`crate::nodes::properties::schema::is_secret`] for NodeType properties.
pub fn is_secret(schema: &FieldTypeSchema) -> bool {
    if let Some(flag) = schema.encrypted {
        return flag;
    }
    // The legacy fallback goes through the ONE `meta` boolean reader, so this
    // and the NodeType-property side cannot drift over what counts as true.
    crate::nodes::properties::schema::meta_bool(schema.meta.as_ref(), "secret")
}
