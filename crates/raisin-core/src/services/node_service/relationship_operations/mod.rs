//! Relationship operations for NodeService
//!
//! Provides graph database functionality for managing relationships between nodes:
//! - Add/remove relationships
//! - Query incoming/outgoing relationships
//! - Filter relationships by target type
//! - Cross-workspace relationship support

mod mutations;

use raisin_error::{Error, Result};
use raisin_models::nodes::RelationRef;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{scope::StorageScope, NodeRepository, RelationRepository, Storage};
use serde::{Deserialize, Serialize};

use super::NodeService;

/// Complete relationship information for a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRelationships {
    /// Outgoing relationships FROM this node
    pub outgoing: Vec<RelationRef>,

    /// Incoming relationships TO this node
    /// Each entry is (source_workspace, source_node_id, relation_ref)
    pub incoming: Vec<IncomingRelation>,
}

/// Represents an incoming relationship to a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingRelation {
    /// Workspace containing the source node
    pub source_workspace: String,

    /// ID of the source node
    pub source_node_id: String,

    /// The relationship details
    pub relation: RelationRef,
}

/// Transform node type to Cypher-compatible label format
///
/// Converts node types to valid Cypher labels by removing colons and using camelCase:
/// - "raisin:Folder" -> "RaisinFolder"
/// - "raisin:Page" -> "RaisinPage"
/// - "pageTemplate" -> "PageTemplate"
/// - "Asset" -> "Asset"
///
/// This ensures Cypher queries like `MATCH (a:RaisinPage)` parse correctly.
pub(super) fn transform_node_type_to_cypher_label(node_type: &str) -> String {
    // Split by colon if namespace is present
    let parts: Vec<&str> = node_type.split(':').collect();

    if parts.len() == 2 {
        // Has namespace: "raisin:Folder" -> "RaisinFolder"
        let namespace = parts[0];
        let type_name = parts[1];

        // Capitalize namespace first letter
        let mut namespace_chars = namespace.chars();
        let capitalized_namespace = match namespace_chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + namespace_chars.as_str(),
        };

        // Type name is already capitalized, just concatenate
        format!("{}{}", capitalized_namespace, type_name)
    } else {
        // No namespace: "pageTemplate" -> "PageTemplate"
        let mut chars = node_type.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        }
    }
}

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Get all relationships for a node (both incoming and outgoing)
    ///
    /// Returns a comprehensive view of all relationships connected to this node.
    pub async fn get_node_relationships(&self, node_path: &str) -> Result<NodeRelationships> {
        // Get the node
        let node = self
            .storage
            .nodes()
            .get_by_path(self.scope(), node_path, self.revision.as_ref())
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node not found: {}", node_path)))?;

        // Get outgoing relationships
        let scope = StorageScope::new(
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
            &self.workspace_id,
        );

        let outgoing = self
            .storage
            .relations()
            .get_outgoing_relations(scope, &node.id, self.revision.as_ref())
            .await?;

        // Get incoming relationships
        let incoming_raw = self
            .storage
            .relations()
            .get_incoming_relations(scope, &node.id, self.revision.as_ref())
            .await?;

        // Convert to IncomingRelation format
        let incoming = incoming_raw
            .into_iter()
            .map(
                |(source_workspace, source_node_id, relation)| IncomingRelation {
                    source_workspace,
                    source_node_id,
                    relation,
                },
            )
            .collect();

        Ok(NodeRelationships { outgoing, incoming })
    }

    /// Get outgoing relationships filtered by target node type
    ///
    /// Returns only relationships where the target node is of the specified type.
    /// Useful for finding all relationships to a specific kind of node (e.g., all Pages, all Assets).
    pub async fn get_relationships_by_type(
        &self,
        node_path: &str,
        target_node_type: &str,
    ) -> Result<Vec<RelationRef>> {
        // Get the node
        let node = self
            .storage
            .nodes()
            .get_by_path(self.scope(), node_path, self.revision.as_ref())
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node not found: {}", node_path)))?;

        // Get filtered relationships
        let relations = self
            .storage
            .relations()
            .get_relations_by_type(
                StorageScope::new(
                    &self.tenant_id,
                    &self.repo_id,
                    &self.branch,
                    &self.workspace_id,
                ),
                &node.id,
                target_node_type,
                self.revision.as_ref(),
            )
            .await?;

        Ok(relations)
    }
}
