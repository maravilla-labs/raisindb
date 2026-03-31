//! Node get operations (get by ID, get at specific revision)

use super::super::super::helpers::is_tombstone;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::BranchRepository;

impl NodeRepositoryImpl {
    /// Get a node at HEAD revision
    pub(in crate::repositories::nodes) async fn get_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        populate_has_children: bool,
    ) -> Result<Option<Node>> {
        let blob_revision =
            match self.get_latest_revision_for_node(tenant_id, repo_id, branch, workspace, id)? {
                Some(rev) => rev,
                None => {
                    tracing::info!("REPO get_impl: node_id={} - no revision found", id);
                    return Ok(None);
                }
            };

        // Get branch HEAD for path materialization
        let path_revision = self
            .branch_repo
            .get_head(tenant_id, repo_id, branch)
            .await
            .unwrap_or(blob_revision);

        tracing::trace!(
            "REPO get_impl: node_id={}, blob_revision={}, path_revision={}",
            id,
            blob_revision,
            path_revision
        );

        let key =
            keys::node_key_versioned(tenant_id, repo_id, branch, workspace, id, &blob_revision);
        let cf = cf_handle(&self.db, cf::NODES)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                if is_tombstone(&bytes) {
                    tracing::trace!(
                        "REPO get_impl: node_id={} at revision={} is tombstone",
                        id,
                        blob_revision
                    );
                    return Ok(None);
                }

                let mut node = self.deserialize_node_with_path(
                    &bytes,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    id,
                    &path_revision,
                )?;
                tracing::trace!(
                    "REPO get_impl: node_id={} successfully deserialized, path={}",
                    id,
                    node.path
                );

                if populate_has_children {
                    self.populate_node_has_children(
                        tenant_id, repo_id, branch, workspace, &mut node, None,
                    )
                    .await?;
                }

                Ok(Some(node))
            }
            Ok(None) => {
                tracing::trace!(
                    "REPO get_impl: node_id={} at revision={} - key not found in db",
                    id,
                    blob_revision
                );
                Ok(None)
            }
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    /// Get a node at a specific revision (time-travel)
    pub(in crate::repositories::nodes) async fn get_at_revision_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        target_revision: &HLC,
        populate_has_children: bool,
    ) -> Result<Option<Node>> {
        let revision = match self.get_revision_at_or_before(
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            target_revision,
        )? {
            Some(rev) => rev,
            None => {
                tracing::trace!(
                    "REPO get_at_revision_impl: node_id={} - no revision found at or before {}",
                    id,
                    target_revision
                );
                return Ok(None);
            }
        };

        tracing::trace!(
            "REPO get_at_revision_impl: node_id={}, found_revision={} (target={})",
            id,
            revision,
            target_revision
        );

        let key = keys::node_key_versioned(tenant_id, repo_id, branch, workspace, id, &revision);
        let cf = cf_handle(&self.db, cf::NODES)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                if is_tombstone(&bytes) {
                    tracing::trace!(
                        "REPO get_at_revision_impl: node_id={} at revision={} is tombstone",
                        id,
                        revision
                    );
                    return Ok(None);
                }

                let mut node = self.deserialize_node_with_path(
                    &bytes,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    id,
                    target_revision,
                )?;
                tracing::debug!(
                    "REPO get_at_revision_impl: node_id={} successfully deserialized, path={}",
                    id,
                    node.path
                );

                if populate_has_children {
                    self.populate_node_has_children(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &mut node,
                        Some(target_revision),
                    )
                    .await?;
                }

                Ok(Some(node))
            }
            Ok(None) => {
                tracing::debug!(
                    "REPO get_at_revision_impl: node_id={} at revision={} - key not found in db",
                    id,
                    revision
                );
                Ok(None)
            }
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }
}
