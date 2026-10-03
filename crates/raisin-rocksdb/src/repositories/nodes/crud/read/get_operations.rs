//! Node get operations (get by ID, get at specific revision)

use super::super::super::helpers::is_tombstone;
use super::super::super::storage_node::PropertiesMode;
use super::super::super::NodeRepositoryImpl;
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
        self.get_impl_as(
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            populate_has_children,
            PropertiesMode::Load,
        )
        .await
    }

    /// [`Self::get_impl`], optionally without decoding the properties.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) async fn get_impl_as(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        populate_has_children: bool,
        mode: PropertiesMode,
    ) -> Result<Option<Node>> {
        let (blob_revision, bytes) =
            match self.get_latest_revision_with_blob(tenant_id, repo_id, branch, workspace, id)? {
                Some(found) => found,
                None => {
                    tracing::trace!("REPO get_impl: node_id={} - no revision found", id);
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

        if is_tombstone(&bytes) {
            tracing::trace!(
                "REPO get_impl: node_id={} at revision={} is tombstone",
                id,
                blob_revision
            );
            return Ok(None);
        }

        let mut node = self.deserialize_node_with_path_as(
            &bytes,
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            &path_revision,
            mode,
        )?;
        tracing::trace!(
            "REPO get_impl: node_id={} successfully deserialized, path={}",
            id,
            node.path
        );

        if populate_has_children {
            self.populate_node_has_children(tenant_id, repo_id, branch, workspace, &mut node, None)
                .await?;
        }

        Ok(Some(node))
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
        self.get_at_revision_impl_as(
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            target_revision,
            populate_has_children,
            PropertiesMode::Load,
        )
        .await
    }

    /// [`Self::get_at_revision_impl`], optionally without decoding the
    /// properties.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) async fn get_at_revision_impl_as(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        target_revision: &HLC,
        populate_has_children: bool,
        mode: PropertiesMode,
    ) -> Result<Option<Node>> {
        // The seek that finds the version also yields its blob: no second
        // point read of the same key.
        let (revision, bytes) = match self.get_versioned_at_or_before(
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            target_revision,
        )? {
            Some(found) => found,
            None => {
                tracing::trace!(
                    "REPO get_at_revision_impl: node_id={} - no revision found at or before {}",
                    id,
                    target_revision
                );
                return Ok(None);
            }
        };

        if is_tombstone(&bytes) {
            tracing::trace!(
                "REPO get_at_revision_impl: node_id={} at revision={} is tombstone",
                id,
                revision
            );
            return Ok(None);
        }

        let mut node = self.deserialize_node_with_path_as(
            &bytes,
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            target_revision,
            mode,
        )?;
        tracing::trace!(
            "REPO get_at_revision_impl: node_id={}, found_revision={} (target={}), path={}",
            id,
            revision,
            target_revision,
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
}
