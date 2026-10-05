//! COMPOUND and UNIQUE entries on the replication apply path (plan Phase 8
//! step 3), and the fail-closed mark when they cannot be written.
//!
//! Both are schema-driven: the entries depend on the node type's compound
//! declarations and `unique: true` properties. This path must not read a
//! NodeType (the deadlock rule), so it reads the definitions CACHE
//! (`indexing::compound::defs`, warmed by local writes, builds, the boot sweep
//! and `Event::Schema`) and:
//!
//! - **warm** — writes the entries through the same writers the local paths
//!   use, in the node's own batch: the compound index stays `Ready` and
//!   replica compound queries are index-served (Phase 2.5's "mark `NotBuilt`
//!   on every upsert" is retired for these writes);
//! - **cold** — writes the node without them and marks every recorded
//!   compound index of the workspace `NotBuilt` IN THE SAME BATCH, under the
//!   compound transition lock (`CompoundStateStore::write_marking_stale`, the
//!   generation that keeps a running build from stamping `Ready` over it),
//!   and requests a local build (`indexing::compound::cold`), which warms the
//!   cache so the next write is warm. Never a silent skip. UNIQUE has no
//!   availability gate; a cold write derives its claims from the claims the
//!   replaced version owns in the index instead (`cold_unique_delta`, plan
//!   Phase 13a), so it never leaves a stale claim live.

use crate::indexing::compound::{cold, types_of, write_compound_delta, DefsSet};
use crate::indexing::{Baseline, IndexCtx};
use crate::repositories::nodes::{
    end_claims_at, owned_unique_names, write_unique_delta, UniqueSide,
};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::BranchScope;
use rocksdb::WriteBatch;

use super::OperationApplicator;

/// The version a replicated write replaces, and the versions stored above it.
fn prior_and_successors<'a>(
    baseline: Baseline<'a>,
) -> (Option<&'a Node>, &'a [crate::mvcc_read::StoredVersion]) {
    match baseline {
        Baseline::Full(prior) => (prior, &[]),
        Baseline::OutOfOrder { prior, successors } => (prior, successors),
        Baseline::Predecessor(prior) => (Some(prior), &[]),
        Baseline::NoPrior => (None, &[]),
    }
}

/// A version and its type's `unique: true` property names.
fn side<'a>(defs: &'a DefsSet, node: &'a Node) -> UniqueSide<'a> {
    UniqueSide {
        node,
        properties: defs.unique(&node.node_type),
    }
}

impl OperationApplicator {
    /// Commit `batch` (a replicated upsert of `node` at `revision`, replacing
    /// `baseline`'s prior) with the node's COMPOUND and UNIQUE entries, or —
    /// definitions cold — with the workspace's compound indexes marked stale.
    pub(in crate::replication::application) fn commit_with_schema_indexes(
        &self,
        mut batch: WriteBatch,
        ctx: &IndexCtx<'_>,
        baseline: Baseline<'_>,
        node: &Node,
        revision: &HLC,
        in_place: bool,
    ) -> Result<()> {
        let scope = BranchScope::new(ctx.tenant_id, ctx.repo_id, ctx.branch);
        let types = types_of(&baseline, node);
        let Some(defs) = DefsSet::peek(&self.db, scope, types.iter().copied()) else {
            // Requested even with no compound index recorded: the drain also
            // warms the definitions the UNIQUE claims need — these types
            // included, so one with no NodeType record is cached as such and
            // the next write is warm.
            cold::request_build_for(
                &self.db,
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                ctx.workspace,
                &types,
            );
            self.cold_unique_delta(&mut batch, ctx, baseline, node, revision, in_place)?;
            return self.write_marking_compound_stale(
                batch,
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                ctx.workspace,
            );
        };
        write_compound_delta(&mut batch, &self.db, ctx, &defs, baseline, node, revision)?;

        let (prior, successors) = prior_and_successors(baseline);
        let new_side = side(&defs, node);
        write_unique_delta(
            &mut batch,
            &self.db,
            ctx,
            prior.map(|n| side(&defs, n)),
            new_side,
            revision,
            in_place,
        )?;
        if let Some((first_rev, first)) = successors.first() {
            let next = first.as_ref().map(|n| side(&defs, n));
            end_claims_at(&mut batch, &self.db, ctx, new_side, next, first_rev)?;
        }
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to apply upsert: {}", e)))
    }

    /// The UNIQUE half of a COLD upsert (plan Phase 13a). Without the
    /// definitions the claims cannot be derived from the schema, but the ones
    /// the replaced version holds can be read from the index: every value of
    /// `prior` THIS node has held a claim on (`owned_unique_names` — its own
    /// entry, not the newest one, so another node of the same promotion
    /// already applied at `revision` cannot hide it) names a property some
    /// version declared `unique: true`. They are diffed against `node`
    /// through the one writer — a value that changed is ended, the new value
    /// claimed — so a claim written while the cache was warm is never left
    /// live by a write applied while it was cold (it used to be: the old
    /// value then stayed claimed forever and refused a legitimate later
    /// write). A property whose prior held no claim here gets none, as before.
    fn cold_unique_delta(
        &self,
        batch: &mut WriteBatch,
        ctx: &IndexCtx<'_>,
        baseline: Baseline<'_>,
        node: &Node,
        revision: &HLC,
        in_place: bool,
    ) -> Result<()> {
        let (Some(prior), successors) = prior_and_successors(baseline) else {
            return Ok(());
        };
        let names = owned_unique_names(&self.db, ctx, prior, revision)?;
        if names.is_empty() {
            return Ok(());
        }
        // Claims are keyed by type: a retyped node keeps none of them.
        let same_type = |n: &Node| n.node_type == prior.node_type;
        let kept: &[String] = if same_type(node) { &names } else { &[] };
        let new_side = UniqueSide {
            node,
            properties: kept,
        };
        let old_side = UniqueSide {
            node: prior,
            properties: &names,
        };
        write_unique_delta(
            batch,
            &self.db,
            ctx,
            Some(old_side),
            new_side,
            revision,
            in_place,
        )?;
        if let Some((first_rev, first)) = successors.first() {
            let next = first.as_ref().filter(|n| same_type(n)).map(|n| UniqueSide {
                node: n,
                properties: kept,
            });
            end_claims_at(batch, &self.db, ctx, new_side, next, first_rev)?;
        }
        Ok(())
    }

    /// Commit `batch` (a replicated node write to `workspace` that this path
    /// did not compound-index — a cold upsert, or a legacy op handler that
    /// writes no index entries at all) together with the stale mark for the
    /// workspace's compound indexes, atomically, and request a local build.
    ///
    /// A failure here fails the apply and nothing was written, so the op is
    /// redelivered — the only answer that cannot leave `Ready` over stale
    /// entries.
    pub(in crate::replication::application) fn write_marking_compound_stale(
        &self,
        batch: WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<()> {
        let store = crate::compound_state::CompoundStateStore::new(self.db.clone());
        let marked = store.write_marking_stale(batch, tenant_id, repo_id, branch, workspace)?;
        // Every mark asks for the local build that re-earns `Ready` (the job
        // event handler drains the request: warm definitions, sweep builds).
        if marked > 0 {
            cold::request_build(&self.db, tenant_id, repo_id, branch, workspace);
        }
        if marked > 0 {
            tracing::debug!(
                tenant_id,
                repo_id,
                branch,
                workspace,
                marked,
                "replicated write: compound indexes marked NotBuilt until a local rebuild"
            );
        }
        Ok(())
    }
}
