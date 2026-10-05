//! The build's half of the compound state machine: registering a build and
//! its compare-and-set `Ready` (see `marker.rs` for the marks it races).

use raisin_error::Result;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_storage::compound::{CompoundBuildPhase, CompoundIndexState};

use super::marker::{mark, transitions};
use super::store::{read_state, CompoundStateStore};

impl CompoundStateStore {
    /// Register a build of `definition` and return its TICKET, to be handed
    /// back to [`Self::complete_build`].
    ///
    /// The ticket is drawn at random and stored in the record
    /// (`CompoundIndexState::build_token`): the build stamps `Ready` only while
    /// the record still carries it. A later registration replaces it, every
    /// mark clears it, and no deleted-and-recreated record or peer record put
    /// over this one by a checkpoint ingest can reproduce it — so two
    /// interleaved builders never both pass, and a generation reset can never
    /// make an old build's compare-and-set match again.
    ///
    /// The FIRST build of an index also writes a `Building` record, so that a
    /// mark arriving during it has a record to advance. A rebuild of an index
    /// that already has a record leaves the record's phase alone — use
    /// [`Self::begin_rebuild`] when the build first CLEARS the keyspace.
    pub fn begin_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
        head: raisin_hlc::HLC,
    ) -> Result<u64> {
        self.begin(
            tenant_id, repo_id, branch, workspace, definition, head, false,
        )
    }

    /// [`Self::begin_build`] for a build that empties the keyspace first: the
    /// record goes to `Building` (unusable) whatever its phase was, keeping its
    /// generation, so the planner never trusts `Ready` over a cleared keyspace.
    /// The caller holds the index's keyspace lock
    /// (`indexing::compound::keyspace::lock`), so no other builder of this
    /// process clears or writes the keyspace meanwhile.
    pub fn begin_rebuild(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
        head: raisin_hlc::HLC,
    ) -> Result<u64> {
        self.begin(
            tenant_id, repo_id, branch, workspace, definition, head, true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn begin(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
        head: raisin_hlc::HLC,
        force_building: bool,
    ) -> Result<u64> {
        let _guard = transitions();
        let existing = read_state(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &definition.name,
        )?;
        let ticket = new_ticket();
        let record = match existing {
            Some(mut kept) if !force_building => {
                kept.build_token = ticket;
                kept
            }
            existing => {
                let mut building = CompoundIndexState::ready(definition, head);
                building.phase = CompoundBuildPhase::Building;
                building.stale_generation = existing.map_or(0, |s| s.stale_generation);
                building.build_token = ticket;
                building
            }
        };
        self.put_unlocked(tenant_id, repo_id, branch, workspace, &record)?;
        Ok(ticket)
    }

    /// Stamp `ready` if the record still carries the ticket
    /// [`Self::begin_build`] returned (`started_under`): no mark arrived, no
    /// other build registered, the record was not replaced. Returns whether
    /// it was stamped; `false` leaves the record to whoever replaced the
    /// ticket (`NotBuilt` after a mark, `Building` under a newer build).
    ///
    /// A WORKSPACE index is also stamped only while its workspace still
    /// declares exactly what was built. The build read the declaration long
    /// before this (its precheck scans the whole workspace), and a change in
    /// that window is reconciled while there is no record to mark, or before
    /// the build's `begin` captures the generation — so the generation alone
    /// cannot refuse it. Checked here, under the transition lock the
    /// reconcile also takes, from the record as stored (never through the
    /// reconciling reader, which would take that lock again). A refused stamp
    /// marks the record `NotBuilt`, so nothing is left `Building` and the next
    /// build — the job's retry re-reads the declaration — starts over.
    pub fn complete_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        mut ready: CompoundIndexState,
        started_under: u64,
    ) -> Result<bool> {
        let _guard = transitions();
        let stored = read_state(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &ready.index_name,
        )?;
        let Some(current) = stored
            .as_ref()
            .filter(|state| started_under != 0 && state.build_token == started_under)
            .map(|state| state.stale_generation)
        else {
            return Ok(false);
        };
        if !super::workspace_reconcile::still_declared(
            &self.db, tenant_id, repo_id, workspace, &ready,
        )? {
            tracing::info!(
                index = %ready.index_name,
                workspace = %workspace,
                branch = %branch,
                "workspace compound declaration changed during the build; not stamping Ready"
            );
            let mut refused = stored.unwrap_or_else(|| ready.clone());
            refused.stale_generation = current;
            mark(&mut refused);
            self.put_unlocked(tenant_id, repo_id, branch, workspace, &refused)?;
            return Ok(false);
        }
        ready.stale_generation = current;
        ready.build_token = 0;
        self.put_unlocked(tenant_id, repo_id, branch, workspace, &ready)?;
        Ok(true)
    }
}

/// A fresh, non-zero build ticket (`0` means "no build registered").
fn new_ticket() -> u64 {
    match uuid::Uuid::new_v4().as_u64_pair().0 {
        0 => 1,
        ticket => ticket,
    }
}
