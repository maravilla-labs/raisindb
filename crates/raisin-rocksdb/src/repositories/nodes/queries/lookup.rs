//! Node lookup operations by path
//!
//! This module provides functions for looking up and deleting nodes by their path.

use super::super::storage_node::PropertiesMode;
use super::super::NodeRepositoryImpl;
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
        // Public API - populate has_children for frontend display
        self.get_by_path_impl_as(
            tenant_id,
            repo_id,
            branch,
            workspace,
            path,
            max_revision,
            true,
            PropertiesMode::Load,
        )
        .await
    }

    /// [`Self::get_by_path_impl`] populating `has_children` only when asked
    /// and decoding properties per `mode` (`NodeRepository::get_for_read`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn get_by_path_impl_as(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        max_revision: Option<&HLC>,
        populate_has_children: bool,
        mode: PropertiesMode,
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

        match max_revision {
            Some(rev) => {
                self.get_at_revision_impl_as(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node_id,
                    rev,
                    populate_has_children,
                    mode,
                )
                .await
            }
            None => {
                self.get_impl_as(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node_id,
                    populate_has_children,
                    mode,
                )
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
        // THE PATH_INDEX step, shared with the batched reader.
        let entry = crate::mvcc_read::path_index_entry_in(
            &mut crate::mvcc_read::DbRead(&self.db),
            tenant_id,
            repo_id,
            branch,
            workspace,
            path,
            max_revision,
        )?
        .map(|(_, id)| id);

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
