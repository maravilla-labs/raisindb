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

//! NodeType struct definition, NodeTypeVersion, and core implementation.

use chrono::{DateTime, Utc};
use nanoid::nanoid;
use serde::{Deserialize, Serialize};

fn deserialize_nullable_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value = Option::<Vec<T>>::deserialize(deserializer)?;
    Ok(value.unwrap_or_default())
}
use std::collections::HashMap;
use validator::Validate;

use crate::nodes::properties::schema::{CompoundIndexDefinition, IndexType, PropertyValueSchema};
use crate::nodes::properties::value::PropertyValue;

pub type OverrideProperties = HashMap<String, PropertyValue>;

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Validate)]
pub struct NodeType {
    #[serde(default = "default_uuid")]
    pub id: Option<String>,
    #[serde(default)]
    pub strict: Option<bool>,
    #[validate(regex(path = "*crate::nodes::types::utils::URL_FRIENDLY_NAME_REGEX"))]
    pub name: String,
    #[serde(default)]
    #[validate(regex(path = "*crate::nodes::types::utils::URL_FRIENDLY_NAME_REGEX"))]
    pub extends: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub mixins: Vec<String>,
    #[serde(default)]
    pub overrides: Option<OverrideProperties>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub version: Option<i32>,
    #[serde(default)]
    pub properties: Option<Vec<PropertyValueSchema>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub allowed_children: Vec<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub required_nodes: Vec<String>,
    #[serde(default)]
    pub initial_structure: Option<super::super::super::initial_structure::InitialNodeStructure>,
    #[serde(default)]
    pub versionable: Option<bool>,
    #[serde(default)]
    pub publishable: Option<bool>,
    #[serde(default)]
    pub auditable: Option<bool>,
    /// Whether this node type should be indexed at all
    /// Default: None (treated as true for backward compatibility)
    #[serde(default)]
    pub indexable: Option<bool>,
    /// Which index types are enabled for this node type
    /// Default: None (all index types allowed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_types: Option<Vec<IndexType>>,
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub published_by: Option<String>,
    #[serde(default)]
    pub previous_version: Option<String>,
    /// Compound indexes for efficient multi-column queries
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compound_indexes: Option<Vec<CompoundIndexDefinition>>,
    /// Whether this NodeType represents a mixin (reusable property set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_mixin: Option<bool>,
}

fn default_uuid() -> Option<String> {
    Some(nanoid!(16))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeTypeVersion {
    pub id: Option<String>,
    pub node_type_id: String,
    pub version: i32,
    pub node_type: NodeType,
    pub created_at: String,
    pub updated_at: Option<DateTime<Utc>>,
}

impl NodeType {
    pub fn auditable(&self) -> bool {
        self.auditable.unwrap_or(false)
    }

    pub fn is_published(&self) -> bool {
        self.publishable.unwrap_or(false)
    }

    /// Create a minimal NodeType for testing with required fields only
    #[cfg(test)]
    pub fn test_minimal(name: impl Into<String>) -> Self {
        Self {
            id: Some(nanoid!(16)),
            strict: None,
            name: name.into(),
            extends: None,
            mixins: Vec::new(),
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: Vec::new(),
            required_nodes: Vec::new(),
            initial_structure: None,
            versionable: None,
            publishable: None,
            auditable: None,
            indexable: None,
            index_types: None,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            previous_version: None,
            compound_indexes: None,
            is_mixin: None,
        }
    }
}
