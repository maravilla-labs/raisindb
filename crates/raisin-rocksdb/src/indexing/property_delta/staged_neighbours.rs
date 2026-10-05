//! The REFERENCE, SPATIAL and UNIQUE halves of a commit-time correction
//! (`StagedDeltaCheck::revalidate_final`, plan Phase 7b).
//!
//! None of these indexes skips, so none can lose an entry the winning version
//! needs. What they CAN do under a race is keep one: their stale-entry
//! tombstones are derived from the version the write READ at staging, so a
//! version committed meanwhile (below this write, or above it) leaves values
//! nothing ends — `REFERENCES(x)` or `ST_DWITHIN` matching a node whose
//! newest version has neither, or a UNIQUE claim on a value the node no
//! longer holds (every other node is then refused that value). The
//! correction ends them through the same helpers every writer uses
//! (`add_stale_reference_tombstones`, `tombstone_superseded_spatial_indexes`,
//! `unique_delta::{write_unique_delta, end_claims_at}`), never a second
//! implementation:
//!
//! - the true predecessor's values the final state lacks, at the revision;
//! - with versions stored ABOVE the revision, the final state's values the
//!   first of them lacks, at that version's revision (the replica's
//!   `tombstone_superseded_by_newer`, on the origin).
//!
//! UNIQUE needs the types' `unique: true` names, read from the definitions
//! cache (this runs under the commit lock: no NodeType read). The write that
//! is being corrected resolved them at staging, so the cache is warm; when it
//! is not (evicted between staging and commit), the UNIQUE half is skipped
//! and logged — a stale claim refuses a value, it never admits a duplicate.

use crate::indexing::{IndexCtx, NodeSpatialPolicies, SpatialIndexTargets};
use crate::mvcc_read::StoredVersion;
use crate::repositories::nodes::CommitClaims;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};

/// End what `prior` (the version stored just below `revision`) indexed that
/// `final_version` (`None`: a delete) does not, and what `final_version`
/// indexes that the first of `successors` does not. `in_place`: the write
/// reuses `revision`, and `prior` is the content stored AT it.
#[allow(clippy::too_many_arguments)]
pub(super) fn correct_reference_and_spatial(
    db: &std::sync::Arc<DB>,
    batch: &mut WriteBatch,
    ctx: &IndexCtx<'_>,
    prior: Option<&Node>,
    final_version: Option<&Node>,
    successors: &[StoredVersion],
    revision: &HLC,
    in_place: bool,
    held: Option<&CommitClaims>,
) -> Result<()> {
    let Some(policy_node) = final_version.or(prior) else {
        return Ok(());
    };
    let cf_reference = cf_handle(db, cf::REFERENCE_INDEX)?;
    let spatial = SpatialIndexTargets {
        spatial_index: cf_handle(db, cf::SPATIAL_INDEX)?,
    };
    let policies = NodeSpatialPolicies::from_local_state(
        &crate::spatial_state::SpatialStateStore::new(db.clone()),
        ctx,
        policy_node,
    );
    let end = |batch: &mut WriteBatch, old: &Node, new: Option<&Node>, at: &HLC| -> Result<()> {
        let emptied;
        let superseding = match new {
            Some(new) => new,
            None => {
                emptied = Node {
                    properties: Default::default(),
                    ..old.clone()
                };
                &emptied
            }
        };
        crate::repositories::add_stale_reference_tombstones(
            batch,
            cf_reference,
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            old,
            superseding,
            at,
        );
        crate::indexing::tombstone_superseded_spatial_indexes(
            batch, &spatial, ctx, old, new, at, &policies,
        )
    };
    if let Some(prior) = prior {
        end(batch, prior, final_version, revision)?;
    }
    if let (Some(final_version), Some((first_rev, first))) = (final_version, successors.first()) {
        end(batch, final_version, first.as_ref(), first_rev)?;
    }
    correct_unique(
        db,
        batch,
        ctx,
        prior,
        final_version,
        successors,
        revision,
        in_place,
        held,
    )
}

