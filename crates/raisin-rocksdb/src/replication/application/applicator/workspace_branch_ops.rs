//! Branch, revision metadata, and tag operations
//!
//! Workspace record application lives in the sibling `workspace_ops` module,
//! because it carries its own last-write-wins guard.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_replication::Operation;

use super::super::db_helpers::serialize_and_write_compact;
use super::OperationApplicator;

impl OperationApplicator {
    /// Apply a branch update operation
    ///
    /// UpdateBranch operations can arrive out of order (network delays, retries),
    /// so this is last-write-wins BY REVISION, not by arrival order: an incoming
    /// head is only applied if it is not older than what's already stored,
    /// otherwise a delayed/duplicate peer message would silently regress head and
    /// hide already-visible local writes (the same class of bug fixed in
    /// `RocksDBTransaction::update_branch_head` for the local commit path).
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

        use raisin_storage::BranchRepository;
        let current_head = self
            .branch_repo
            .get_branch(tenant_id, repo_id, &branch.name)
            .await
            .ok()
            .flatten()
            .map(|b| b.head);

        if let Some(current_head) = current_head {
            if branch.head < current_head {
                tracing::warn!(
                    "⏪ Ignoring older UpdateBranch: incoming {} < current {} ({}/{}/{})",
                    branch.head,
                    current_head,
                    tenant_id,
                    repo_id,
                    branch.name
                );
                return Ok(());
            }
        }

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
