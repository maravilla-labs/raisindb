//! Detailed change tracking for CRDT replication
//!
//! This module tracks granular changes within a transaction to enable
//! precise operation capture for replication.

use raisin_hlc::HLC;
use raisin_models::nodes::{Node, RelationRef};
use serde_json::Value;
use std::collections::HashMap;

/// Detailed information about a property change
#[derive(Debug, Clone)]
pub struct PropertyChange {
    pub property_name: String,
    pub old_value: Option<Value>,
    pub new_value: Option<Value>,
}

/// Detailed information about a relation change
///
/// Relations are identified by the composite key (source_id, target_id, relation_type).
/// Uses Last-Write-Wins (LWW) CRDT semantics - the latest change wins based on HLC timestamps.
#[derive(Debug, Clone)]
pub struct RelationChange {
    pub source_id: String,
    pub source_workspace: String,
    pub relation_type: String,
    pub target_id: String,
    pub target_workspace: String,
    pub relation: Option<RelationRef>,
    pub is_addition: bool, // true = add, false = remove
}

/// Detailed information about a node move
#[derive(Debug, Clone)]
pub struct NodeMove {
    pub node_id: String,
    pub old_parent_id: Option<String>,
    pub new_parent_id: Option<String>,
    pub position: Option<String>,
}

/// Node metadata field changes (tracked separately from generic properties)
#[derive(Debug, Clone, Default)]
pub struct NodeMetadataChanges {
    /// Name change: (old_name, new_name)
    pub name_change: Option<(String, String)>,

    /// Archetype change: (old_archetype, new_archetype)
    pub archetype_change: Option<(Option<String>, Option<String>)>,

    /// Order key change: (old_order_key, new_order_key)
    pub order_key_change: Option<(String, String)>,

    /// Owner change: (old_owner_id, new_owner_id)
    pub owner_change: Option<(Option<String>, Option<String>)>,

    /// Publish state change: (published_by, published_at_ms)
    pub publish_change: Option<(String, u64)>,

    /// Unpublish (from published to unpublished)
    pub unpublish: bool,
}

/// Detailed changes for a single node
#[derive(Debug, Clone)]
pub struct NodeChanges {
    /// The node ID
    pub node_id: String,

    /// Workspace where the change occurred
    pub workspace: String,

    /// Revision using Hybrid Logical Clock for causal ordering
    pub revision: HLC,

    /// Path of the node (for subscription matching)
    pub path: Option<String>,

    /// Node type (for subscription matching)
    pub node_type: Option<String>,

    /// Full node data (for creates)
    pub node_data: Option<Node>,

    /// Property changes (old → new)
    pub property_changes: Vec<PropertyChange>,

    /// Relation changes (additions and removals)
    pub relation_changes: Vec<RelationChange>,

    /// Node move information
    pub move_info: Option<NodeMove>,

    /// Node metadata field changes
    pub metadata_changes: NodeMetadataChanges,

    /// Whether this is a creation
    pub is_create: bool,

    /// Whether this is a deletion
    pub is_delete: bool,
}

impl NodeChanges {
    pub fn new_create(node_id: String, workspace: String, revision: HLC, node: Node) -> Self {
        Self {
            node_id,
            workspace,
            revision,
            path: Some(node.path.clone()),
            node_type: Some(node.node_type.clone()),
            node_data: Some(node),
            property_changes: Vec::new(),
            relation_changes: Vec::new(),
            move_info: None,
            metadata_changes: NodeMetadataChanges::default(),
            is_create: true,
            is_delete: false,
        }
    }

    pub fn new_delete(
        node_id: String,
        workspace: String,
        revision: HLC,
        path: Option<String>,
        node_type: Option<String>,
        node_snapshot: Option<Node>,
    ) -> Self {
        Self {
            node_id,
            workspace,
            revision,
            path,
            node_type,
            node_data: node_snapshot,
            property_changes: Vec::new(),
            relation_changes: Vec::new(),
            move_info: None,
            metadata_changes: NodeMetadataChanges::default(),
            is_create: false,
            is_delete: true,
        }
    }

