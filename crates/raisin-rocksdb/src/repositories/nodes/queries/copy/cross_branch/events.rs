//! Node-event emission for cross-branch copy (branch promotion).
//!
//! **Why this lives in the storage primitive and not in the callers.**
//! `copy_nodes_across_branches` writes nodes onto the target branch in one raw
//! `WriteBatch`; it never passes through the transaction commit path, so none of
//! that path's `emit_node_events` machinery runs. Everything that keeps DERIVED
//! state in step — the embedding job, the fulltext batch indexer, the spatial
//! reconciler, trigger evaluation, WS subscriptions — is driven by `Event::Node`.
//! No event therefore means: the promoted node is on the target branch and is
//! visible to `SELECT`, while semantic search, `FULLTEXT_MATCH` and every
//! subscriber behave as if it were never published. Silently, with no error.
//!
//! That is exactly what happened. The WS handler
//! (`raisin-transport-ws/.../handlers/branches.rs`) compensated by publishing
//! events in its OWN body; the function binding
//! (`raisin-functions/.../callbacks/branches.rs::create_branch_copy_nodes`) —
//! which is the one Studio publish actually calls — did not. One primitive, two
//! callers, opposite observable behaviour: the mirrored-path drift CLAUDE.md
//! names as this codebase's dominant bug class. Emitting HERE makes every caller
//! correct at once, and is why the WS handler's private loop was deleted rather
//! than copied into the binding.
//!
//! **Content-change suppression.** A publish is normally re-run over a set that
//! is mostly unchanged, and re-embedding an unchanged node costs real money and
//! mints a revision for nothing. The transaction path already refuses to record
//! a no-op update (`transaction/context/nodes/create/tracking.rs::track_update`);
//! this mirrors it, so a steady-state re-publish emits nothing for the nodes that
//! did not actually change. The staging pass decides — see
//! `stage_cross_branch_entry` — because only it holds the previous target-branch
//! node to compare against.

use super::super::super::super::NodeRepositoryImpl;
use raisin_events::{Event, NodeEvent, NodeEventKind};
use raisin_hlc::HLC;
use raisin_models::tree::ChangeOperation;
use raisin_storage::CrossBranchNodeChange;
use std::collections::HashSet;

impl NodeRepositoryImpl {
    /// Publish one `Event::Node` per promoted node, scoped to the TARGET branch.
    ///
    /// Skips nodes the staging pass proved content-identical to what the target
    /// already held (`content_unchanged`): those produce no observable change,
    /// so re-indexing them is pure churn.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_cross_branch_events(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
        actor: &str,
        changes: &[CrossBranchNodeChange],
        content_unchanged: &HashSet<String>,
    ) {
        let mut emitted = 0usize;
        let mut suppressed = 0usize;

        for change in changes {
            if change.operation == ChangeOperation::Modified
                && content_unchanged.contains(&change.node_id)
            {
                suppressed += 1;
                continue;
            }

            let kind = match change.operation {
                ChangeOperation::Added => NodeEventKind::Created,
                ChangeOperation::Deleted => NodeEventKind::Deleted,
                ChangeOperation::Modified | ChangeOperation::Reordered => NodeEventKind::Updated,
            };

            // Mirrors the commit path's metadata contract: `source: local` is
            // what `UnifiedJobEventHandler::is_remote_event` reads to tell a
            // local write from a replicated one, and `actor` is what the audit
            // subscriber attributes the write to.
            let mut metadata = std::collections::HashMap::new();
            metadata.insert(
                "source".to_string(),
                serde_json::Value::String("local".to_string()),
            );
            metadata.insert(
                "actor".to_string(),
                serde_json::Value::String(actor.to_string()),
            );

            self.event_bus.publish(Event::Node(NodeEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo_id.to_string(),
                branch: target_branch.to_string(),
                workspace_id: workspace.to_string(),
                node_id: change.node_id.clone(),
                node_type: Some(change.node_type.clone()),
                revision: *revision,
                kind,
                path: Some(change.path.clone()),
                metadata: Some(metadata),
            }));
            emitted += 1;
        }

        tracing::debug!(
            target_branch = %target_branch,
            workspace = %workspace,
            revision = %revision,
            emitted,
            suppressed,
            "copy_nodes_across_branches: published node events for the target branch"
        );
    }
}
