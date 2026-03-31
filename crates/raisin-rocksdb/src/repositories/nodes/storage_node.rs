//! Internal storage representation for nodes
//!
//! `StorageNode` is the internal struct used for RocksDB serialization.
//! Unlike `Node`, it excludes the `path` field to enable O(1) move operations.
//!
//! The path is materialized from the PATH_INDEX on read, rather than being
//! stored redundantly in every node blob.

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{Node, RelationRef};
use raisin_models::StorageTimestamp;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Internal storage representation - path NOT stored in blob
///
/// This struct mirrors `Node` but excludes:
/// - `path`: Materialized from PATH_INDEX on read (the key optimization!)
/// - `has_children`: Computed field, never stored
///
/// And adds:
/// - `parent_id`: For O(1) parent lookup without path parsing
///
/// # Why this exists
///
/// When moving a node tree, we previously had to update every descendant's
/// path field (O(N) blob writes). By not storing path in the blob:
/// - Move operations only need to update the PATH_INDEX entries
/// - Root node blob gets updated (for parent field change)
/// - Descendant blobs remain unchanged
///
/// # Serialization
///
/// Uses MessagePack via `rmp_serde` for compact binary format, matching
/// the existing node serialization approach.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct StorageNode {
    /// Unique identifier for this node
    pub id: String,

    /// Display name of the node (used in URLs and hierarchical paths)
    pub name: String,

    // NOTE: `path` is intentionally omitted - this is the key optimization!
    // Path is looked up from PATH_INDEX on read.
    /// The NodeType name that defines this node's schema
    pub node_type: String,

    /// Optional archetype for specialized rendering
    #[serde(default)]
    pub archetype: Option<String>,

    /// Key-value map of properties, validated against the NodeType schema
    #[serde(default)]
    pub properties: HashMap<String, PropertyValue>,

    /// Ordered list of child node IDs
    #[serde(default)]
    pub children: Vec<String>,

    /// Fractional index for ordering among siblings
    #[serde(default)]
    pub order_key: String,

    // NOTE: `has_children` is intentionally omitted - it's a computed field
    // that is populated at read time based on children.len() or index lookup.
    /// Name of the parent node (None for root nodes)
    #[serde(default)]
    pub parent: Option<String>,

    /// Parent node ID for O(1) parent lookup
    ///
    /// This enables efficient parent resolution without path parsing.
    /// For root nodes (path = "/name"), this is None.
    #[serde(default)]
    pub parent_id: Option<String>,

    /// Version number for this node (incremented on updates)
    #[serde(default = "default_version")]
    pub version: i32,

    /// Timestamp when this node was created (stored as i64 nanoseconds in MessagePack)
    #[serde(default)]
    pub created_at: Option<StorageTimestamp>,

    /// Timestamp when this node was last updated (stored as i64 nanoseconds in MessagePack)
    #[serde(default)]
    pub updated_at: Option<StorageTimestamp>,

    /// Timestamp when this node was published (None if unpublished, stored as i64 nanoseconds in MessagePack)
    #[serde(default)]
    pub published_at: Option<StorageTimestamp>,

    /// User ID who published this node
    #[serde(default)]
    pub published_by: Option<String>,

    /// User ID who last updated this node
    #[serde(default)]
    pub updated_by: Option<String>,

    /// User ID who created this node
    #[serde(default)]
    pub created_by: Option<String>,

    /// Translations for multi-language support
    #[serde(default)]
    pub translations: Option<HashMap<String, PropertyValue>>,

    /// Tenant ID for multi-tenant deployments
    #[serde(default)]
    pub tenant_id: Option<String>,

    /// Workspace this node belongs to
    #[serde(default)]
    pub workspace: Option<String>,

    /// Owner user ID (for access control)
    #[serde(default)]
    pub owner_id: Option<String>,

    /// Relations to other nodes
    #[serde(default)]
    pub relations: Vec<RelationRef>,
}

fn default_version() -> i32 {
    1
}

