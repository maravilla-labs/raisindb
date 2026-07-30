//! Snapshot operations: upsert and delete node snapshots

use super::super::OperationApplicator;
use raisin_error::Result;
use raisin_events::NodeEventKind;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_replication::Operation;

/// Apply a node snapshot upsert (decomposed from ApplyRevision for CRDT commutativity)
pub(in crate::replication::application) async fn apply_upsert_node_snapshot(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node: &Node,
    parent_id: Option<&str>,
    revision: &HLC,
    cf_order_key: &str,
    op: &Operation,
) -> Result<()> {
    let workspace = node.workspace.as_deref().unwrap_or("default");

    // Applies via the ONE replicated-upsert body. This used to call a second,
    // hand-maintained copy in `replication_core.rs` that was missing the
    // `load_latest_node` stale-tombstone diff, the NODE_PATH index write, and
    // spatial indexing entirely — so snapshot-based replication left stale path,
    // property and reference entries live on peers, and geometry unindexed. That
    // copy is gone; `event_kind = None` preserves this path's timestamp-derived
    // Created/Updated distinction.
    applicator.apply_replicated_upsert_with_event(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node,
        parent_id,
        revision,
        cf_order_key,
        super::event_helpers::EventAttribution::from_op(op),
        None,
    )?;

    tracing::debug!(
        node_id = %node.id,
        revision = ?revision,
        "Applied UpsertNodeSnapshot with LWW semantics"
    );

    // Note: Event emission is handled by apply_replicated_upsert()
    // It will emit Created for new nodes, Updated for existing nodes

    Ok(())
}

/// Apply a node snapshot delete (decomposed from ApplyRevision for CRDT commutativity)
///
/// This handler applies Delete-Wins semantics - deletions always take precedence.
/// The deletion is written as a tombstone with the given revision HLC.
pub(in crate::replication::application) async fn apply_delete_node_snapshot(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    revision: &HLC,
    op: &Operation,
) -> Result<()> {
    // Load the node to get its full information for deletion
    let node = match applicator.load_latest_node(tenant_id, repo_id, branch, node_id)? {
        Some(n) => n,
        None => {
            // Node doesn't exist, nothing to delete (idempotent)
            tracing::debug!(
                node_id = %node_id,
                revision = ?revision,
                "Node not found for DeleteNodeSnapshot - treating as already deleted"
            );
            return Ok(());
        }
    };

    let workspace = node.workspace.as_deref().unwrap_or("default");

    // We use None for parent_id - delete logic handles this gracefully
    let parent_id: Option<&str> = None;

    // Apply the delete using the existing replicated delete logic
    let attribution = super::event_helpers::EventAttribution::from_op(op);
    applicator.apply_replicated_delete(
        tenant_id,
        repo_id,
        branch,
        workspace,
        &node,
        parent_id,
        revision,
        attribution,
    )?;

    tracing::debug!(
        node_id = %node_id,
        revision = ?revision,
        "Applied DeleteNodeSnapshot with Delete-Wins semantics"
    );

    // NO event emission here: `apply_replicated_delete` above already emitted
    // `Deleted` for this node with identical arguments. One logical delete must
    // produce exactly one event (see the live applicator's `apply_delete_node`).

    Ok(())
}
