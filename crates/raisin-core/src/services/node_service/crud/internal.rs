//! Internal helper methods for CRUD operations
//!
//! Contains put_without_versioning and other internal helpers.

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::{
    scope::StorageScope, transactional::TransactionalStorage, NodeRepository, Storage,
};

use super::super::NodeService;
use crate::sanitize_name;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Internal: Put node without triggering versioning
    ///
    /// Used by publish() to avoid double-versioning. The publish workflow is:
    /// 1. Create version of draft state (explicit versioning)
    /// 2. Modify node to published state
    /// 3. Store using this method (skips automatic versioning)
    ///
    /// This ensures we version the DRAFT state, not the already-published state.
    pub(crate) async fn put_without_versioning(&self, mut node: models::nodes::Node) -> Result<()> {
        // Same validation as put(), but skip versioning at the end

        // CRITICAL: Check if node exists and prevent NodeType changes
        if let Some(existing) = self
            .storage
            .nodes()
            .get(self.scope(), &node.id, self.revision.as_ref())
            .await?
        {
            if existing.node_type != node.node_type {
                return Err(raisin_error::Error::Validation(format!(
                    "Cannot change node_type from '{}' to '{}'. NodeType changes are not allowed.",
                    existing.node_type, node.node_type
                )));
            }
        }

        // Validate NodeType exists and validate node against schema
        // NOTE: RocksDB transaction layer also validates, but InMemoryStorage doesn't,
        // so we validate here as well for consistent behavior across backends.
        self.validator
            .validate_node_type_exists(&node.node_type)
            .await?;
        self.validator
            .validate_node(&self.workspace_id, &node)
            .await?;

        // Validate and sanitize path
        if node.path.is_empty() {
            return Err(raisin_error::Error::Validation(
                "Node path cannot be empty".to_string(),
            ));
        }

        if let Some((parent, leaf)) = node.path.rsplit_once('/') {
            if leaf.is_empty() {
                return Err(raisin_error::Error::Validation(
                    "Node path cannot end with '/'".to_string(),
                ));
            }
            let clean = sanitize_name(leaf)?;
            node.name = clean.clone();
            node.path = if parent.is_empty() || parent == "/" {
                format!("/{}", clean)
            } else {
                format!("{}/{}", parent, clean)
            };
        } else {
            return Err(raisin_error::Error::Validation(
                "Node path must contain at least one '/' separator".to_string(),
            ));
        }
        if let Some(ws) = &node.workspace {
            if ws != &self.workspace_id {
                node.workspace = Some(self.workspace_id.clone());
            }
        } else {
            node.workspace = Some(self.workspace_id.clone());
        }
        // CRITICAL: Write to workspace delta (branch-specific draft), NOT committed storage!
        self.storage
            .put_workspace_delta(
                StorageScope::new(
                    &self.tenant_id,
                    &self.repo_id,
                    &self.branch,
                    &self.workspace_id,
                ),
                &node,
            )
            .await?;

        // Check if this is a root node based on PATH, not parent field
        let is_root = match node.parent_path() {
            Some(pp) => pp == "/",
            None => true,
        };

        // If this is a root-level node, update the ROOT node's children array in workspace delta
        if is_root {
            // Try workspace delta first
            let root_node_opt = self
                .storage
                .get_workspace_delta(
                    StorageScope::new(
                        &self.tenant_id,
                        &self.repo_id,
                        &self.branch,
                        &self.workspace_id,
                    ),
                    "/",
                )
                .await?;

            // Fallback to committed storage if not in delta
            let root_node_opt = if root_node_opt.is_none() {
                self.storage
                    .nodes()
                    .get_by_path(self.scope(), "/", self.revision.as_ref())
                    .await?
            } else {
                root_node_opt
            };

            if let Some(mut root_node) = root_node_opt {
                if !root_node.children.contains(&node.id) {
                    root_node.children.push(node.id.clone());
                    self.storage
                        .put_workspace_delta(
                            StorageScope::new(
                                &self.tenant_id,
                                &self.repo_id,
                                &self.branch,
                                &self.workspace_id,
                            ),
                            &root_node,
                        )
                        .await?;
                }
            }
        }

        // Audit is still performed (we want to track the publish operation)
        // But versioning is SKIPPED (that's the whole point of this method)

        Ok(())
    }
}
