//! Relationship mutation operations: add and remove relations.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::RelationRef;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    scope::StorageScope, BranchRepository, NodeRepository, RelationRepository, Storage,
};
use serde_json::json;
use std::collections::HashMap;

use super::transform_node_type_to_cypher_label;
use crate::services::node_service::NodeService;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Add a relationship from source node to target node
    ///
    /// Creates a directed relationship from the source node (at source_path) to a target node
    /// in the specified workspace. Supports cross-workspace relationships.
    pub async fn add_relation(
        &self,
        source_path: &str,
        target_workspace: &str,
        target_path: &str,
        weight: Option<f32>,
        relation_type: Option<String>,
    ) -> Result<()> {
        // Get the source node to verify it exists and get its ID
        let source_node = self
            .storage
            .nodes()
            .get_by_path(self.scope(), source_path, self.revision.as_ref())
            .await?
            .ok_or_else(|| Error::NotFound(format!("Source node not found: {}", source_path)))?;

        // Get the target node to verify it exists and get its ID and type
        let target_scope = raisin_storage::StorageScope::new(
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
            target_workspace,
        );
        let target_node = self
            .storage
            .nodes()
            .get_by_path(target_scope, target_path, self.revision.as_ref())
            .await?
            .ok_or_else(|| Error::NotFound(format!("Target node not found: {}", target_path)))?;

        // Create the relation with semantic type "references" as default
        let rel_type = relation_type.unwrap_or_else(|| "references".to_string());

        // Transform node types to Cypher-compatible labels (no colons)
        let source_cypher_label = transform_node_type_to_cypher_label(&source_node.node_type);
        let target_cypher_label = transform_node_type_to_cypher_label(&target_node.node_type);

        let relation = RelationRef::new(
            target_node.id.clone(),
            target_workspace.to_string(),
            target_cypher_label.clone(),
            rel_type,
            weight,
        );

        // Add the relationship within a transaction for proper replication tracking
        let ctx = self.storage.begin_context().await?;

        // Set auth context for RLS enforcement
        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;
        ctx.set_actor("system")?;
        ctx.set_message(&format!(
            "Added relation '{}' from {} to {}",
            relation.relation_type, source_path, target_path
        ))?;

        ctx.add_relation(
            &self.workspace_id,
            &source_node.id,
            &source_cypher_label,
            relation.clone(),
        )
        .await?;

        ctx.commit().await?;

        // Get the current revision after adding the relationship
        let current_revision = self
            .storage
            .branches()
            .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
            .await?
            .map(|b| b.head)
            .unwrap_or_else(|| HLC::new(0, 0));

        // Emit RelationAdded event for source (outgoing)
        let mut outgoing_meta = HashMap::new();
        outgoing_meta.insert("related_node_id".to_string(), json!(target_node.id.clone()));
        outgoing_meta.insert(
            "related_workspace".to_string(),
            json!(target_workspace.to_string()),
        );
        outgoing_meta.insert("direction".to_string(), json!("outgoing"));

        self.storage
            .event_bus()
            .publish(raisin_storage::Event::Node(raisin_storage::NodeEvent {
                tenant_id: self.tenant_id.clone(),
                repository_id: self.repo_id.clone(),
                branch: self.branch.clone(),
                workspace_id: self.workspace_id.clone(),
                node_id: source_node.id.clone(),
                node_type: Some(source_node.node_type.clone()),
                revision: current_revision,
                kind: raisin_storage::NodeEventKind::RelationAdded {
                    relation_type: relation.relation_type.clone(),
                    target_node_id: target_node.id.clone(),
                },
                path: Some(source_node.path.clone()),
                metadata: Some(outgoing_meta),
            }));

        // Emit RelationAdded event for target (incoming)
        let mut incoming_meta = HashMap::new();
        incoming_meta.insert("related_node_id".to_string(), json!(source_node.id.clone()));
        incoming_meta.insert(
            "related_workspace".to_string(),
            json!(self.workspace_id.clone()),
        );
        incoming_meta.insert("direction".to_string(), json!("incoming"));

        self.storage
            .event_bus()
            .publish(raisin_storage::Event::Node(raisin_storage::NodeEvent {
                tenant_id: self.tenant_id.clone(),
                repository_id: self.repo_id.clone(),
                branch: self.branch.clone(),
                workspace_id: target_workspace.to_string(),
                node_id: target_node.id.clone(),
                node_type: Some(target_node.node_type.clone()),
                revision: current_revision,
                kind: raisin_storage::NodeEventKind::RelationAdded {
                    relation_type: relation.relation_type.clone(),
                    target_node_id: target_node.id.clone(),
                },
                path: Some(target_node.path.clone()),
                metadata: Some(incoming_meta),
            }));

        Ok(())
    }

    /// Remove a relationship between two nodes
    ///
    /// Removes the directed relationship from source to target.
    /// Returns `true` if the relationship existed and was removed, `false` if it didn't exist.
    pub async fn remove_relation(
        &self,
        source_path: &str,
        target_workspace: &str,
        target_path: &str,
    ) -> Result<bool> {
        // Get the source node
        let source_node = self
            .storage
            .nodes()
            .get_by_path(self.scope(), source_path, self.revision.as_ref())
            .await?
            .ok_or_else(|| Error::NotFound(format!("Source node not found: {}", source_path)))?;

        // Get the target node
        let target_scope = raisin_storage::StorageScope::new(
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
            target_workspace,
        );
        let target_node = self
            .storage
            .nodes()
            .get_by_path(target_scope, target_path, self.revision.as_ref())
            .await?
            .ok_or_else(|| Error::NotFound(format!("Target node not found: {}", target_path)))?;

        // Get outgoing relations to determine the relation_type
        let outgoing_rels = self
            .storage
            .relations()
            .get_outgoing_relations(
                StorageScope::new(
                    &self.tenant_id,
                    &self.repo_id,
                    &self.branch,
                    &self.workspace_id,
                ),
                &source_node.id,
                self.revision.as_ref(),
            )
            .await?;

        // Find the relation type for the target node before removal
        let relation_type = outgoing_rels
            .iter()
            .find(|rel| rel.target == target_node.id)
            .map(|rel| rel.relation_type.clone())
            .unwrap_or_else(|| "references".to_string());

        // Remove the relationship within a transaction for proper replication tracking
        let ctx = self.storage.begin_context().await?;

        // Set auth context for RLS enforcement
        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;
        ctx.set_actor("system")?;
        ctx.set_message(&format!(
            "Removed relation '{}' from {} to {}",
            relation_type, source_path, target_path
        ))?;

        let removed = ctx
            .remove_relation(
                &self.workspace_id,
                &source_node.id,
                target_workspace,
                &target_node.id,
            )
            .await?;

        ctx.commit().await?;

        // Only emit event if relationship was actually removed
        if removed {
            // Get the current revision after removing the relationship
            let current_revision = self
                .storage
                .branches()
                .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
                .await?
                .map(|b| b.head)
                .unwrap_or_else(|| HLC::new(0, 0));

            // Emit RelationRemoved event for source (outgoing)
            let mut outgoing_meta = HashMap::new();
            outgoing_meta.insert("related_node_id".to_string(), json!(target_node.id.clone()));
            outgoing_meta.insert("related_workspace".to_string(), json!(target_workspace));
            outgoing_meta.insert("direction".to_string(), json!("outgoing"));

            self.storage
                .event_bus()
                .publish(raisin_storage::Event::Node(raisin_storage::NodeEvent {
                    tenant_id: self.tenant_id.clone(),
                    repository_id: self.repo_id.clone(),
                    branch: self.branch.clone(),
                    workspace_id: self.workspace_id.clone(),
                    node_id: source_node.id.clone(),
                    node_type: Some(source_node.node_type.clone()),
                    revision: current_revision,
                    kind: raisin_storage::NodeEventKind::RelationRemoved {
                        relation_type: relation_type.clone(),
                        target_node_id: target_node.id.clone(),
                    },
                    path: Some(source_node.path.clone()),
                    metadata: Some(outgoing_meta),
                }));

            // Emit RelationRemoved event for target (incoming)
            let mut incoming_meta = HashMap::new();
            incoming_meta.insert("related_node_id".to_string(), json!(source_node.id.clone()));
            incoming_meta.insert(
                "related_workspace".to_string(),
                json!(self.workspace_id.clone()),
            );
            incoming_meta.insert("direction".to_string(), json!("incoming"));

            self.storage
                .event_bus()
                .publish(raisin_storage::Event::Node(raisin_storage::NodeEvent {
                    tenant_id: self.tenant_id.clone(),
                    repository_id: self.repo_id.clone(),
                    branch: self.branch.clone(),
                    workspace_id: target_workspace.to_string(),
                    node_id: target_node.id.clone(),
                    node_type: Some(target_node.node_type.clone()),
                    revision: current_revision,
                    kind: raisin_storage::NodeEventKind::RelationRemoved {
                        relation_type,
                        target_node_id: target_node.id.clone(),
                    },
                    path: Some(target_node.path.clone()),
                    metadata: Some(incoming_meta),
                }));
        }

        Ok(removed)
    }
}
