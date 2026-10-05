//! A fork inherits its source's compound build state with the entries.
//!
//! `copy_branch_indexes` copies the source's COMPOUND_INDEX keys verbatim (at
//! their revisions, up to the fork revision), so the fork's keyspace is
//! exactly as complete as the source's — including its history floor. The
//! state record says so; without it the fork reads `NotBuilt` and, worse, a
//! write this node cannot index there (a cold replicated upsert) has no
//! record to mark, so nothing would ever say the fork's entries are missing
//! one. A merge from the fork back relies on the record
//! (`branches/merge/compound_after_copy.rs`).

use super::marker::transitions;
use super::store::{read_state, CompoundStateStore};
use raisin_error::Result;

impl CompoundStateStore {
    /// Give `target` (a branch just forked from `source`) every `Ready`
    /// record of the source it does not have yet. Returns how many.
    pub fn inherit_on_fork(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source: &str,
        target: &str,
    ) -> Result<usize> {
        let records = self.list_for_branch(tenant_id, repo_id, source)?;
        let _guard = transitions();
        let mut inherited = 0;
        for (workspace, mut state) in records {
            if !state.availability().is_ready()
                || read_state(
                    &self.db,
                    tenant_id,
                    repo_id,
                    target,
                    &workspace,
                    &state.index_name,
                )?
                .is_some()
            {
                continue;
            }
            state.stale_generation = 0;
            self.put_unlocked(tenant_id, repo_id, target, &workspace, &state)?;
            inherited += 1;
        }
        Ok(inherited)
    }
}
