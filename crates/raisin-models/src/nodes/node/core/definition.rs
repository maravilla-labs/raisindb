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

//! Core Node struct definition and implementation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::nodes::properties::Properties;
use crate::nodes::properties::PropertyValue;

fn default_version() -> i32 {
    1
}

/// Reserved node ID for the root node in each workspace.
/// This ID is automatically created when a workspace is initialized.
pub const ROOT_NODE_ID: &str = "M2016N2019L2022T";

/// A content node in RaisinDB's hierarchical structure
///
/// Nodes are the primary content entities, organized in a tree structure within workspaces.
/// Each node has a type (NodeType) that defines its schema, allowed children, and behavior.
///
/// # Examples
///
/// ```
/// use raisin_models::nodes::Node;
/// use std::collections::HashMap;
///
/// let node = Node {
///     id: "node-123".to_string(),
///     name: "My Page".to_string(),
///     path: "/content/my-page".to_string(),
///     node_type: "raisin:page".to_string(),
///     archetype: None,
///     properties: HashMap::new(),
///     children: vec![],
///     order_key: "a".to_string(),
///     has_children: None,
///     parent: Some("content".to_string()),  // Just the parent name, not full path!
///     version: 1,
///     created_at: None,
///     updated_at: None,
///     published_at: None,
///     published_by: None,
///     updated_by: None,
///     created_by: None,
///     translations: None,
///     tenant_id: None,
///     workspace: Some("ws1".to_string()),
///     owner_id: None,
///     relations: Vec::new(),
/// };
/// ```
#[serde_with::serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Unique identifier for this node
    #[serde(default)]
    pub id: String,

    /// Display name of the node (used in URLs and hierarchical paths)
    pub name: String,

    /// Full path to this node in the tree (e.g., "/content/my-page")
    #[serde(default)]
    pub path: String,

    /// The NodeType name that defines this node's schema
    pub node_type: String,

    /// Optional archetype for specialized rendering
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_string_lenient_msgpack"
    )]
    pub archetype: Option<String>,

    /// Key-value map of properties, validated against the NodeType schema
    #[serde(default)]
    pub properties: HashMap<String, PropertyValue>,

    /// Ordered list of child node IDs
    /// Note: The actual ordering is determined by each child's `order_key` field
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_vec_string_lenient"
    )]
    pub children: Vec<String>,

    /// Fractional index for ordering among siblings
    /// Base62 string that's lexicographically sortable
    /// Examples: "a", "b", "b5", "c"
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_string_lenient_msgpack"
    )]
    pub order_key: String,

    /// Whether this node has children (computed field, not stored)
    /// This is populated at the service layer before JSON serialization
    pub has_children: Option<bool>,

    /// Name of the parent node (None for root nodes)
    ///
    /// This stores ONLY the parent node's name, not its full path.
    /// To get the full parent path, use `parent_path()`.
    ///
    /// Example: If this node's path is "/content/docs/page1",
    /// then parent should be "docs" (not "/content/docs")
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_string_lenient_msgpack"
    )]
    pub parent: Option<String>,

    /// Version number for this node (incremented on updates)
    #[serde(default = "default_version")]
    pub version: i32,

    /// Timestamp when this node was created (RFC3339 string in MessagePack)
    /// Defaults are applied at the service layer before storage
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_datetime_lenient_msgpack"
    )]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,

    /// Timestamp when this node was last updated (RFC3339 string in MessagePack)
    /// Defaults are applied at the service layer before storage
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_datetime_lenient_msgpack"
    )]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,

    /// Timestamp when this node was published (RFC3339 string in MessagePack, None if unpublished)
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_datetime_lenient_msgpack"
    )]
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,

    /// User ID who published this node
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_string_lenient_msgpack"
    )]
    pub published_by: Option<String>,

    /// User ID who last updated this node
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_string_lenient_msgpack"
    )]
    pub updated_by: Option<String>,

    /// User ID who created this node
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_string_lenient_msgpack"
    )]
    pub created_by: Option<String>,

    /// Translations for multi-language support
    #[serde(default)]
    pub translations: Option<HashMap<String, PropertyValue>>,

    /// Tenant ID for multi-tenant deployments
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_string_lenient_msgpack"
    )]
    pub tenant_id: Option<String>,

    /// Workspace this node belongs to
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_string_lenient_msgpack"
    )]
    pub workspace: Option<String>,

    /// Owner user ID (for access control)
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_optional_string_lenient_msgpack"
    )]
    pub owner_id: Option<String>,

    /// Relations to other nodes
    #[serde(
        default,
        deserialize_with = "crate::migrations::deserialize_vec_lenient"
    )]
    pub relations: Vec<super::super::RelationRef>,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            path: String::new(),
            node_type: String::new(),
            archetype: None,
            properties: HashMap::new(),
            children: Vec::new(),
            order_key: String::new(),
            has_children: None,
            parent: None,
            version: 1,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: None,
            tenant_id: None,
            workspace: None,
            owner_id: None,
            relations: Vec::new(),
        }
    }
}

