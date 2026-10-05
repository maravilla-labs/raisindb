//! Replicated node DELETES: the `Delete` change of an `ApplyRevision` and the
//! `DeleteNodeSnapshot` it decomposes into — ONE body for both.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_replication::Operation;
use rocksdb::WriteBatch;

use super::super::node_operations::EventAttribution;
use super::OperationApplicator;

impl OperationApplicator {
    /// Apply a single replicated node delete
    ///
    /// Delegates to the shared `crate::tombstones` module (single source of
    /// truth for deletion tombstones), so replicated deletes clean up the same
    /// families as local deletes — including packed adjacency lists,
    /// compound/spatial indexes, and NODE_PATH.
    ///
    /// UNIQUE claims too, which the shared module leaves to its callers (they
    /// need the node type's `unique: true` names). This path must not read a
    /// NodeType (the deadlock rule), and the definitions CACHE is not enough:
    /// cold, it ended nothing, so a claim written while it was warm stayed
    /// live on this replica forever and refused a later legitimate write of
    /// that value. Instead every claim this node has held for a value of the
    /// deleted version — the carried one AND the one stored here at the
    /// delete — is found from the index (`owned_unique_names`, one seek per
    /// top-level property) and ended — correct on every node, warm or cold,
    /// and exact: a claim exists only for a property some version declared
    /// `unique: true` (plan Phase 13a).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::replication::application) fn apply_replicated_delete(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        parent_id: Option<&str>,
        revision: &HLC,
        attribution: EventAttribution<'_>,
    ) -> Result<()> {
        let mut batch = WriteBatch::default();

        // The ORDERED_CHILDREN tombstone is keyed by the parent's ID, which
        // the replicated change carries: pass it UNCONDITIONALLY. (This used to
        // feed `node.parent` — the parent's NAME — whenever the peer's node had
        // one, so the fix for the delete tombstoner never reached replicas.)
        let ctx = crate::tombstones::TombstoneContext::new(tenant_id, repo_id, branch, workspace);
        let cfs = crate::tombstones::TombstoneColumnFamilies::from_arc_db(&self.db)?;
        crate::tombstones::add_node_tombstones_with_parent(
            &mut batch,
            self.db.as_ref(),
            &ctx,
            &cfs,
            node,
            revision,
            parent_id,
        )?;

        // UNIQUE claims: every claim THIS node has held for a value of the
        // deleted version, found from the index itself (plan Phase 13a) — no
        // NodeType read, no definitions cache, so a cold cache ends them too.
        // Probed for the CARRIED version and for the version stored here at
        // the delete: a concurrent update from another origin may have moved
        // this replica's version on (V1 -> V2) while the deleting origin
        // still saw V1, and V2's claim must end here exactly as the origin
        // ends it when V2 reaches it, out of order, below its delete.
        let index_ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace);
        let local = crate::mvcc_read::node_version_at_or_before(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.id,
            Some(revision),
        )?
        .and_then(|(_, version)| version);
        for version in std::iter::once(node).chain(local.as_ref()) {
            let owned = crate::repositories::nodes::owned_unique_names(
                &self.db, &index_ctx, version, revision,
            )?;
            if !owned.is_empty() {
                crate::repositories::nodes::tombstone_unique_entries(
                    &mut batch, &self.db, tenant_id, repo_id, branch, workspace, version, &owned,
                    revision, None,
                )?;
            }
        }

        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to apply replicated delete: {}", e))
        })?;

        super::super::node_operations::emit_node_event(
            &self.event_bus,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.id,
            Some(node.node_type.clone()),
            Some(node.path.clone()),
            revision,
            raisin_events::NodeEventKind::Deleted,
            "replication",
            attribution,
        );

        Ok(())
    }

    /// Apply a node snapshot delete (decomposed from ApplyRevision for CRDT commutativity)
    ///
    /// Uses Delete-Wins semantics - deletions always take precedence.
    ///
    /// The op carries the `Delete` change's pre-delete node (stamped with its
    /// workspace) and ORDERED_CHILDREN parent, and is applied from them exactly
    /// as the `ApplyRevision` it came from is. Resolving them here instead was
    /// wrong whenever another op of the same revision ran first: a pruned
    /// subtree's parent path is tombstoned before its child is applied, and a
    /// renamed parent's old path is gone, so the parent resolved to nothing and
    /// the child stayed listed under it on this replica only.
    ///
    /// An op from an older binary names only the id: the node is found by a
    /// workspace scan, as the version in force at the delete, and its parent
    /// as of that version's revision rather than by path at HEAD.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::replication::application) async fn apply_delete_node_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_id: &str,
        carried: Option<&Node>,
        parent_id: Option<&str>,
        revision: &HLC,
        op: &Operation,
    ) -> Result<()> {
        if let Some(node) = carried {
            return self.apply_replicated_delete(
                tenant_id,
                repo_id,
                branch,
                super::node_workspace(node),
                node,
                parent_id,
                revision,
                EventAttribution::from_op(op),
            );
        }

        // The version in force at the delete, its parent resolved as of that
        // version; failing that (a delete older than every stored version),
        // the latest, its parent at HEAD — what this path always did.
        let found = match self.load_node_replaced_by(
            tenant_id,
            repo_id,
            branch,
            super::WorkspaceHint::Unknown,
            node_id,
            revision,
        )? {
            Some((at, node)) => Some((Some(at), node)),
            None => self
                .load_latest_node(
                    tenant_id,
                    repo_id,
                    branch,
                    super::WorkspaceHint::Unknown,
                    node_id,
                )?
                .map(|node| (None, node)),
        };
        let Some((at, node)) = found else {
            tracing::debug!(
                node_id = %node_id,
                revision = ?revision,
                "Node not found for DeleteNodeSnapshot - treating as already deleted"
            );
            return Ok(());
        };

        // Found by scan: the stored node's workspace is its key's.
        let workspace = super::node_workspace(&node);
        let parent = crate::repositories::nodes::parent_index_id(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.path,
            at.as_ref(),
        )?;

        self.apply_replicated_delete(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node,
            parent.as_deref().or(parent_id),
            revision,
            EventAttribution::from_op(op),
        )?;

        tracing::debug!(
            node_id = %node_id,
            revision = ?revision,
            "Applied DeleteNodeSnapshot with Delete-Wins semantics"
        );

        Ok(())
    }
}
