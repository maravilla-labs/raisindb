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
    _op: &Operation,
) -> Result<()> {
    let workspace = node.workspace.as_deref().unwrap_or("default");

    // Apply the upsert using the existing replicated upsert logic
    // This writes a versioned key with the revision HLC, implementing LWW
    // The storage layer naturally handles multiple versions, and load_latest_node
    // will always return the version with the highest revision
    super::super::replication_core::apply_replicated_upsert(
        applicator,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node,
        parent_id,
        revision,
        cf_order_key,
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
    _op: &Operation,
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
    super::super::replication_core::apply_replicated_delete(
        applicator, tenant_id, repo_id, branch, workspace, &node, parent_id, revision,
    )?;

    tracing::debug!(
        node_id = %node_id,
        revision = ?revision,
        "Applied DeleteNodeSnapshot with Delete-Wins semantics"
    );

    // Emit NodeEvent for websocket clients
    super::event_helpers::emit_node_event(
        &applicator.event_bus,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        Some(node.node_type.clone()),
        Some(node.path.clone()),
        revision,
        NodeEventKind::Deleted,
        "replication",
    );

    Ok(())
}
