//! Workspace, branch, revision metadata, and tag operations

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::workspace::Workspace;
use raisin_replication::Operation;

use super::super::db_helpers::{delete_key, serialize_and_write_compact};
use super::OperationApplicator;

impl OperationApplicator {
    /// Apply a workspace update operation
    pub(in crate::replication::application) async fn apply_update_workspace(
        &self,
        tenant_id: &str,
        repo_id: &str,
        workspace_id: &str,
        workspace: &Workspace,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying workspace update: {}/{}/{} from node {}",
            tenant_id,
            repo_id,
            workspace_id,
            op.cluster_node_id
        );

        let key = keys::workspace_key(tenant_id, repo_id, workspace_id);
        let cf = cf_handle(&self.db, cf::WORKSPACES)?;

        serialize_and_write_compact(
            &self.db,
            cf,
            key,
            workspace,
            &format!(
                "apply_update_workspace_{}/{}/{}",
                tenant_id, repo_id, workspace_id
            ),
        )?;

        tracing::info!(
            "✅ Workspace applied successfully: {}/{}/{}",
            tenant_id,
            repo_id,
            workspace_id
        );
        Ok(())
    }

    /// Apply a workspace delete operation
    pub(in crate::replication::application) async fn apply_delete_workspace(
        &self,
        tenant_id: &str,
        repo_id: &str,
        workspace_id: &str,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying workspace delete: {}/{}/{} from node {}",
            tenant_id,
            repo_id,
            workspace_id,
            op.cluster_node_id
        );

        let key = keys::workspace_key(tenant_id, repo_id, workspace_id);
        let cf = cf_handle(&self.db, cf::WORKSPACES)?;

        delete_key(
            &self.db,
            cf,
            key,
            &format!(
                "apply_delete_workspace_{}/{}/{}",
                tenant_id, repo_id, workspace_id
            ),
        )?;

        tracing::info!(
            "✅ Workspace deleted successfully: {}/{}/{}",
            tenant_id,
            repo_id,
            workspace_id
        );
        Ok(())
    }

    /// Apply a branch update operation
    pub(in crate::replication::application) async fn apply_update_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &raisin_context::Branch,
        op: &Operation,
    ) -> Result<()> {
        let revision = Self::op_revision(op)?;

        tracing::info!(
            "📥 Applying branch update: {}/{}/{} from node {} with revision {}",
            tenant_id,
            repo_id,
            branch.name,
            op.cluster_node_id,
            revision
        );

        let key = keys::branch_key(tenant_id, repo_id, &branch.name);
        let cf = cf_handle(&self.db, cf::BRANCHES)?;

        serialize_and_write_compact(
            &self.db,
            cf,
            key,
            branch,
            &format!(
                "apply_update_branch_{}/{}/{}",
                tenant_id, repo_id, branch.name
            ),
        )?;

        tracing::info!(
            "✅ Branch applied successfully: {}/{}/{}",
            tenant_id,
            repo_id,
            branch.name
        );
        Ok(())
    }

    /// Apply a revision metadata creation operation
    pub(in crate::replication::application) async fn apply_create_revision_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision_meta: &raisin_storage::RevisionMeta,
        op: &Operation,
    ) -> Result<()> {
        let _revision = Self::op_revision(op)?;

        tracing::info!(
            "📥 Applying revision metadata: {}/{} revision={} branch={} from node {}",
            tenant_id,
            repo_id,
            revision_meta.revision,
            revision_meta.branch,
            op.cluster_node_id
        );

        let key = keys::revision_meta_key(tenant_id, repo_id, &revision_meta.revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;

        let value = rmp_serde::to_vec(&revision_meta).map_err(|e| {
            raisin_error::Error::storage(format!("RevisionMeta serialization error: {}", e))
        })?;

        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::info!(
            "✅ Revision metadata applied: {}/{} revision={}",
            tenant_id,
            repo_id,
            revision_meta.revision
        );
        Ok(())
    }

    /// Apply a branch delete operation
    pub(in crate::replication::application) async fn apply_delete_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
        op: &Operation,
    ) -> Result<()> {
        let revision = Self::op_revision(op)?;

        tracing::info!(
            "📥 Applying branch delete: {}/{}/{} from node {} with revision {}",
            tenant_id,
            repo_id,
            branch_id,
            op.cluster_node_id,
            revision
        );

        let key = keys::branch_key(tenant_id, repo_id, branch_id);
        let cf = cf_handle(&self.db, cf::BRANCHES)?;

        self.db
            .delete_cf(cf, key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::info!(
            "✅ Branch deleted successfully: {}/{}/{}",
            tenant_id,
            repo_id,
            branch_id
        );
        Ok(())
    }

    /// Apply a tag creation operation
    pub(in crate::replication::application) async fn apply_create_tag(
        &self,
        tenant_id: &str,
        repo_id: &str,
        tag_name: &str,
        revision: &str,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying tag creation: {}/{}/{} -> {} from node {}",
            tenant_id,
            repo_id,
            tag_name,
            revision,
            op.cluster_node_id
        );

        let key = keys::tag_key(tenant_id, repo_id, tag_name);
        let cf = cf_handle(&self.db, cf::TAGS)?;

        self.db
            .put_cf(cf, key, revision.as_bytes())
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::info!(
            "✅ Tag created successfully: {}/{}/{}",
            tenant_id,
            repo_id,
            tag_name
        );
        Ok(())
    }

    /// Apply a tag deletion operation
    pub(in crate::replication::application) async fn apply_delete_tag(
        &self,
        tenant_id: &str,
        repo_id: &str,
        tag_name: &str,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying tag delete: {}/{}/{} from node {}",
            tenant_id,
            repo_id,
            tag_name,
            op.cluster_node_id
        );

        let key = keys::tag_key(tenant_id, repo_id, tag_name);
        let cf = cf_handle(&self.db, cf::TAGS)?;

        self.db
            .delete_cf(cf, key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::info!(
            "✅ Tag deleted successfully: {}/{}/{}",
            tenant_id,
            repo_id,
            tag_name
        );
        Ok(())
    }
}
