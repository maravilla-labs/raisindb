//! The re-derivation half of a commit-time correction
//! (`StagedDeltaCheck::revalidate_final`, plan Phases 7 and 7b): the write of
//! the node's FINAL version against what is stored NOW, through the one
//! writers — PROPERTY_INDEX (`write_property_index_delta` with `Full` /
//! `OutOfOrder`, or `tombstone_all_entries` + `reassert_successors` for a
//! delete), COMPOUND (`staged_compound`) and REFERENCE / SPATIAL / UNIQUE
//! (`staged_neighbours`). There is no second derivation.

use super::{
    reassert_successors, tombstone_all_entries, write_property_index_delta, Baseline, InPlace,
    InPlaceTargets, PropertyIndexTarget,
};
use crate::indexing::IndexCtx;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};

/// Append to `batch` the write of `final_version` (`None`: a delete) at
/// `revision` against the versions stored now. `in_place`: the write reuses
/// `revision` (`Some(resolved_targets)`); its baseline is then the content
/// stored AT the revision — a racing in-place rewrite's, which the staged
/// write never saw — and its entries are placed like the staged write's.
pub(super) fn rederive(
    db: &std::sync::Arc<DB>,
    batch: &mut WriteBatch,
    ctx: &IndexCtx<'_>,
    node_id: &str,
    revision: &HLC,
    in_place: Option<bool>,
    final_version: Option<&Node>,
    held: Option<&crate::repositories::nodes::CommitClaims>,
) -> Result<()> {
    let (t, r, b, w) = (ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace);
    // An in-place write replaces what is stored AT its revision (a racing
    // in-place rewrite's content); any other write, the version below.
    let prior = match in_place {
        Some(_) => {
            crate::mvcc_read::node_version_at_or_before(db, t, r, b, w, node_id, Some(revision))?
        }
        None => crate::mvcc_read::node_version_before(db, t, r, b, w, node_id, revision)?,
    }
    .and_then(|(_, node)| node);
    let successors = crate::mvcc_read::node_versions_above(db, t, r, b, w, node_id, revision)?;
    // REFERENCE, SPATIAL and UNIQUE derive their stale-entry tombstones
    // from the version the write read too: end what the true neighbours
    // call for.
    super::staged_neighbours::correct_reference_and_spatial(
        db,
        batch,
        ctx,
        prior.as_ref(),
        final_version,
        &successors,
        revision,
        in_place.is_some(),
        held,
    )?;
    match final_version {
        Some(node) => {
            let baseline = if successors.is_empty() {
                Baseline::Full(prior.as_ref())
            } else {
                Baseline::OutOfOrder {
                    prior: prior.as_ref(),
                    successors: &successors,
                }
            };
            let targets = match in_place {
                Some(true) => Some(InPlaceTargets::resolve(
                    db,
                    ctx,
                    prior.as_ref(),
                    node,
                    revision,
                )?),
                _ => None,
            };
            write_property_index_delta(
                batch,
                PropertyIndexTarget::from_db(db)?,
                ctx,
                baseline,
                node,
                revision,
                match in_place {
                    Some(_) => InPlace::Reused(targets.as_ref()),
                    None => InPlace::No,
                },
            )?;
            // The compound write was staged against the same baseline and
            // may have skipped unchanged tuples: correct it the same way.
            super::staged_compound::correct_compound(db, batch, ctx, baseline, node, revision)?;
        }
        None => {
            let cf = cf_handle(db, cf::PROPERTY_INDEX)?;
            if let Some(prior) = &prior {
                tombstone_all_entries(batch, cf, ctx, prior, revision);
            }
            reassert_successors(batch, cf, ctx, node_id, &successors);
            // The one compound delete tombstoner, against the version in
            // force below the delete (it re-asserts successors itself).
            let ended = prior
                .as_ref()
                .or_else(|| successors.iter().find_map(|(_, v)| v.as_ref()));
            if let Some(ended) = ended {
                crate::indexing::compound::tombstone_compound_for_delete(
                    batch, db, ctx, ended, revision,
                )?;
            }
        }
    }
    Ok(())
}
