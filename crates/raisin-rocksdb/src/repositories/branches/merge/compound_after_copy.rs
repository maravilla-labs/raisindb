//! What a merge owes the target's COMPOUND state after `copy_branch_indexes`
//! replayed the source's keys into it.
//!
//! 1. **Definitions.** The copy brought the source's NodeType versions. The
//!    target's definitions cache feeds every compound write and build there
//!    and is cache-first, so it is re-read from storage now; a cached type
//!    whose declarations changed gets its indexes marked `NotBuilt` by the
//!    refresh (`indexing::compound::refresh`).
//! 2. **History.** The copied entries end the target's older tuples only if
//!    the source index holds the tombstones its writers wrote for every source
//!    change since the common ancestor. A source build keeps no history below
//!    its floor (`built_through`), so when the floor is not below the
//!    earliest source change — or the source index is not `Ready` at all — the
//!    target's index can hold a tuple nothing ends: it is marked `NotBuilt`
//!    and rebuilt locally.

use super::super::BranchRepositoryImpl;
use crate::compound_state::{read_state, CompoundStateStore};
use raisin_error::Result;
use raisin_hlc::HLC;
use std::sync::Arc;

impl BranchRepositoryImpl {
    /// See the module doc. `earliest_change` is the lowest source revision
    /// since the common ancestor that changed nodes (`None`: nothing did).
    pub(super) async fn compound_after_merge_copy(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        earliest_change: Option<HLC>,
    ) -> Result<()> {
        let node_types = crate::repositories::NodeTypeRepositoryImpl::new(
            self.db.clone(),
            Arc::new(crate::repositories::RevisionRepositoryImpl::new(
                self.db.clone(),
                "merge-index-defs".to_string(),
            )),
            Arc::new(BranchRepositoryImpl::new(self.db.clone())),
        );
        let scope = raisin_storage::BranchScope::new(tenant_id, repo_id, target_branch);
        if let Err(e) =
            crate::indexing::compound::defs::refresh_branch(&self.db, &node_types, scope, &[]).await
        {
            tracing::warn!(error = %e, "merge: could not refresh the target's index definitions");
        }

        let Some(earliest) = earliest_change else {
            return Ok(());
        };
        let store = CompoundStateStore::new(self.db.clone());
        for (workspace, state) in store.list_for_branch(tenant_id, repo_id, target_branch)? {
            if !state.availability().is_ready() {
                continue; // nothing served from it to protect
            }
            let source = read_state(
                &self.db,
                tenant_id,
                repo_id,
                source_branch,
                &workspace,
                &state.index_name,
            )?;
            let complete = source.is_some_and(|s| {
                s.availability().is_ready()
                    && s.definition_hash == state.definition_hash
                    && s.built_through < earliest
            });
            if complete {
                continue;
            }
            store.mark_not_built(
                tenant_id,
                repo_id,
                target_branch,
                &workspace,
                &state.index_name,
            )?;
            crate::indexing::compound::cold::request_build(
                &self.db,
                tenant_id,
                repo_id,
                target_branch,
                &workspace,
            );
            tracing::info!(
                index = %state.index_name,
                workspace = %workspace,
                source = %source_branch,
                "merge: source compound history incomplete since the merge base; target index marked NotBuilt"
            );
        }
        Ok(())
    }
}
