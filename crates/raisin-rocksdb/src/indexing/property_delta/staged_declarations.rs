//! The workspace-DECLARATION half of a commit-time re-check (plan Phase 13e).
//!
//! A write derives its workspace-index entries from the declarations it reads
//! when it STAGES (`compound::workspace_defs`), not when it commits. A
//! declaration that changes in between is reconciled — the index marked
//! `NotBuilt`, a build requested — and that build may clear the keyspace and
//! read its floor before this write commits. Its entries, derived from the old
//! declaration, would then land in a keyspace the build stamps `Ready` for the
//! new one: old-layout tuples nothing ends, the new-layout ones missing.
//!
//! So every [`StagedDeltaCheck`] records the declaration-change sequence when
//! it is recorded, and the commit — under the node commit lock, before the
//! batch is written — asks whether its workspace's declarations changed since.
//! When they did, the workspace's OWN indexes are marked `NotBuilt` (the
//! generation advances, so a build already running loses its compare-and-set
//! and its retry clears this write's entries) and a build is requested: the
//! commit-time correction's fail-closed answer (`staged_compound`), for the
//! records the change concerns. A NodeType index is untouched — its
//! declaration did not change.
//!
//! Known gap, shared with that correction: the mark is written just before
//! the batch, not in it, so a build that begins AND clears and reads its
//! floor in between still misses this write. And a bulk writer that records
//! `always` / `rekey` checks after staging (a move, a merge) records the
//! sequence late: a change between its staging and its check is not seen.

use super::StagedDeltaCheck;
use crate::compound_state::{CompoundStateStore, StaleScope};
use crate::indexing::compound::{cold, workspace_defs};
use raisin_error::Result;
use rocksdb::DB;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Mark the workspace indexes of every workspace `checks` wrote whose
/// declarations changed after the check recorded them (the earliest check
/// per workspace decides). Call under the node commit lock, before the batch
/// is written. Returns how many workspaces were marked.
pub fn fail_changed_declarations<'a, I>(db: &Arc<DB>, checks: I) -> Result<usize>
where
    I: IntoIterator<Item = &'a StagedDeltaCheck>,
{
    let mut earliest: BTreeMap<(String, String, String, String), u64> = BTreeMap::new();
    for check in checks {
        let ((tenant, repo, branch, workspace), seq) = check.declarations_scope();
        let key = (
            tenant.to_string(),
            repo.to_string(),
            branch.to_string(),
            workspace.to_string(),
        );
        earliest
            .entry(key)
            .and_modify(|at| *at = (*at).min(seq))
            .or_insert(seq);
    }
    let mut marked = 0;
    for ((tenant, repo, branch, workspace), seq) in earliest {
        if !workspace_defs::changed_since(db, &tenant, &repo, &workspace, seq)? {
            continue;
        }
        CompoundStateStore::new(db.clone()).mark_workspace_stale_in(
            &tenant,
            &repo,
            &branch,
            &workspace,
            StaleScope::WorkspaceOwned,
        )?;
        cold::request_build(db, &tenant, &repo, &branch, &workspace);
        tracing::info!(
            tenant = %tenant,
            repo = %repo,
            branch = %branch,
            workspace = %workspace,
            "workspace compound declarations changed while a write was staged; \
             workspace indexes marked NotBuilt"
        );
        marked += 1;
    }
    Ok(marked)
}