impl Node {
    /// Get the full parent path derived from this node's path
    ///
    /// This derives the parent's full path by extracting it from the node's path.
    /// Returns None if this is a root node.
    ///
    /// # Example
    /// ```
    /// # use raisin_models::nodes::Node;
    /// let mut node = Node::default();
    /// node.path = "/content/docs/page1".to_string();
    /// assert_eq!(node.parent_path(), Some("/content/docs".to_string()));
    /// ```
    pub fn parent_path(&self) -> Option<String> {
        if self.path.is_empty() || self.path == "/" {
            return None;
        }

        self.path.rsplit_once('/').map(|(parent, _)| {
            if parent.is_empty() {
                "/".to_string()
            } else {
                parent.to_string()
            }
        })
    }

    /// DEPRECATED: Use parent_path() instead
    #[deprecated(since = "0.1.0", note = "Use parent_path() instead")]
    pub fn get_parent_path_from_node_path(&self) -> Option<String> {
        self.parent_path()
    }

    /// Extract the parent name from a given path
    ///
    /// This is a utility function to extract just the parent node's name from a full path.
    ///
    /// # Example
    /// ```
    /// # use raisin_models::nodes::Node;
    /// assert_eq!(Node::extract_parent_name_from_path("/content/docs/page1"), Some("docs".to_string()));
    /// assert_eq!(Node::extract_parent_name_from_path("/about"), Some("/".to_string())); // Root level node
    /// assert_eq!(Node::extract_parent_name_from_path("/"), None); // Root itself has no parent
    /// assert_eq!(Node::extract_parent_name_from_path("invalid"), None);
    /// ```
    pub fn extract_parent_name_from_path(path: &str) -> Option<String> {
        if path.is_empty() || path == "/" {
            return None;
        }

        // Get the parent path first
        let parent_path = path.rsplit_once('/')?.0;

        if parent_path.is_empty() || parent_path == "/" {
            // Parent is root - return "/" for API display
            return Some("/".to_string());
        }

        // Extract the name from the parent path
        parent_path
            .rsplit_once('/')
            .map(|(_, name)| name.to_string())
    }

    /// DEPRECATED: Use parent_path() instead
    ///
    /// This method is kept for backward compatibility but will be removed.
    #[deprecated(since = "0.1.0", note = "Use parent_path() instead")]
    pub fn get_parent_full_path(&self) -> String {
        self.parent_path().unwrap_or_default()
    }

    pub fn get_relative_path(&self, target_path: &str) -> String {
        if target_path.starts_with('/') {
            return target_path.to_string();
        }
        let binding = self.parent_path().unwrap_or_default();
        let current_dir_parts: Vec<&str> = binding.split('/').filter(|s| !s.is_empty()).collect();
        let target_parts: Vec<&str> = target_path.split('/').collect();
        if target_path.is_empty() || target_path == "./" {
            return "./".to_string();
        }
        if target_path.starts_with("../") {
            return target_path.to_string();
        }
        if !target_path.contains("../") {
            return target_path.to_string();
        }
        let mut up_count = 0;
        for part in &target_parts {
            if *part == ".." {
                up_count += 1;
            } else {
                break;
            }
        }
        let remaining_dirs = if up_count >= current_dir_parts.len() {
            return target_path.to_string();
        } else {
            current_dir_parts.len() - up_count
        };
        let prefix = "../".repeat(remaining_dirs);
        let suffix = target_parts[up_count..].join("/");
        format!("{}{}", prefix, suffix)
    }

    pub fn get_properties(&self) -> Properties<'_> {
        Properties::new(&self.properties)
    }
}
