//! The RETIRE pass of a promotion (plan Phase 13a): everything a copied entry
//! ENDS on the target — a displaced occupant, the node's previous placement
//! (PATH_INDEX, ORDERED_CHILDREN) and the UNIQUE claims it gives up — staged
//! for EVERY entry before any entry is put (`stage.rs`).
//!
//! PATH_INDEX and UNIQUE keys carry no node id, and every write of the
//! promotion lands at ONE revision, so a node vacating a path or a value and
//! another node taking it write the SAME key. A WriteBatch applies in order:
//! retire-then-put leaves the new owner live, put-then-retire erased it. When
//! a promotion moved A out of `/p` and B into it, and B happened to be staged
//! first, A's stale-path tombstone landed after B's mapping and `/p` resolved
//! to nothing on the origin (replicas, applying one node per batch, are
//! protected by the delete tombstoner's ownership check). Ordering by PASS
//! rather than by entry is what makes the outcome independent of the order
//! the source tree happens to list its nodes in. The `delete_missing` prune
//! runs before this pass, for the same reason.
//!
//! Keyed by node id, and so free of the hazard: NODE_PATH, ORDERED_CHILDREN
//! (its key ends with the child id), PROPERTY / REFERENCE / SPATIAL /
//! COMPOUND entries and the localized name index (its forward key carries the
//! node id). Their stale-entry tombstones stay with the node's put.

use super::super::super::super::NodeRepositoryImpl;
use super::{CopyAccumulators, CopyEntry, CopyScope};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{CrossBranchNodeChange, NodeChangeInfo};
use rocksdb::WriteBatch;

/// One entry after the retire pass: what the put pass writes.
pub(super) struct PreparedEntry {
    /// The node as it is written on the target.
    pub(super) node: Node,
    /// The target's version of the same id before the promotion.
    pub(super) old_dst: Option<Node>,
    pub(super) operation: ChangeOperation,
    /// Its ORDERED_CHILDREN label under `entry.dst_parent_id`.
    pub(super) order_label: String,
}

impl NodeRepositoryImpl {
    /// The retire pass for one entry (see the module doc). Reads committed
    /// target state only — nothing staged is readable — so every entry sees
    /// the pre-promotion target.
    pub(super) async fn prepare_cross_branch_entry(
        &self,
        batch: &mut WriteBatch,
        entry: &CopyEntry,
        scope: &CopyScope<'_>,
        acc: &mut CopyAccumulators,
    ) -> Result<PreparedEntry> {
        let src_node = &entry.node;
        let (t, r, b, w) = (
            scope.tenant_id,
            scope.repo_id,
            scope.target_branch,
            scope.workspace,
        );

        // Same id on the target branch -> Added vs Modified.
        let old_dst = self.get_impl(t, r, b, w, &src_node.id, false).await?;
        let operation = if old_dst.is_some() {
            ChangeOperation::Modified
        } else {
            ChangeOperation::Added
        };

        self.retire_displaced_occupant(batch, src_node, scope, acc)
            .await?;

        let mut node = src_node.clone();
        node.parent = Node::extract_parent_name_from_path(&node.path);
        node.has_children = None; // computed field, never stored
        node.children = vec![];
        if operation == ChangeOperation::Modified {
            // Keep creation metadata, stamp the modification.
            node.updated_at = Some(scope.now);
        }

        // Child order: replay the source fractional label onto the target.
        // (Label collisions with independent target children are possible —
        // same caveat as branch merge; reads dedup and the ordering is
        // healed lazily on the next reorder.)
        let order_label = match self.get_order_label_for_child(
            t,
            scope.repo_id,
            scope.source_branch,
            w,
            &entry.src_parent_id,
            &node.id,
        )? {
            Some(label) => label,
            // No source ordering entry (unusual) — append at the end of the
            // target parent instead, through the one minter (an `inc` of the
            // FULL last label fails on its `::` suffix and fell back to a
            // duplicate `first()`).
            None => self.next_append_label(t, r, b, w, &entry.dst_parent_id, scope.revision)?,
        };

        if let Some(old) = &old_dst {
            // Reject a `properties` change onto an immutable destination node.
            // Unlike `update_impl`, this promotion path hand-rolls its own
            // upsert rather than calling it, so it needs its own independent
            // check — see `crate::immutability`. Fails OPEN if the destination
            // type can't be resolved.
            use raisin_storage::NodeTypeRepository as _;
            if let Some(old_type) = self
                .node_type_repo
                .get(
                    raisin_storage::BranchScope::new(t, r, b),
                    &old.node_type,
                    None,
                )
                .await?
            {
                crate::immutability::reject_if_immutable(
                    &old_type,
                    &old.id,
                    &old.properties,
                    &node.properties,
                )?;
            }

            // If the node moved/renamed on the source since the last copy,
            // its old target path and old ordered-children slot are stale.
            self.tombstone_stale_placement(
                batch,
                old,
                &node,
                &entry.dst_parent_id,
                &order_label,
                t,
                r,
                b,
                w,
                scope.revision,
            )
            .await?;

            // The UNIQUE claims it gives up; the claims it holds are put in
            // the put pass.
            let ctx = crate::indexing::IndexCtx::new(t, r, b, w);
            self.add_unique_delta_to_batch(
                batch,
                Some(old),
                &node,
                &ctx,
                scope.revision,
                false,
                crate::repositories::nodes::UniqueHalf::Ends,
            )
            .await?;
        }

        Ok(PreparedEntry {
            node,
            old_dst,
            operation,
            order_label,
        })
    }

