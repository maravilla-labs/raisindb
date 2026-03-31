//! Branch and revision operation handlers for replication
//!
//! This module contains operation handlers for:
//! - apply_update_branch
//! - apply_create_revision_meta
//! - apply_delete_branch

use super::super::OperationApplicator;
use super::db_helpers::serialize_and_write_compact;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_replication::Operation;

/// Apply a branch update operation
pub(super) async fn apply_update_branch(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &raisin_context::Branch,
    op: &Operation,
) -> Result<()> {
    // Validate that the operation has a revision (required for HLC consistency)
    let revision = OperationApplicator::op_revision(op)?;

    tracing::info!(
        "📥 Applying branch update: {}/{}/{} from node {} with revision {}",
        tenant_id,
        repo_id,
        branch.name,
        op.cluster_node_id,
        revision
    );

    let key = keys::branch_key(tenant_id, repo_id, &branch.name);
    let cf = cf_handle(&applicator.db, cf::BRANCHES)?;

    serialize_and_write_compact(
        &applicator.db,
        cf,
        key,
        branch,
        &format!(
            "apply_update_branch_{}/{}/{}",
            tenant_id, repo_id, branch.name
        ),
    )?;

    // CRITICAL: Update the cached branch head to make newly replicated data visible
    // But ONLY if the incoming head is newer than the current head (LWW + out-of-order delivery)
    // UpdateBranch operations can arrive out of order due to network delays, so we must
    // protect against older operations rolling back the branch head
    use raisin_storage::BranchRepository;

    // Get current branch head from cache/DB
    if let Ok(Some(current_branch)) = applicator
        .branch_repo
        .get_branch(tenant_id, repo_id, &branch.name)
        .await
    {
        // Only update if incoming head is newer (or equal, for idempotency)
        if branch.head >= current_branch.head {
            tracing::debug!(
                "🔄 Updating branch head from {} to {}",
                current_branch.head,
                branch.head
            );
            applicator
                .branch_repo
                .update_head(tenant_id, repo_id, &branch.name, branch.head)
                .await?;
        } else {
            tracing::warn!(
                "⏪ Ignoring older UpdateBranch: incoming {} < current {}",
                branch.head,
                current_branch.head
            );
        }
    } else {
        // Branch doesn't exist yet, safe to update
        applicator
            .branch_repo
            .update_head(tenant_id, repo_id, &branch.name, branch.head)
            .await?;
    }

    tracing::info!(
        "✅ Branch applied successfully: {}/{}/{} with head {}",
        tenant_id,
        repo_id,
        branch.name,
        branch.head
    );
    Ok(())
}

/// Apply a revision metadata creation operation
///
/// This writes the RevisionMeta record that populates the revision history log
pub(super) async fn apply_create_revision_meta(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    revision_meta: &raisin_storage::RevisionMeta,
    op: &Operation,
) -> Result<()> {
    // Validate that the operation has a revision
    let _revision = OperationApplicator::op_revision(op)?;

    tracing::info!(
        "📥 Applying revision metadata: {}/{} revision={} branch={} from node {}",
        tenant_id,
        repo_id,
        revision_meta.revision,
        revision_meta.branch,
        op.cluster_node_id
    );

    let key = keys::revision_meta_key(tenant_id, repo_id, &revision_meta.revision);
    let cf = cf_handle(&applicator.db, cf::REVISIONS)?;

    let value = rmp_serde::to_vec(&revision_meta).map_err(|e| {
        raisin_error::Error::storage(format!("RevisionMeta serialization error: {}", e))
    })?;

    applicator
        .db
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
pub(super) async fn apply_delete_branch(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch_id: &str,
    op: &Operation,
) -> Result<()> {
    // Validate that the operation has a revision
    let revision = OperationApplicator::op_revision(op)?;

    tracing::info!(
        "📥 Applying branch delete: {}/{}/{} from node {} with revision {}",
        tenant_id,
        repo_id,
        branch_id,
        op.cluster_node_id,
        revision
    );

    let key = keys::branch_key(tenant_id, repo_id, branch_id);
    let cf = cf_handle(&applicator.db, cf::BRANCHES)?;

    applicator
        .db
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