    pub fn new_update(
        node_id: String,
        workspace: String,
        revision: HLC,
        path: Option<String>,
        node_type: Option<String>,
    ) -> Self {
        Self {
            node_id,
            workspace,
            revision,
            path,
            node_type,
            node_data: None,
            property_changes: Vec::new(),
            relation_changes: Vec::new(),
            move_info: None,
            metadata_changes: NodeMetadataChanges::default(),
            is_create: false,
            is_delete: false,
        }
    }

    pub fn add_property_change(
        &mut self,
        property_name: String,
        old_value: Option<Value>,
        new_value: Option<Value>,
    ) {
        self.property_changes.push(PropertyChange {
            property_name,
            old_value,
            new_value,
        });
    }

    pub fn add_relation_addition(
        &mut self,
        source_id: String,
        source_workspace: String,
        relation_type: String,
        target_id: String,
        target_workspace: String,
        relation: RelationRef,
    ) {
        self.relation_changes.push(RelationChange {
            source_id,
            source_workspace,
            relation_type,
            target_id,
            target_workspace,
            relation: Some(relation),
            is_addition: true,
        });
    }

    pub fn add_relation_removal(
        &mut self,
        source_id: String,
        source_workspace: String,
        relation_type: String,
        target_id: String,
        target_workspace: String,
    ) {
        self.relation_changes.push(RelationChange {
            source_id,
            source_workspace,
            relation_type,
            target_id,
            target_workspace,
            relation: None,
            is_addition: false,
        });
    }

    pub fn set_move(
        &mut self,
        old_parent_id: Option<String>,
        new_parent_id: Option<String>,
        position: Option<String>,
    ) {
        self.move_info = Some(NodeMove {
            node_id: self.node_id.clone(),
            old_parent_id,
            new_parent_id,
            position,
        });
    }

    pub fn set_name_change(&mut self, old_name: String, new_name: String) {
        self.metadata_changes.name_change = Some((old_name, new_name));
    }

    pub fn set_archetype_change(
        &mut self,
        old_archetype: Option<String>,
        new_archetype: Option<String>,
    ) {
        self.metadata_changes.archetype_change = Some((old_archetype, new_archetype));
    }

    pub fn set_order_key_change(&mut self, old_order_key: String, new_order_key: String) {
        self.metadata_changes.order_key_change = Some((old_order_key, new_order_key));
    }

    pub fn set_owner_change(&mut self, old_owner_id: Option<String>, new_owner_id: Option<String>) {
        self.metadata_changes.owner_change = Some((old_owner_id, new_owner_id));
    }

    pub fn set_publish(&mut self, published_by: String, published_at_ms: u64) {
        self.metadata_changes.publish_change = Some((published_by, published_at_ms));
    }

    pub fn set_unpublish(&mut self) {
        self.metadata_changes.unpublish = true;
    }
}

/// Tracks detailed changes during a transaction
#[derive(Debug, Clone, Default)]
pub struct ChangeTracker {
    /// Map of node_id → detailed changes
    changes: HashMap<String, NodeChanges>,
}