/// The UNIQUE half: claims of `prior` the final state lacks end at
/// `revision`; claims of the final state the first successor lacks end at
/// the successor's revision. A claim another node of this commit holds
/// (`held`) is never ended: this correction is APPENDED to the batch, after
/// that node's put of the same key (plan Phase 13a).
#[allow(clippy::too_many_arguments)]
fn correct_unique(
    db: &DB,
    batch: &mut WriteBatch,
    ctx: &IndexCtx<'_>,
    prior: Option<&Node>,
    final_version: Option<&Node>,
    successors: &[StoredVersion],
    revision: &HLC,
    in_place: bool,
    held: Option<&CommitClaims>,
) -> Result<()> {
    use crate::repositories::nodes::crud::indexing::unique_delta::{
        end_claims_at_held, write_unique_ends, write_unique_puts, UniqueSide,
    };
    let first = successors.first();
    let types = || {
        prior
            .into_iter()
            .chain(final_version)
            .chain(first.and_then(|(_, v)| v.as_ref()))
            .map(|n| n.node_type.as_str())
    };
    // A type that was cold when the commit collected its claims may have
    // warmed since: its taker's claims are then missing from `held`, and a
    // correction run now could end them. Skip it like a cold type.
    if held.is_some_and(|held| held.any_cold(types())) {
        tracing::debug!(
            node_id = ?final_version.or(prior).map(|n| n.id.as_str()),
            "unique definitions were cold when the commit's claims were collected: UNIQUE claims not re-validated"
        );
        return Ok(());
    }
    let scope = raisin_storage::BranchScope::new(ctx.tenant_id, ctx.repo_id, ctx.branch);
    let Some(defs) = crate::indexing::compound::DefsSet::peek(db, scope, types()) else {
        tracing::debug!(
            node_id = ?final_version.or(prior).map(|n| n.id.as_str()),
            "unique definitions not cached at commit: UNIQUE claims not re-validated"
        );
        return Ok(());
    };
    fn sided<'a>(defs: &'a crate::indexing::compound::DefsSet, node: &'a Node) -> UniqueSide<'a> {
        UniqueSide {
            node,
            properties: defs.unique(&node.node_type),
        }
    }
    let side = |node| sided(&defs, node);
    match (prior, final_version) {
        (prior, Some(final_version)) => {
            let new = side(final_version);
            write_unique_ends(
                batch,
                db,
                ctx,
                prior.map(&side),
                new,
                revision,
                in_place,
                held,
            )?;
            write_unique_puts(batch, db, ctx, new, revision, in_place)?;
        }
        (Some(prior), None) => {
            end_claims_at_held(batch, db, ctx, side(prior), None, revision, held)?
        }
        (None, None) => {}
    }
    if let (Some(final_version), Some((first_rev, first))) = (final_version, first) {
        end_claims_at_held(
            batch,
            db,
            ctx,
            side(final_version),
            first.as_ref().map(&side),
            first_rev,
            held,
        )?;
    }
    Ok(())
}

/// Record in `held` the UNIQUE claims `node` holds at `revision`, from the
/// definitions cache. A cold type records none and is marked cold, so a
/// correction needing it ends no claim either — even if the cache warms
/// between this pass and the correction pass (one snapshot per commit).
pub(super) fn hold_claims(
    db: &DB,
    ctx: &IndexCtx<'_>,
    node: &Node,
    revision: &HLC,
    held: &mut CommitClaims,
) {
    use crate::repositories::nodes::crud::indexing::unique_delta::{unique_claims, UniqueSide};
    let scope = raisin_storage::BranchScope::new(ctx.tenant_id, ctx.repo_id, ctx.branch);
    let Some(defs) = crate::indexing::compound::DefsSet::peek(db, scope, [node.node_type.as_str()])
    else {
        held.mark_cold(&node.node_type);
        return;
    };
    let claims = unique_claims(UniqueSide {
        node,
        properties: defs.unique(&node.node_type),
    });
    held.hold(ctx, &claims, revision, &node.id);
}
