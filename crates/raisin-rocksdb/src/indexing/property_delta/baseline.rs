//! The one place a property-index baseline is decided.

use super::OwnedBaseline;
use crate::indexing::IndexCtx;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::DB;

/// THE baseline decision for a write of `node_id` at `revision`.
///
/// `skip_permitted` is the caller's gate — `index.skip_unchanged` on AND this
/// node's property index rebuilt by this writer (see
/// `NodeRepositoryImpl::skip_unchanged_permitted`). Without it nothing is
/// skipped: the answer is [`OwnedBaseline::Full`] of `fallback` (the caller's
/// own view of the prior version) — UNLESS versions above `revision` are
/// already stored. That check runs whatever the gate says (one seek, normally
/// empty): once a branch has had skip writes, a successor may keep unchanged
/// entries below `revision`, and a full put's tombstones there would mask them.
/// Turning the flag off (the documented rollback) or a reset gate (a verify
/// miss, a checkpoint ingest) must not drop that protection
/// (`out_of_order_write_after_rollback_keeps_successor_entries`).
///
/// With the gate open, the stored versions decide, through the one baseline
/// reader:
///
/// - newest stored version below `revision`, live: `Predecessor(it)`;
/// - newest stored version below `revision`, a tombstone, or none at all:
///   `NoPrior` (no live version exists on the branch);
/// - versions ABOVE `revision` exist (a write committed below one already
///   stored — out of order, or a node stranded above HEAD):
///   `OutOfOrder { newest strictly below, those versions }`;
/// - a version AT `revision` (an in-place write the caller did not mark):
///   `Full(newest strictly below)`.
#[allow(clippy::too_many_arguments)]
pub fn resolve_baseline(
    db: &DB,
    ctx: &IndexCtx<'_>,
    node_id: &str,
    revision: &HLC,
    fallback: Option<&Node>,
    skip_permitted: bool,
) -> Result<OwnedBaseline> {
    if !skip_permitted {
        let successors = crate::mvcc_read::node_versions_above(
            db,
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            node_id,
            revision,
        )?;
        if successors.is_empty() {
            return Ok(OwnedBaseline::Full(fallback.cloned()));
        }
        let prior = crate::mvcc_read::node_version_before(
            db,
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            node_id,
            revision,
        )?
        .and_then(|(_, node)| node);
        return Ok(OwnedBaseline::OutOfOrder { prior, successors });
    }
    let newest = crate::mvcc_read::node_version_at_or_before(
        db,
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        node_id,
        None,
    )?;
    Ok(match newest {
        None => OwnedBaseline::NoPrior,
        Some((at, None)) if at < *revision => OwnedBaseline::NoPrior,
        Some((at, Some(node))) if at < *revision => OwnedBaseline::Predecessor(node),
        Some((at, _)) => {
            let prior = crate::mvcc_read::node_version_before(
                db,
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                ctx.workspace,
                node_id,
                revision,
            )?
            .and_then(|(_, node)| node);
            if at == *revision {
                OwnedBaseline::Full(prior)
            } else {
                OwnedBaseline::OutOfOrder {
                    prior,
                    successors: crate::mvcc_read::node_versions_above(
                        db,
                        ctx.tenant_id,
                        ctx.repo_id,
                        ctx.branch,
                        ctx.workspace,
                        node_id,
                        revision,
                    )?,
                }
            }
        }
    })
}
