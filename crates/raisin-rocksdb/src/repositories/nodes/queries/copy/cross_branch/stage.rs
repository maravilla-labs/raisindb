//! Per-node staging for cross-branch copy (STEP 3 of
//! `copy_nodes_across_branches_impl`): index writes, stale-value tombstones,
//! translation carry-over, and change bookkeeping for one copied entry.

use super::super::super::super::NodeRepositoryImpl;
use super::{CopyAccumulators, CopyEntry, CopyScope};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{CrossBranchNodeChange, NodeChangeInfo};
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Stage one copied node into the shared WriteBatch.
    pub(super) async fn stage_cross_branch_entry(
        &self,
        batch: &mut WriteBatch,
        entry: &CopyEntry,
        scope: &CopyScope<'_>,
        acc: &mut CopyAccumulators,
    ) -> Result<()> {
        let src_node = &entry.node;

        // Same id on the target branch -> Added vs Modified.
        let old_dst = self
            .get_impl(
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
                &src_node.id,
                false,
            )
            .await?;
        let operation = if old_dst.is_some() {
            ChangeOperation::Modified
        } else {
            ChangeOperation::Added
        };

        // A DIFFERENT node already sitting on the destination path must be
        // retired, not shadowed. `old_dst` above answers "same id, is this an
        // update?"; it cannot see "same path, different id", which is what a
        // source-side re-create produces (`deploy --install` mints fresh ids).
        // Left alone the incoming node overwrites the path mapping and the
        // previous occupant becomes a scan-only orphan — see
        // `displace_path_occupant` for what that does to a live site.
        //
        // Ordering is load-bearing: this runs BEFORE the node is staged,
        // because the tombstone and the new mapping share one PATH_INDEX key.
        let displaced = self
            .get_by_path_impl(
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
                &src_node.path,
                None,
            )
            .await?
            .filter(|occ| occ.id != src_node.id);
        if let Some(occ) = &displaced {
            self.displace_path_occupant(
                batch,
                occ,
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
                scope.revision,
            )
            .await?;
        }

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
            scope.tenant_id,
            scope.repo_id,
            scope.source_branch,
            scope.workspace,
            &entry.src_parent_id,
            &node.id,
        )? {
            Some(label) => label,
            None => {
                // No source ordering entry (unusual) — append at the end
                // of the target parent instead.
                match self.get_last_order_label(
                    scope.tenant_id,
                    scope.repo_id,
                    scope.target_branch,
                    scope.workspace,
                    &entry.dst_parent_id,
                )? {
                    Some(last) => crate::fractional_index::inc(&last)
                        .unwrap_or_else(|_| crate::fractional_index::first()),
                    None => crate::fractional_index::first(),
                }
            }
        };

        // Node blob + path/node_path/property/reference/relation/ordered
        // index entries, all at the shared revision.
        self.add_node_to_batch_with_parent_id(
            batch,
            &node,
            scope.tenant_id,
            scope.repo_id,
            scope.target_branch,
            scope.workspace,
            scope.revision,
            Some(&order_label),
            Some(&entry.dst_parent_id),
        )?;

        // A `secret://` reference is branch-agnostic but `cf::SECRETS` is
        // branch-scoped, so the sealed record has to travel with the node or the
        // promoted node's encrypted fields resolve to nothing on the target —
        // silently, because reads never resolve a reference. See `secrets.rs`.
        self.stage_secret_copies(batch, &node, scope, &mut acc.secrets)?;

        // Compound/unique indexes: tombstone the OLD target values first,
        // then write the new entries — without the tombstones a changed
        // column value would leave the stale old-value entry live (the
        // exact bug class fixed in update_impl).
        if let Some(old) = &old_dst {
            self.add_compound_tombstones_to_batch(
                batch,
                old,
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
            )?;
            self.add_unique_tombstones_to_batch(
                batch,
                old,
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
                scope.revision,
            )
            .await?;
        }
        self.add_compound_indexes_to_batch(
            batch,
            &node,
            scope.tenant_id,
            scope.repo_id,
            scope.target_branch,
            scope.workspace,
            scope.revision,
        )
        .await?;
        self.add_unique_indexes_to_batch(
            batch,
            &node,
            scope.tenant_id,
            scope.repo_id,
            scope.target_branch,
            scope.workspace,
            scope.revision,
        )
        .await?;

        // If the node moved/renamed on the source since the last copy,
        // its old target path and old ordered-children slot are stale.
        if let Some(old) = &old_dst {
            self.tombstone_stale_placement(
                batch,
                old,
                &node,
                &entry.dst_parent_id,
                &order_label,
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
                scope.revision,
            )
            .await?;
        }

        // Carry translations (node-level and block-level) to the target
        // branch under the SAME node id.
        self.copy_translations_to_batch(
            batch,
            &node.id,
            scope.tenant_id,
            scope.repo_id,
            scope.source_branch,
            scope.target_branch,
            scope.workspace,
            scope.revision,
            scope.now,
            &scope.meta_actor,
            &scope.meta_message,
            scope.meta_is_system,
            operation,
            &mut acc.change_infos,
        )?;

        // Track the max label per parent for the last-child metadata cache.
        let slot = acc
            .max_label_per_parent
            .entry(entry.dst_parent_id.clone())
            .or_insert_with(|| order_label.clone());
        if order_label > *slot {
            *slot = order_label.clone();
        }

        // Report the displacement as its own Deleted change. A subscriber that
        // only saw the Added would keep a cache entry for a node that no longer
        // exists, at a path now owned by someone else.
        if let Some(occ) = displaced {
            acc.changes.push(CrossBranchNodeChange {
                node_id: occ.id.clone(),
                path: occ.path.clone(),
                node_type: occ.node_type.clone(),
                operation: ChangeOperation::Deleted,
            });
            acc.change_infos.push(NodeChangeInfo {
                node_id: occ.id,
                workspace: scope.workspace.to_string(),
                operation: ChangeOperation::Deleted,
                translation_locale: None,
            });
        }

        acc.changes.push(CrossBranchNodeChange {
            node_id: node.id.clone(),
            path: node.path.clone(),
            node_type: node.node_type.clone(),
            operation,
        });
        acc.change_infos.push(NodeChangeInfo {
            node_id: node.id.clone(),
            workspace: scope.workspace.to_string(),
            operation,
            translation_locale: None,
        });
        acc.nodes_for_replication
            .push((node, entry.dst_parent_id.clone(), order_label));

        Ok(())
    }
}
