//! ApplyRevision snapshot capture for direct (non-transaction) repo writes.
//!
//! Direct `NodeRepository` writes (add/update/delete/move) replicate the same
//! way transaction commits do: a single `ApplyRevision` operation carrying
//! full node snapshots. Granular ops (`CreateNode`/`SetProperty`/`DeleteNode`)
//! could drop state the peer needs (path/name changes, removed properties,
//! moved subtrees), so every direct write path funnels through this helper.

use super::NodeRepositoryImpl;
use crate::constants::SYSTEM_ACTOR;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::operations::OperationMeta;
use raisin_replication::operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind};

/// Who a direct-repository write is attributed to.
///
/// The transaction path resolves exactly these two fields from the
/// transaction's `AuthContext` (`transaction/replication/capture.rs`), so the
/// operation a peer replays carries the same pair the origin stamped on its own
/// `NodeEvent`. The repository layer holds no `AuthContext` (see
/// `NodeRepositoryImpl`'s fields), so here the pair must be *passed in* by the
/// caller that does know it -- exactly how the local side already receives it
/// (`emit_move_node_events(.., actor)`, `OperationMeta::actor`).
///
/// This is provenance only. It never grants or widens permission: capture runs
/// after the write is already durable and authorized.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WriteAttribution<'a> {
    /// The human or service principal, as `AuthContext::actor_id()` renders it.
    pub actor: Option<&'a str>,
    /// The non-human initiator, in the `agent_identity` vocabulary.
    pub agent: Option<&'a str>,
}

impl<'a> WriteAttribution<'a> {
    /// Attribution carrying only an actor -- for the ordering paths, whose
    /// trait methods take `actor: Option<&str>` and no agent.
    ///
    /// TODO(attribution): `NodeRepository::reorder_child` / `move_child_before`
    /// / `move_child_after` (`raisin-storage/src/traits/node/mod.rs`) carry
    /// `actor: Option<&str>` but no agent, so a reorder driven by a trigger or
    /// flow replicates with the right *actor* and a `None` agent. Closing that
    /// needs a sibling `agent: Option<&str>` on those three trait methods,
    /// which is ~20 production and ~45 test call sites across six crates
    /// (raisin-storage, -storage-memory, -core, -functions, -transport-ws,
    /// -transport-http, -transport-inprocess). Deliberately not done here: the
    /// churn dwarfs the fix and would bury the rest of this change. Same gap
    /// applies to `publish`/`unpublish`, which take only a path.
    pub(crate) fn actor(actor: Option<&'a str>) -> Self {
        Self { actor, agent: None }
    }

    /// Attribution read off the caller-supplied `OperationMeta`, the carrier the
    /// direct-write paths already thread for revision metadata.
    pub(crate) fn from_operation_meta(meta: Option<&'a OperationMeta>) -> Self {
        Self {
            actor: meta.map(|m| m.actor.as_str()),
            agent: meta.and_then(|m| m.agent.as_deref()),
        }
    }

    /// The actor to record, falling back to `"system"` when the caller had none
    /// -- the same rendering `OperationMeta`-building call sites already use.
    fn actor_or_system(&self) -> &str {
        self.actor
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .unwrap_or(SYSTEM_ACTOR)
    }

    fn agent_owned(&self) -> Option<String> {
        self.agent
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string)
    }
}

impl NodeRepositoryImpl {
    /// Capture one `ApplyRevision` operation covering the given node changes.
    ///
    /// Call AFTER the atomic RocksDB batch write so parent/order-label lookups
    /// observe the committed state. Never fails the write path: capture errors
    /// are logged and swallowed, matching the previous granular capture.
    pub(crate) async fn capture_apply_revision_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        changes: Vec<(Node, ReplicatedNodeChangeKind)>,
        revision: HLC,
        attribution: WriteAttribution<'_>,
    ) {
        if !self.operation_capture.is_enabled() || changes.is_empty() {
            return;
        }

        let mut node_changes = Vec::with_capacity(changes.len());
        for (mut node, kind) in changes {
            if node.workspace.is_none() {
                node.workspace = Some(workspace.to_string());
            }

            let parent_id = match node.parent_path() {
                Some(parent_path) if parent_path == "/" => Some("/".to_string()),
                Some(parent_path) => match self
                    .get_by_path_impl(tenant_id, repo_id, branch, workspace, &parent_path, None)
                    .await
                {
                    Ok(Some(parent)) => Some(parent.id),
                    Ok(None) => None,
                    Err(e) => {
                        tracing::warn!(
                            node_id = %node.id,
                            error = %e,
                            "Failed to resolve parent for replication capture"
                        );
                        None
                    }
                },
                None => None,
            };

            // Full CF order key (label + node-id suffix) so peers reproduce the
            // exact ORDERED_CHILDREN entry. On delete the entry may already be
            // tombstoned - fall back to the node's own order_key.
            let cf_order_key = match parent_id.as_deref() {
                Some(pid) => self
                    .get_order_label_for_child(tenant_id, repo_id, branch, workspace, pid, &node.id)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| node.order_key.clone()),
                None => String::new(),
            };

            node_changes.push(ReplicatedNodeChange {
                node,
                parent_id,
                kind,
                cf_order_key,
            });
        }

        self.capture_apply_revision_prepared(
            tenant_id,
            repo_id,
            branch,
            node_changes,
            revision,
            attribution,
        )
        .await;
    }

    /// Capture one `ApplyRevision` operation from already-built node changes.
    ///
    /// For callers that already know each change's parent_id and CF order key
    /// (e.g. tree/branch copy, which allocated them during the write) - skips
    /// the per-node lookups `capture_apply_revision_snapshot` performs.
    pub(crate) async fn capture_apply_revision_prepared(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_changes: Vec<ReplicatedNodeChange>,
        revision: HLC,
        attribution: WriteAttribution<'_>,
    ) {
        if !self.operation_capture.is_enabled() || node_changes.is_empty() {
            return;
        }

        let op_type = raisin_replication::OpType::ApplyRevision {
            branch_head: revision,
            node_changes,
        };

        // `capture_operation_with_attribution` (not `..._with_revision`, which
        // hardcodes `agent: None`) so the replicated op carries the same
        // actor+agent pair the origin recorded locally.
        if let Err(e) = self
            .operation_capture
            .capture_operation_with_attribution(
                tenant_id.to_string(),
                repo_id.to_string(),
                branch.to_string(),
                op_type,
                attribution.actor_or_system().to_string(),
                attribution.agent_owned(),
                None,
                true,
                Some(revision),
            )
            .await
        {
            tracing::warn!(error = %e, "Failed to capture ApplyRevision for direct write");
        }
    }
}
