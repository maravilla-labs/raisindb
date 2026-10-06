//! Writing a resolved conflict's DELETION into the target branch.

use super::superseded::{order_entry, Superseded};
use super::unique_props::SchemaDefs;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

/// Delete `node_id` from `target_branch` at `revision`: the node blob and
/// every index entry ANY superseded version wrote.
///
/// Each version goes through the one delete tombstoner. Its ORDERED_CHILDREN
/// entry is additionally tombstoned at the label stored on the branch the
/// version came from, since a source-side entry reaches the target only when
/// the copy runs — after this.
#[allow(clippy::too_many_arguments)]
pub(super) async fn write_resolved_deletion(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    target_branch: &str,
    workspace: &str,
    node_id: &str,
    revision: &HLC,
    superseded: &[Superseded],
    unique: &SchemaDefs,
    source: (&str, &HLC),
) -> Result<()> {
    let mut batch = WriteBatch::default();
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let cf_ordered = cf_handle(db, cf::ORDERED_CHILDREN)?;
    let ctx =
        crate::tombstones::TombstoneContext::new(tenant_id, repo_id, target_branch, workspace);
    let cfs = crate::tombstones::TombstoneColumnFamilies::from_db(db)?;
    let index_ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, target_branch, workspace);
    // UNIQUE claims the merged view gives to other nodes are not ended
    // (`merged_view`).
    let others = super::merged_view::MergedView::new(index_ctx, revision, source)
        .others_claims(db, superseded, unique, node_id)?;

    for old in superseded {
        // The ORDERED_CHILDREN parent as the version's OWN branch had it.
        let parent = super::superseded::parent_index_id(
            db,
            tenant_id,
            repo_id,
            &old.branch,
            workspace,
            &old.node,
            Some(&old.at),
        )?;
        crate::tombstones::add_node_tombstones_with_parent(
            &mut batch,
            db,
            &ctx,
            &cfs,
            &old.node,
            revision,
            parent.as_deref(),
        )?;
        crate::repositories::nodes::tombstone_unique_entries(
            &mut batch,
            db,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &old.node,
            unique.of(&old.node.node_type),
            revision,
            Some(&others),
        )?;
        if let Some((parent, label)) = order_entry(
            db,
            tenant_id,
            repo_id,
            &old.branch,
            workspace,
            &old.node,
            Some(&old.at),
        )? {
            let key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &parent,
                &label,
                revision,
                node_id,
            );
            batch.put_cf(cf_ordered, key, keys::TOMBSTONE_VALUE);
        }
    }

    // The blob tombstone even when no side had a live version to tombstone.
    batch.put_cf(
        cf_nodes,
        keys::node_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            node_id,
            revision,
        ),
        keys::TOMBSTONE_VALUE,
    );
    // ...and its `NODE_DELETES` entry beside it, for the same reason: this is
    // the one node tombstone written outside the delete funnel, and a `Ready`
    // branch must hold an entry for every tombstone (`crate::node_delete_index`).
    crate::node_delete_index::stage_delete(
        &mut batch,
        db,
        (tenant_id, repo_id, target_branch, workspace),
        node_id,
        revision,
    )?;
    if superseded.is_empty() {
        // The delete funnel above materializes block-overlay `T`s (plan Phase
        // 11c); with no live version on either side it did not run.
        crate::translation_write::materialize_block_deletion(
            db,
            &mut batch,
            (tenant_id, repo_id, target_branch, workspace),
            node_id,
            revision,
        )?;
    }

    // One node commit step (plan Phase 7b): locked, and the delete's
    // property and compound tombstones re-derived against what is stored on
    // the target at that moment.
    let mut commit = crate::indexing::NodeCommit::new(tenant_id, repo_id, target_branch);
    commit
        .check(
            crate::indexing::StagedDeltaCheck::always(&index_ctx, node_id, revision),
            None,
        )
        .hold_external(&others);
    commit.write(db, batch).await?;
    // No compound stale mark: the delete tombstoner above already retires
    // every version's COMPOUND_INDEX entries, so the index stays exact.

    Ok(())
}