    /// A DIFFERENT node already sitting on `src_node`'s destination path must
    /// be retired, not shadowed. The same-id lookup answers "is this an
    /// update?"; it cannot see "same path, different id", which is what a
    /// source-side re-create produces (`deploy --install` mints fresh ids).
    /// PATH_INDEX carries the node id in its VALUE, so the incoming node
    /// would just overwrite the mapping and strand the previous occupant:
    /// blob and every other entry live, reachable by a table scan only.
    /// `get_by_path` then returns one generation, `CHILD_OF` misses children
    /// filed under the other, and a path-qualified DELETE reports zero rows.
    /// Not hypothetical: a three-day-old publish branch had 88 nodes across
    /// 34 paths, and an event page rendered an empty programme because its
    /// tracks hung off a superseded copy of their parent.
    ///
    /// The occupant is retired like a prune — the one body — and replicated
    /// as a `Delete` change. Skipped: one the `delete_missing` prune already
    /// retired (its tombstones are in the batch and its change is reported),
    /// and one that is itself part of the promoted set — it is MOVING away
    /// (its source path differs), not displaced. Retiring a mover deleted it
    /// on every peer that applied the `Delete` change, and on the origin
    /// wherever its retire tombstones outlived its own put.
    async fn retire_displaced_occupant(
        &self,
        batch: &mut WriteBatch,
        src_node: &Node,
        scope: &CopyScope<'_>,
        acc: &mut CopyAccumulators,
    ) -> Result<()> {
        let (t, r, b, w) = (
            scope.tenant_id,
            scope.repo_id,
            scope.target_branch,
            scope.workspace,
        );
        let Some(occ) = self
            .get_by_path_impl(t, r, b, w, &src_node.path, None)
            .await?
            .filter(|occ| {
                occ.id != src_node.id
                    && !acc.retired.contains(&occ.id)
                    && !scope.src_ids.contains(&occ.id)
            })
        else {
            return Ok(());
        };
        let placement = self
            .retire_target_node(batch, &occ, t, r, b, w, scope.revision)
            .await?;
        acc.retired.insert(occ.id.clone());
        // Reported as its own Deleted change: a subscriber that only saw the
        // Added would keep a cache entry for a node that no longer exists, at
        // a path now owned by someone else.
        acc.changes.push(CrossBranchNodeChange {
            node_id: occ.id.clone(),
            path: occ.path.clone(),
            node_type: occ.node_type.clone(),
            operation: ChangeOperation::Deleted,
        });
        acc.change_infos.push(NodeChangeInfo {
            node_id: occ.id.clone(),
            workspace: w.to_string(),
            operation: ChangeOperation::Deleted,
            translation_locale: None,
        });
        acc.displaced.push(super::prune::PrunedNode {
            node: occ,
            placement,
        });
        Ok(())
    }
}
