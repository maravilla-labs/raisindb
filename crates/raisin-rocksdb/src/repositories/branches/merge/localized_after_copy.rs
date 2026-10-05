//! What a merge owes the target's LOCALIZED NAME index after
//! `copy_branch_indexes` replayed the source's rows into it (plan Phase 12).
//!
//! The copy replays the source's `LOCALIZED_NAME_INDEX` rows key by key at
//! their own revisions, and the target's records and overlays become the
//! newest per key across BOTH histories. Two things can then disagree with
//! what the target serves:
//!
//! 1. **The source had no rows** for a node — written before the index
//!    existed, or while it was switched off, on a branch not built yet. The
//!    copy brings the node and its overlays but no claim.
//! 2. **The histories interleave.** The source gave a name up at `rs`
//!    (`T` on the claim); the target set the same name again at `rt > rs`,
//!    which wrote no row because nothing changed from its point of view. The
//!    copied `T` is now the newest version of the claim, while the merged
//!    overlay still names it.
//!
//! So every node the source changed since the merge base is re-synced at the
//! merge revision `M` from the merged view — a FULL put, at or above every
//! copied row, so it is authoritative (a deleted one gets its tombstones).
//! And a target workspace that is `Ready` while the source's is not is
//! invalidated and rebuilt: the change list is what the revision metadata
//! recorded, and an incomplete source must not leave a `Ready` target
//! claiming completeness on its word alone.

use super::super::BranchRepositoryImpl;
use crate::localized_name::keys::NameScope;
use crate::localized_name::{config, state, sync};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_storage::NodeChangeInfo;
use rocksdb::WriteBatch;
use std::collections::BTreeSet;

impl BranchRepositoryImpl {
    /// See the module doc. `changed`: the source's changes since the merge
    /// base (the merge commit's `changed_nodes`).
    pub(super) fn localized_after_merge_copy(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        merge_revision: &HLC,
        changed: &[NodeChangeInfo],
    ) -> Result<()> {
        if !crate::localized_name::enabled() {
            return Ok(());
        }
        let db = &*self.db;
        let Some(cfg) = config::load(db, tenant_id, repo_id)? else {
            return Ok(());
        };
        let fingerprint = cfg.fingerprint();
        let workspaces: BTreeSet<&str> = changed.iter().map(|c| c.workspace.as_str()).collect();
        let mut invalidated = false;
        for workspace in &workspaces {
            let ready = |branch: &str| -> Result<bool> {
                let record = state::read(db, tenant_id, repo_id, branch, workspace)?;
                Ok(state::availability(record.as_ref(), &fingerprint, None).is_ready())
            };
            if ready(target_branch)? && !ready(source_branch)? {
                invalidated |=
                    state::invalidate_workspace(db, tenant_id, repo_id, target_branch, workspace)?;
            }
        }
        if invalidated {
            tracing::info!(
                source = %source_branch,
                target = %target_branch,
                "merge: source localized name index not built; target marked NotBuilt"
            );
            crate::localized_name::auto::request_build(tenant_id, repo_id, target_branch);
        }

        let mut batch = WriteBatch::default();
        let mut seen = BTreeSet::new();
        for change in changed {
            if !seen.insert((change.workspace.as_str(), change.node_id.as_str())) {
                continue;
            }
            let scope = NameScope::new(tenant_id, repo_id, target_branch, &change.workspace);
            match sync::node_at(db, scope, &change.node_id, Some(merge_revision))? {
                Some((_, Some(node))) => {
                    sync::sync_node_full(db, &mut batch, scope, &node, None, merge_revision)?
                }
                _ => sync::stage_node_deleted(
                    db,
                    &mut batch,
                    scope,
                    &change.node_id,
                    merge_revision,
                )?,
            }
        }
        db.write(batch).map_err(|e| Error::storage(e.to_string()))
    }
}
