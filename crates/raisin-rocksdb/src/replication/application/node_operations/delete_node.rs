//! DeleteNode operation handler

use super::super::OperationApplicator;
use raisin_error::Result;
use raisin_events::NodeEventKind;
use raisin_replication::Operation;
use raisin_storage::BranchRepository;

/// Apply a DeleteNode operation
pub(in crate::replication::application) async fn apply_delete_node(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    op: &Operation,
) -> Result<()> {
    let revision = OperationApplicator::op_revision(op)?;
    tracing::info!(
        "Applying DeleteNode: {}/{}/{}/{} from node {} at revision {}",
        tenant_id,
        repo_id,
        branch,
        node_id,
        op.cluster_node_id,
        revision
    );

    let node_snapshot = match applicator.load_latest_node(tenant_id, repo_id, branch, node_id)? {
        Some(node) => node,
        None => {
            tracing::warn!(
                "DeleteNode skipped: node {} not found when applying replication delete",
                node_id
            );
            return Ok(());
        }
    };

    let workspace = node_snapshot.workspace.as_deref().unwrap_or("default");
    let parent_id = applicator.resolve_parent_id_for_snapshot(
        tenant_id,
        repo_id,
        branch,
        workspace,
        &node_snapshot,
    )?;

    super::super::replication_core::apply_replicated_delete(
        applicator,
        tenant_id,
        repo_id,
        branch,
        workspace,
        &node_snapshot,
        parent_id.as_deref(),
        &revision,
    )?;

    // Update branch HEAD
    applicator
        .branch_repo
        .update_head(tenant_id, repo_id, branch, revision)
        .await?;

    tracing::info!(
        "Node deleted successfully: {} (branch HEAD updated to revision {})",
        node_id,
        revision
    );

    // Emit NodeEvent
    super::event_helpers::emit_node_event(
        &applicator.event_bus,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        Some(node_snapshot.node_type.clone()),
        Some(node_snapshot.path.clone()),
        &revision,
        NodeEventKind::Deleted,
        "replication",
    );

    Ok(())
}
