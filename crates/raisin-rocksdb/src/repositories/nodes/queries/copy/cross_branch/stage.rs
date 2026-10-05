//! Per-node staging for cross-branch copy (STEP 3 of
//! `copy_nodes_across_branches_impl`), the PUT pass: index writes,
//! stale-value tombstones keyed by node id, translation carry-over, and change
//! bookkeeping for one copied entry. Everything the entry ENDS on a key
//! without a node id was staged by the retire pass (`prepare.rs`) for every
//! entry first.

use super::super::super::super::NodeRepositoryImpl;
use super::prepare::PreparedEntry;
use super::{CopyAccumulators, CopyEntry, CopyScope};
use raisin_error::Result;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{CrossBranchNodeChange, NodeChangeInfo};
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Stage one copied node into the shared WriteBatch (after the retire
    /// pass of EVERY entry).
    pub(super) async fn stage_cross_branch_entry(
        &self,
        batch: &mut WriteBatch,
        entry: &CopyEntry,
        prepared: PreparedEntry,
        scope: &CopyScope<'_>,
        acc: &mut CopyAccumulators,
    ) -> Result<()> {
        let PreparedEntry {
            node,
            old_dst,
            operation,
            order_label,
        } = prepared;

        // Stale OLD-value entries of the destination's previous version:
        // property values and references the source no longer has, and its
        // old geometries. This is an upsert, so without the diff a property
        // or reference removed on the source stays LIVE on the target — a
        // published page keeps matching `properties->>'k' = old` and
        // `REFERENCES(...)` long after the source dropped them. Same diff as
        // `update_impl`, from the same helpers; written before the new
        // entries, which share keys with any unchanged value.
        //
        // The PROPERTY_INDEX half is the one writer's: it is diffed against
        // `old_dst` (an upsert, not a create — plan Phase 7 item 3), and is a
        // FULL put. The promotion's commit step re-derives it against what is
        // stored on the target then (`indexing::NodeCommit`, plan Phase 7b,
        // `always`), so a write landing between this read and the commit
        // cannot leave the index disagreeing with the records. Promotion is
        // not the hot path.
        let ctx = crate::indexing::IndexCtx::new(
            scope.tenant_id,
            scope.repo_id,
            scope.target_branch,
            scope.workspace,
        );
        let baseline = self.full_baseline(&ctx, &node.id, scope.revision, old_dst.as_ref())?;
        if let Some(old) = &old_dst {
            self.add_stale_reference_tombstones_to_batch(
                batch,
                old,
                &node,
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
                scope.revision,
            )?;
            self.add_spatial_tombstones_to_batch(
                batch,
                old,
                Some(&node),
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
                scope.revision,
            )?;
        }

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
            crate::repositories::nodes::PropertyWrite {
                baseline: baseline.as_ref(),
                in_place: crate::indexing::InPlace::No,
            },
        )?;

        // A `secret://` reference is branch-agnostic but `cf::SECRETS` is
        // branch-scoped, so the sealed record has to travel with the node or the
        // promoted node's encrypted fields resolve to nothing on the target —
        // silently, because reads never resolve a reference. See `secrets.rs`.
        self.stage_secret_copies(batch, &node, scope, &mut acc.secrets)?;

        // Compound indexes: tombstone the OLD target values first, then write
        // the new entries — without the tombstones a changed column value
        // would leave the stale old-value entry live (the exact bug class
        // fixed in update_impl). UNIQUE: the claims the node gave up were
        // ended in the retire pass; here every claim it holds is put.
        self.add_compound_delta_to_batch(batch, &ctx, baseline.as_ref(), &node, scope.revision)
            .await?;
        self.add_unique_delta_to_batch(
            batch,
            old_dst.as_ref(),
            &node,
            &ctx,
            scope.revision,
            false,
            crate::repositories::nodes::UniqueHalf::Puts,
        )
        .await?;

        // Carry translations (node-level and block-level) to the target
        // branch under the SAME node id.
        //
        // The returned flag is "an overlay DIFFERED from the target's", not
        // "an overlay was written" — every overlay is rewritten at the fresh
        // revision on every run, so the write itself says nothing. Deriving it
        // from the `change_infos` length delta (what this used to do) made
        // every translated node look changed forever and silently exempted the
        // whole multilingual half of a site from the suppression below.
        let (translations_differed, staged_overlays) = self.copy_translations_to_batch(
            batch,
            &node.id,
            scope,
            operation,
            &mut acc.change_infos,
            &mut acc.translation_ops,
        )?;

        // The node and its carried overlays are in this one batch, unreadable
        // until written: sync the localized name index against both (plan
        // Phase 12; the record writer's sync saw the target's old overlays).
        crate::localized_name::sync::sync_node_final(
            &self.db,
            batch,
            crate::localized_name::keys::NameScope::new(
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
            ),
            &node,
            Some(&entry.dst_parent_id),
            scope.revision,
            &staged_overlays,
        )?;

        // Carry the node's outgoing edges — branch-scoped, in their own
        // keyspace, and therefore NOT part of the node write above.
        self.copy_relations_to_batch(batch, &node.id, scope, &mut acc.relation_ops)?;

        // Did this re-copy actually change anything OBSERVABLE on the target?
        // A publish is normally re-run over a set that is mostly untouched, and
        // the copy rewrites every node in it at a fresh revision regardless — so
        // without this the event emitted below would re-embed (real spend) and
        // re-index every unchanged node on every run. The row is still written;
        // only the notification is suppressed. Mirrors the transaction path's
        // no-op guard in `create/tracking.rs::track_update`, widened from
        // `properties` alone to everything an indexer reads (path and name feed
        // fulltext; node_type selects the indexing plan) and to the translation
        // overlays, which are indexed too. Erring toward "changed" costs one
        // wasted re-index; erring toward "unchanged" leaves the target's derived
        // state permanently stale, so every doubt resolves to emitting.
        if operation == ChangeOperation::Modified && !translations_differed {
            if let Some(old) = &old_dst {
                if old.properties == node.properties
                    && old.path == node.path
                    && old.name == node.name
                    && old.node_type == node.node_type
                    && old.archetype == node.archetype
                {
                    acc.content_unchanged.insert(node.id.clone());
                }
            }
        }

        // Track the max label per parent for the last-child metadata cache.
        let slot = acc
            .max_label_per_parent
            .entry(entry.dst_parent_id.clone())
            .or_insert_with(|| order_label.clone());
        if order_label > *slot {
            *slot = order_label.clone();
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
