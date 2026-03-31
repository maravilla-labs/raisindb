//! Read operations for NodeService
//!
//! Contains get and get_by_path methods for retrieving nodes.

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::{
    scope::StorageScope, transactional::TransactionalStorage, NodeRepository, Storage,
};

use super::super::NodeService;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Retrieves a node by its ID.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(node))` if the node exists and the user has read permission
    /// - `Ok(None)` if the node does not exist or the user doesn't have permission
    /// - `Err(...)` if there was a storage error
    pub async fn get(&self, id: &str) -> Result<Option<models::nodes::Node>> {
        // CRITICAL: Check workspace delta first (branch-specific drafts)
        // Note: Workspace deltas are always at HEAD, ignore self.revision for deltas
        if self.revision.is_none() {
            if let Some(draft) = self
                .storage
                .get_workspace_delta_by_id(
                    StorageScope::new(
                        &self.tenant_id,
                        &self.repo_id,
                        &self.branch,
                        &self.workspace_id,
                    ),
                    id,
                )
                .await?
            {
                // Apply RLS filtering to draft node
                return Ok(self.apply_rls_filter(draft));
            }
        }

        // Query committed storage with optional revision bound
        // MVCC indexes handle time-travel naturally - no separate snapshots needed
        let result = self
            .storage
            .nodes()
            .get(self.scope(), id, self.revision.as_ref())
            .await?;

        // Apply RLS filtering
        Ok(result.and_then(|node| self.apply_rls_filter(node)))
    }

    /// Gets a node by its path
    ///
    /// Returns None if the node doesn't exist or the user doesn't have read permission.
    pub async fn get_by_path(&self, path: &str) -> Result<Option<models::nodes::Node>> {
        // MVCC snapshot isolation: when self.revision is set (via at_revision()),
        // the repository layer will filter results to revision <= self.revision
        // This enables consistent reads at branch HEAD or historical snapshots

        tracing::debug!(
            "SERVICE get_by_path: path={}, max_revision={:?}",
            path,
            self.revision
        );

        // CRITICAL: Check workspace delta first (branch-specific drafts)
        if let Some(draft) = self
            .storage
            .get_workspace_delta(
                StorageScope::new(
                    &self.tenant_id,
                    &self.repo_id,
                    &self.branch,
                    &self.workspace_id,
                ),
                path,
            )
            .await?
        {
            tracing::debug!(
                target: "node_service::workspace_delta",
                "workspace delta hit: repo={} branch={} workspace={} path={} revision={:?}",
                self.repo_id,
                self.branch,
                self.workspace_id,
                path,
                self.revision
            );
            // Apply RLS filtering to draft node
            return Ok(self.apply_rls_filter(draft));
        }

        // Fall back to committed storage (repository-scoped)
        // Pass self.revision for MVCC filtering
        let result = self
            .storage
            .nodes()
            .get_by_path(self.scope(), path, self.revision.as_ref())
            .await?;

        if let Some(node) = result.as_ref() {
            tracing::debug!(
                target: "node_service::get_by_path",
                "storage hit: repo={} branch={} workspace={} path={} revision={:?} node_id={}",
                self.repo_id,
                self.branch,
                self.workspace_id,
                path,
                self.revision,
                node.id
            );
        } else {
            tracing::warn!(
                target: "node_service::get_by_path",
                "storage miss: repo={} branch={} workspace={} path={} revision={:?}",
                self.repo_id,
                self.branch,
                self.workspace_id,
                path,
                self.revision
            );
        }

        // Apply RLS filtering
        Ok(result.and_then(|node| self.apply_rls_filter(node)))
    }
}
