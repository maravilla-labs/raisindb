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

// PropertyValueSchema, PropertyType, and validation functions

use crate::nodes::properties::value::PropertyValue;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Index types available for properties and node types
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
pub enum IndexType {
    /// Tantivy full-text search with language support and stemming
    Fulltext,
    /// HNSW vector embeddings for AI-powered semantic search
    Vector,
    /// RocksDB property_index CF for exact-match lookups
    Property,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct PropertyValueSchema {
    #[validate(regex(path = "*crate::nodes::properties::utils::URL_FRIENDLY_NAME_REGEX"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type", alias = "property_type")]
    pub property_type: PropertyType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<PropertyValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<HashMap<String, PropertyValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<HashMap<String, PropertyValueSchema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<PropertyValueSchema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<PropertyValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, PropertyValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_translatable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[validate(custom = "validate_allow_additional_properties")]
    pub allow_additional_properties: Option<bool>,
    /// Which indexes this property should be included in
    /// Default: None (property is not indexed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<Vec<IndexType>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub enum PropertyType {
    #[serde(alias = "string")]
    String,
    #[serde(alias = "number")]
    Number,
    #[serde(alias = "boolean")]
    Boolean,
    #[serde(alias = "array")]
    Array,
    #[serde(alias = "object")]
    Object,
    #[serde(alias = "date")]
    Date,
    #[serde(alias = "url")]
    URL,
    #[serde(alias = "reference")]
    Reference,
    #[serde(alias = "nodetype", alias = "nodeType")]
    NodeType,
    #[serde(alias = "element")]
    Element,
    #[serde(alias = "composite")]
    Composite,
    #[serde(alias = "resource")]
    Resource,
}

/// Compound index definition for efficient multi-column queries.
///
/// A compound index combines multiple property columns into a single index key,
/// enabling efficient queries that filter on multiple columns and/or need ordered results.
///
/// Example: An index on `(node_type, category, created_at DESC)` enables queries like:
/// ```sql
/// SELECT * FROM content
/// WHERE node_type = 'news:Article' AND properties->>'category' = 'business'
/// ORDER BY created_at DESC
/// LIMIT 10
/// ```
/// to execute in O(LIMIT) time instead of scanning all matching nodes.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
pub struct CompoundIndexDefinition {
    /// Unique name for this index (used in key encoding)
    pub name: String,

    /// Columns in order (leading columns for equality, trailing for ordering)
    pub columns: Vec<CompoundIndexColumn>,

    /// If true, the last column is used for ordering (created_at, updated_at)
    #[serde(default)]
    pub has_order_column: bool,
}

/// A column in a compound index definition.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
pub struct CompoundIndexColumn {
    /// Property name.
    /// Use system property names for node metadata:
    /// - `__node_type` for node_type
    /// - `__created_at` for created_at
    /// - `__updated_at` for updated_at
    ///   Use regular property names (e.g., `category`, `status`) for JSON properties.
    pub property: String,

    /// For ordering columns: sort direction (true = ASC, false = DESC).
    /// Only applicable when this is the last column and `has_order_column` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ascending: Option<bool>,

    /// Column type hint for proper key encoding.
    /// Timestamps need special encoding for sortable keys.
    pub column_type: CompoundColumnType,
}

/// Type hint for compound index column encoding.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
pub enum CompoundColumnType {
    /// String values (node_type, category, etc.)
    String,
    /// Integer values
    Integer,
    /// Timestamp values (created_at, updated_at) - encoded for sortability
    Timestamp,
    /// Boolean values
    Boolean,
}
