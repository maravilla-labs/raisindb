//! Node lookup operations by path
//!
//! This module provides functions for looking up and deleting nodes by their path.

use super::super::helpers::is_tombstone;
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// Get node by path using PATH_INDEX
    pub(crate) async fn get_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<Node>> {
        tracing::debug!(
            "REPO get_by_path_impl: tenant={}, repo={}, branch={}, workspace={}, path={}",
            tenant_id,
            repo_id,
            branch,
            workspace,
            path
        );

        let Some(node_id) =
            self.resolve_path_index(tenant_id, repo_id, branch, workspace, path, max_revision)?
        else {
            return Ok(None);
        };

        // Public API - populate has_children for frontend display
        match max_revision {
            Some(rev) => {
                self.get_at_revision_impl(
                    tenant_id, repo_id, branch, workspace, &node_id, rev, true,
                )
                .await
            }
            None => {
                self.get_impl(tenant_id, repo_id, branch, workspace, &node_id, true)
                    .await
            }
        }
    }

    /// Get node ID by path using PATH_INDEX without loading the full node
    pub(crate) async fn get_node_id_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<String>> {
        self.resolve_path_index(tenant_id, repo_id, branch, workspace, path, max_revision)
    }

    /// The node id a path names at `max_revision` (HEAD when `None`).
    ///
    /// MVCC: PATH_INDEX keys run newest first, so the relevant entry is the
    /// newest one at or before `max_revision` — found with one seek rather
    /// than by walking every newer entry. If that entry is a tombstone the
    /// path was deleted or moved away by then, and the answer is `None`.
    fn resolve_path_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<String>> {
        let prefix = keys::path_index_key_prefix(tenant_id, repo_id, branch, workspace, path);
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;

        let entry = crate::mvcc_read::newest_at_or_before_with(
            &self.db,
            cf_path,
            &prefix,
            max_revision,
            |_, bytes| (!is_tombstone(bytes)).then(|| String::from_utf8_lossy(bytes).to_string()),
        )?;

        match entry {
            Some(Some(node_id)) => {
                tracing::trace!(
                    "REPO resolve_path_index: path={} -> node_id={}",
                    path,
                    node_id
                );
                Ok(Some(node_id))
            }
            Some(None) => {
                tracing::trace!("REPO resolve_path_index: path={} is deleted/moved", path);
                Ok(None)
            }
            None => {
                tracing::trace!("REPO resolve_path_index: no entries for path={}", path);
                Ok(None)
            }
        }
    }

    /// Delete node by path
    pub(in crate::repositories::nodes) async fn delete_by_path_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) -> Result<bool> {
        // Always use HEAD for delete operations (no max_revision)
        let node = match self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, path, None)
            .await?
        {
            Some(n) => n,
            None => return Ok(false),
        };

        self.delete_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.id,
            crate::repositories::nodes::WriteAttribution::default(),
        )
        .await
    }
}
