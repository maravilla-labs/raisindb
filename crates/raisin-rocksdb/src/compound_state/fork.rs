//! A fork inherits its source's compound build state with the entries — when
//! the copied entries are complete for the fork.
//!
//! `copy_branch_indexes` copies the source's COMPOUND_INDEX keys verbatim (at
//! their revisions, up to the fork revision). A build writes each node's
//! newest version at or below its FLOOR (`built_through`) at that version's
//! own revision, its path as of the floor, and nothing of the history below
//! it. So the copy is as complete as the source's keyspace only when the fork
//! revision is AT OR ABOVE the floor; below it, a node whose only build entry
//! sits above the fork revision is missing from the copy, and one whose
//! ancestor moved between the fork revision and the floor is listed under
//! its post-move parent. Such a record is NOT inherited `Ready` (a HEAD read
//! on the fork is never refused by `at_revision`, so a `Ready` there would
//! serve the hole): the fork gets a `NotBuilt` record instead — failed closed
//! for the planner, and a record the `compound_builds` link finds and builds.
//!
//! An inherited `Ready` matters beyond speed: without a record the fork reads
//! `NotBuilt` and, worse, a write this node cannot index there (a cold
//! replicated upsert) has no record to mark, so nothing would ever say the
//! fork's entries are missing one. A merge from the fork back relies on the
//! record (`branches/merge/compound_after_copy.rs`).

use super::marker::transitions;
use super::store::{read_state, CompoundStateStore};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::compound::CompoundBuildPhase;

/// What [`CompoundStateStore::inherit_on_fork`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ForkInheritance {
    /// `Ready` records the fork took over.
    pub inherited: usize,
    /// Records the fork got `NotBuilt` instead (the source's was not `Ready`,
    /// or its floor is above the fork revision): owed a build there.
    pub withheld: usize,
}

impl CompoundStateStore {
    /// Give `target` (a branch just forked from `source` at `fork_revision`)
    /// every record of the source it does not have yet: `Ready` when the
    /// source's is `Ready` with its floor at or below `fork_revision`,
    /// `NotBuilt` otherwise.
    pub fn inherit_on_fork(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source: &str,
        target: &str,
        fork_revision: &HLC,
    ) -> Result<ForkInheritance> {
        let records = self.list_for_branch(tenant_id, repo_id, source)?;
        let _guard = transitions();
        let mut done = ForkInheritance::default();
        for (workspace, mut state) in records {
            if read_state(
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
            let complete = state.availability().is_ready() && state.built_through <= *fork_revision;
            if complete {
                done.inherited += 1;
            } else {
                state.phase = CompoundBuildPhase::NotBuilt;
                done.withheld += 1;
            }
            state.stale_generation = 0;
            state.build_token = 0;
            self.put_unlocked(tenant_id, repo_id, target, &workspace, &state)?;
        }
        Ok(done)
    }
}