impl StorageNode {
    /// Convert a Node to StorageNode for persistence
    ///
    /// The path is NOT stored - it will be looked up from PATH_INDEX on read.
    /// The parent_id is extracted from the path for efficient parent lookup.
    /// Timestamps are converted from DateTime<Utc> to StorageTimestamp for compact storage.
    pub fn from_node(node: &Node, parent_id: Option<String>) -> Self {
        Self {
            id: node.id.clone(),
            name: node.name.clone(),
            node_type: node.node_type.clone(),
            archetype: node.archetype.clone(),
            properties: node.properties.clone(),
            children: node.children.clone(),
            order_key: node.order_key.clone(),
            parent: node.parent.clone(),
            parent_id,
            version: node.version,
            created_at: node.created_at.map(StorageTimestamp::from),
            updated_at: node.updated_at.map(StorageTimestamp::from),
            published_at: node.published_at.map(StorageTimestamp::from),
            published_by: node.published_by.clone(),
            updated_by: node.updated_by.clone(),
            created_by: node.created_by.clone(),
            translations: node.translations.clone(),
            tenant_id: node.tenant_id.clone(),
            workspace: node.workspace.clone(),
            owner_id: node.owner_id.clone(),
            relations: node.relations.clone(),
        }
    }

    /// Convert StorageNode back to Node with materialized path
    ///
    /// The path must be provided from PATH_INDEX lookup.
    /// has_children is set to None (populated by the service layer if needed).
    /// Timestamps are converted from StorageTimestamp back to DateTime<Utc>.
    pub fn into_node(self, path: String) -> Node {
        Node {
            id: self.id,
            name: self.name,
            path,
            node_type: self.node_type,
            archetype: self.archetype,
            properties: self.properties,
            children: self.children,
            order_key: self.order_key,
            has_children: None, // Computed field, populated at service layer
            parent: self.parent,
            version: self.version,
            created_at: self.created_at.map(|ts| ts.into_inner()),
            updated_at: self.updated_at.map(|ts| ts.into_inner()),
            published_at: self.published_at.map(|ts| ts.into_inner()),
            published_by: self.published_by,
            updated_by: self.updated_by,
            created_by: self.created_by,
            translations: self.translations,
            tenant_id: self.tenant_id,
            workspace: self.workspace,
            owner_id: self.owner_id,
            relations: self.relations,
        }
    }

    /// Get the parent_id for efficient parent resolution
    pub fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_node_roundtrip() {
        let mut node = Node::default();
        node.id = "test-id".to_string();
        node.name = "test-name".to_string();
        node.path = "/parent/test-name".to_string();
        node.node_type = "raisin:Page".to_string();
        node.order_key = "a".to_string();
        node.parent = Some("parent".to_string());
        node.version = 1;

        // Convert to StorageNode
        let storage = StorageNode::from_node(&node, Some("parent-id".to_string()));

        // Verify parent_id is set
        assert_eq!(storage.parent_id(), Some("parent-id"));

        // Convert back with materialized path
        let restored = storage.into_node("/parent/test-name".to_string());

        // Verify all fields match
        assert_eq!(restored.id, node.id);
        assert_eq!(restored.name, node.name);
        assert_eq!(restored.path, node.path);
        assert_eq!(restored.node_type, node.node_type);
        assert_eq!(restored.order_key, node.order_key);
        assert_eq!(restored.parent, node.parent);
        assert_eq!(restored.version, node.version);
        assert_eq!(restored.has_children, None); // Always None from storage
    }

    #[test]
    fn test_storage_node_serialization() {
        let mut node = Node::default();
        node.id = "ser-test".to_string();
        node.name = "serialization-test".to_string();
        node.path = "/test/serialization-test".to_string();
        node.node_type = "raisin:Page".to_string();

        let storage = StorageNode::from_node(&node, None);

        // Serialize to MessagePack
        let bytes = rmp_serde::to_vec(&storage).expect("serialization failed");

        // Deserialize back
        let restored: StorageNode = rmp_serde::from_slice(&bytes).expect("deserialization failed");

        assert_eq!(restored.id, "ser-test");
        assert_eq!(restored.name, "serialization-test");
        assert_eq!(restored.node_type, "raisin:Page");
    }

    #[test]
    fn test_storage_node_excludes_path() {
        let mut node = Node::default();
        node.id = "no-path".to_string();
        node.name = "no-path-test".to_string();
        node.path = "/very/long/path/that/should/not/be/stored".to_string();

        let storage = StorageNode::from_node(&node, None);

        // Serialize to MessagePack
        let bytes = rmp_serde::to_vec(&storage).expect("serialization failed");

        // The serialized bytes should NOT contain the path string
        // (This is a rough check - the path string shouldn't appear in the blob)
        let bytes_str = String::from_utf8_lossy(&bytes);
        assert!(
            !bytes_str.contains("very/long/path"),
            "path should not be in serialized blob"
        );
    }
}
