//! Writing a resolved conflict's DELETION into the target branch.

use super::superseded::{order_entry, Superseded};
use super::unique_props::UniqueProperties;
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
pub(super) fn write_resolved_deletion(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    target_branch: &str,
    workspace: &str,
    node_id: &str,
    revision: &HLC,
    superseded: &[Superseded],
    unique: &UniqueProperties,
) -> Result<()> {
    let mut batch = WriteBatch::default();
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let cf_ordered = cf_handle(db, cf::ORDERED_CHILDREN)?;
    let ctx =
        crate::tombstones::TombstoneContext::new(tenant_id, repo_id, target_branch, workspace);
    let cfs = crate::tombstones::TombstoneColumnFamilies::from_db(db)?;

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

    db.write(batch)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    // No compound stale mark: the delete tombstoner above already retires
    // every version's COMPOUND_INDEX entries, so the index stays exact.

    Ok(())
}