impl ChangeTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn track_create(&mut self, workspace: String, revision: HLC, node: Node) {
        let node_id = node.id.clone();
        self.changes.insert(
            node_id.clone(),
            NodeChanges::new_create(node_id, workspace, revision, node),
        );
    }

    pub fn track_delete(
        &mut self,
        node_id: String,
        workspace: String,
        revision: HLC,
        path: Option<String>,
        node_type: Option<String>,
        node_snapshot: Option<Node>,
    ) {
        self.changes.insert(
            node_id.clone(),
            NodeChanges::new_delete(node_id, workspace, revision, path, node_type, node_snapshot),
        );
    }

    pub fn track_property_change(
        &mut self,
        node_id: String,
        workspace: String,
        revision: HLC,
        property_name: String,
        old_value: Option<Value>,
        new_value: Option<Value>,
        path: Option<String>,
        node_type: Option<String>,
    ) {
        let changes = self.changes.entry(node_id.clone()).or_insert_with(|| {
            NodeChanges::new_update(node_id, workspace, revision, path, node_type)
        });
        changes.add_property_change(property_name, old_value, new_value);
    }

    pub fn track_relation_add(
        &mut self,
        source_id: String,
        source_workspace: String,
        revision: HLC,
        relation_type: String,
        target_id: String,
        target_workspace: String,
        relation: RelationRef,
    ) {
        let changes = self.changes.entry(source_id.clone()).or_insert_with(|| {
            NodeChanges::new_update(
                source_id.clone(),
                source_workspace.clone(),
                revision,
                None,
                None,
            )
        });
        changes.add_relation_addition(
            source_id,
            source_workspace,
            relation_type,
            target_id,
            target_workspace,
            relation,
        );
    }

    pub fn track_relation_remove(
        &mut self,
        source_id: String,
        source_workspace: String,
        revision: HLC,
        relation_type: String,
        target_id: String,
        target_workspace: String,
    ) {
        let changes = self.changes.entry(source_id.clone()).or_insert_with(|| {
            NodeChanges::new_update(
                source_id.clone(),
                source_workspace.clone(),
                revision,
                None,
                None,
            )
        });
        changes.add_relation_removal(
            source_id,
            source_workspace,
            relation_type,
            target_id,
            target_workspace,
        );
    }

    pub fn track_move(
        &mut self,
        node_id: String,
        workspace: String,
        revision: HLC,
        old_parent_id: Option<String>,
        new_parent_id: Option<String>,
        position: Option<String>,
    ) {
        let changes = self.changes.entry(node_id.clone()).or_insert_with(|| {
            NodeChanges::new_update(node_id.clone(), workspace, revision, None, None)
        });
        changes.set_move(old_parent_id, new_parent_id, position);
    }

    pub fn track_name_change(
        &mut self,
        node_id: String,
        workspace: String,
        revision: HLC,
        old_name: String,
        new_name: String,
    ) {
        let changes = self.changes.entry(node_id.clone()).or_insert_with(|| {
            NodeChanges::new_update(node_id.clone(), workspace, revision, None, None)
        });
        changes.set_name_change(old_name, new_name);
    }

    pub fn track_archetype_change(
        &mut self,
        node_id: String,
        workspace: String,
        revision: HLC,
        old_archetype: Option<String>,
        new_archetype: Option<String>,
    ) {
        let changes = self.changes.entry(node_id.clone()).or_insert_with(|| {
            NodeChanges::new_update(node_id.clone(), workspace, revision, None, None)
        });
        changes.set_archetype_change(old_archetype, new_archetype);
    }

    pub fn track_order_key_change(
        &mut self,
        node_id: String,
        workspace: String,
        revision: HLC,
        old_order_key: String,
        new_order_key: String,
    ) {
        let changes = self.changes.entry(node_id.clone()).or_insert_with(|| {
            NodeChanges::new_update(node_id.clone(), workspace, revision, None, None)
        });
        changes.set_order_key_change(old_order_key, new_order_key);
    }

    pub fn track_owner_change(
        &mut self,
        node_id: String,
        workspace: String,
        revision: HLC,
        old_owner_id: Option<String>,
        new_owner_id: Option<String>,
    ) {
        let changes = self.changes.entry(node_id.clone()).or_insert_with(|| {
            NodeChanges::new_update(node_id.clone(), workspace, revision, None, None)
        });
        changes.set_owner_change(old_owner_id, new_owner_id);
    }

    pub fn track_publish(
        &mut self,
        node_id: String,
        workspace: String,
        revision: HLC,
        published_by: String,
        published_at_ms: u64,
    ) {
        let changes = self.changes.entry(node_id.clone()).or_insert_with(|| {
            NodeChanges::new_update(node_id.clone(), workspace, revision, None, None)
        });
        changes.set_publish(published_by, published_at_ms);
    }

    pub fn track_unpublish(&mut self, node_id: String, workspace: String, revision: HLC) {
        let changes = self.changes.entry(node_id.clone()).or_insert_with(|| {
            NodeChanges::new_update(node_id.clone(), workspace, revision, None, None)
        });
        changes.set_unpublish();
    }

    pub fn get_changes(&self) -> &HashMap<String, NodeChanges> {
        &self.changes
    }

    pub fn into_changes(self) -> HashMap<String, NodeChanges> {
        self.changes
    }
}
