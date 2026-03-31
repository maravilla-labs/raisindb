//! Transactional operations for NodeService
//!
//! This module provides transactional versions of multi-step operations
//! to ensure atomicity and consistency.

use raisin_error::Result;
use raisin_models as models;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{scope::BranchScope, transactional::TransactionalStorage, NodeTypeRepository};
use std::collections::HashMap;

use crate::services::transaction::Transaction;
use crate::NodeService;

impl<S> NodeService<S>
where
    S: TransactionalStorage,
{
    /// Create a new transaction for atomic multi-node operations
    ///
    /// Returns a Transaction builder that can accumulate operations
    /// and commit them all at once, creating a single repository revision.
    ///
    /// The transaction inherits the auth context from this NodeService instance,
    /// ensuring RLS enforcement during commit.
    ///
    /// # Example
    /// ```rust,ignore
    /// let mut tx = nodes_svc.transaction();
    /// tx.create(node1);
    /// tx.update(node2_id, props);
    /// tx.delete(node3_id);
    /// tx.commit("Bulk update", "user-123").await?;
    /// ```
    pub fn transaction(&self) -> Transaction<S> {
        let mut tx = Transaction::new(
            self.storage.clone(),
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            self.workspace_id.clone(),
        );
        // Pass auth context from NodeService to Transaction for RLS enforcement
        if let Some(auth) = &self.auth_context {
            tx = tx.with_auth_context(auth.clone());
        }
        tx
    }

    /// Add a node with initial structure in a transaction
    ///
    /// This ensures that if creating the initial structure fails,
    /// the entire operation is rolled back including the parent node.
    pub async fn add_node_transactional(
        &self,
        parent_path: &str,
        mut node: models::nodes::Node,
    ) -> Result<models::nodes::Node> {
        // Start a transaction
        let ctx = self.storage.begin_context().await?;

        // Set auth context for RLS enforcement
        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        // Generate ID if needed
        if node.id.is_empty() {
            node.id = nanoid::nanoid!();
        }

        // Set path (parent will be auto-derived by storage layer)
        if parent_path == "/" || parent_path.is_empty() {
            node.path = format!("/{}", node.name);
        } else {
            node.path = format!("{}/{}", parent_path, node.name);
        }
        // Do NOT set node.parent - storage layer will derive it from node.path

        // Save the parent node in transaction
        ctx.put_node(&self.workspace_id, &node).await?;

        // Handle initial structure if present
        if let Some(node_type) = self
            .storage
            .node_types()
            .get(
                BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                &node.node_type,
                None,
            )
            .await?
        {
            if let Some(initial_structure) = &node_type.initial_structure {
                if let Some(children_defs) = &initial_structure.children {
                    for child_def in children_defs {
                        let child = self.create_node_from_initial_child(&node.path, child_def)?;

                        // Save child in transaction
                        ctx.put_node(&self.workspace_id, &child).await?;
                    }
                }
            }
        }

        // Update ROOT node's children if this is a root-level node
        if node.parent.is_none() {
            if let Some(mut root_node) = ctx.get_node_by_path(&self.workspace_id, "/").await? {
                if !root_node.children.contains(&node.id) {
                    root_node.children.push(node.id.clone());
                    ctx.put_node(&self.workspace_id, &root_node).await?;
                }
            }
        }

        // Commit the transaction
        ctx.commit().await?;

        // CRITICAL: Derive parent NAME from path before returning
        // Storage layer auto-derives this during save, but we need to sync the in-memory node
        node.parent = models::nodes::Node::extract_parent_name_from_path(&node.path);

        Ok(node)
    }

    /// Delete a node and update parent references in a transaction
    pub async fn delete_node_transactional(&self, node_id: &str) -> Result<bool> {
        let ctx = self.storage.begin_context().await?;

        // Set auth context for RLS enforcement
        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        // Check if node exists
        let node = match ctx.get_node(&self.workspace_id, node_id).await? {
            Some(n) => n,
            None => return Ok(false),
        };

        // Delete the node
        ctx.delete_node(&self.workspace_id, node_id).await?;

        // Update parent's children list or ROOT node
        if let Some(_parent_path) = &node.parent {
            // TODO: Update parent node's children list
        } else {
            // Update ROOT node's children
            if let Some(mut root_node) = ctx.get_node_by_path(&self.workspace_id, "/").await? {
                root_node.children.retain(|id| id != &node.id);
                ctx.put_node(&self.workspace_id, &root_node).await?;
            }
        }

        // Commit transaction
        ctx.commit().await?;

        Ok(true)
    }

    /// Move a node to a new location in a transaction
    pub async fn move_node_transactional(
        &self,
        node_id: &str,
        new_parent_path: &str,
        new_name: Option<&str>,
    ) -> Result<()> {
        let ctx = self.storage.begin_context().await?;

        // Set auth context for RLS enforcement
        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        // Get the node
        let mut node = match ctx.get_node(&self.workspace_id, node_id).await? {
            Some(n) => n,
            None => {
                return Err(raisin_error::Error::NotFound(format!(
                    "Node {} not found",
                    node_id
                )))
            }
        };

        let _old_path = node.path.clone();
        let old_parent = node.parent.clone();

        // Update node path and parent (parent will be auto-derived by storage layer)
        let name = new_name.unwrap_or(&node.name);
        if new_parent_path == "/" || new_parent_path.is_empty() {
            node.path = format!("/{}", name);
        } else {
            node.path = format!("{}/{}", new_parent_path, name);
        }
        node.name = name.to_string();
        // Do NOT set node.parent - storage layer will derive it from node.path

        // Save updated node
        ctx.put_node(&self.workspace_id, &node).await?;

        // Update all children paths recursively
        // TODO: This would need to recursively update all descendant nodes

        // Update old parent's children list or ROOT node
        if old_parent.is_none() {
            if let Some(mut root_node) = ctx.get_node_by_path(&self.workspace_id, "/").await? {
                root_node.children.retain(|id| id != &node.id);
                ctx.put_node(&self.workspace_id, &root_node).await?;
            }
        }

        // Update new parent's children list or ROOT node
        if node.parent.is_none() {
            if let Some(mut root_node) = ctx.get_node_by_path(&self.workspace_id, "/").await? {
                if !root_node.children.contains(&node.id) {
                    root_node.children.push(node.id.clone());
                    ctx.put_node(&self.workspace_id, &root_node).await?;
                }
            }
        }

        // Commit transaction
        ctx.commit().await?;

        Ok(())
    }

    /// Helper to create a node from initial child definition
    fn create_node_from_initial_child(
        &self,
        parent_path: &str,
        child_def: &models::nodes::types::initial_structure::InitialChild,
    ) -> Result<models::nodes::Node> {
        // Convert properties from serde_json::Value to PropertyValue
        let properties = if let Some(props) = &child_def.properties {
            props
                .iter()
                .map(|(k, v)| {
                    // Convert serde_json::Value to PropertyValue
                    let property_value = match v {
                        serde_json::Value::String(s) => PropertyValue::String(s.clone()),
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                PropertyValue::Integer(i)
                            } else if let Some(f) = n.as_f64() {
                                PropertyValue::Float(f)
                            } else {
                                PropertyValue::String(n.to_string())
                            }
                        }
                        serde_json::Value::Bool(b) => PropertyValue::Boolean(*b),
                        serde_json::Value::Array(_arr) => {
                            // For now, just convert to string representation
                            PropertyValue::String(serde_json::to_string(v).unwrap_or_default())
                        }
                        serde_json::Value::Object(_) => {
                            // For now, just convert to string representation
                            PropertyValue::String(serde_json::to_string(v).unwrap_or_default())
                        }
                        serde_json::Value::Null => PropertyValue::String(String::new()),
                    };
                    (k.clone(), property_value)
                })
                .collect()
        } else {
            HashMap::new()
        };

        let node = models::nodes::Node {
            id: nanoid::nanoid!(),
            name: crate::sanitize_name(&child_def.name)?,
            path: format!("{}/{}", parent_path, crate::sanitize_name(&child_def.name)?),
            node_type: child_def.node_type.clone(),
            archetype: child_def.archetype.clone(),
            properties,
            children: vec![],
            order_key: String::new(), // Will be assigned by storage layer
            has_children: None,       // Computed at service layer
            parent: None,             // Will be auto-derived by storage layer from path
            version: 1,
            created_at: Some(chrono::Utc::now()),
            updated_at: None,
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: child_def.translations.as_ref().map(|trans| {
                trans
                    .iter()
                    .map(|(lang, val)| {
                        // Convert serde_json::Value to PropertyValue
                        let property_value = match val {
                            serde_json::Value::String(s) => PropertyValue::String(s.clone()),
                            serde_json::Value::Number(n) => {
                                if let Some(i) = n.as_i64() {
                                    PropertyValue::Integer(i)
                                } else if let Some(f) = n.as_f64() {
                                    PropertyValue::Float(f)
                                } else {
                                    PropertyValue::String(n.to_string())
                                }
                            }
                            serde_json::Value::Bool(b) => PropertyValue::Boolean(*b),
                            _ => PropertyValue::String(
                                serde_json::to_string(val).unwrap_or_default(),
                            ),
                        };
                        (lang.clone(), property_value)
                    })
                    .collect()
            }),
            tenant_id: None,
            workspace: Some(self.workspace_id.clone()),
            owner_id: None,
            relations: Vec::new(),
        };

        // Recursively create nested children if present
        if let Some(nested_children) = &child_def.children {
            for nested_child in nested_children {
                let _ = self.create_node_from_initial_child(&node.path, nested_child)?;
            }
        }

        Ok(node)
    }
}
