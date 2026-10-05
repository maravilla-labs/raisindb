//! The COMPOUND half of a commit-time correction (`StagedDeltaCheck`, plan
//! Phases 8 and 7b): the compound write was staged against the same baseline
//! as the property write, so it is corrected the same way — or, when the
//! definitions are not cached or an index-only write (an ancestor move)
//! landed meanwhile, the workspace's compound indexes fail CLOSED.
//!
//! A move's re-key of a descendant it does not rewrite ([`correct_rekey`]) is
//! the compound half alone: no record is written, so the descendant's
//! compound entries must follow the version STORED at the move's revision,
//! not the one the move listed before an update committed in between.

use crate::compound_state::StaleScope;
use crate::indexing::{Baseline, IndexCtx};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};

/// The full compound write of `node` against what is stored now. Definitions
/// from the cache (this runs under the commit lock: no NodeType read); cold →
/// the types' compound indexes are marked `NotBuilt` BEFORE this batch
/// commits (the safe order — a crash costs a rebuild, never a `Ready` over a
/// missed correction) and a local build is requested. The workspace's own
/// indexes need no NodeType (plan Phase 13e): corrected either way.
pub(super) fn correct_compound(
    db: &std::sync::Arc<DB>,
    batch: &mut WriteBatch,
    ctx: &IndexCtx<'_>,
    baseline: Baseline<'_>,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    use crate::indexing::compound::{types_of, write_compound_delta, writes_compound, DefsSet};
    let scope = raisin_storage::BranchScope::new(ctx.tenant_id, ctx.repo_id, ctx.branch);
    let types = types_of(&baseline, node);
    match DefsSet::peek(db, scope, types.iter().copied()) {
        Some(defs) => {
            if writes_compound(db, ctx, &defs)? {
                write_compound_delta(batch, db, ctx, &defs, baseline, node, revision)?;
            }
        }
        None => {
            let workspace_only = DefsSet::workspace_only(types.iter().copied());
            write_compound_delta(batch, db, ctx, &workspace_only, baseline, node, revision)?;
            fail_compound_closed(db, ctx, &types, StaleScope::TypeOwned)?;
        }
    }
    Ok(())
}

/// Mark the workspace's compound indexes `scope` covers `NotBuilt` BEFORE
/// the batch commits (a crash costs a rebuild, never a `Ready` over a missed
/// correction) and request the local build, naming `types` so the drain
/// resolves them.
pub(super) fn fail_compound_closed(
    db: &std::sync::Arc<DB>,
    ctx: &IndexCtx<'_>,
    types: &[&str],
    scope: StaleScope,
) -> Result<()> {
    crate::compound_state::CompoundStateStore::new(db.clone()).mark_workspace_stale_in(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        scope,
    )?;
    crate::indexing::compound::cold::request_build_for(
        db,
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        types,
    );
    Ok(())
}

/// A move re-keyed `listed` (the version it read) to `moved` (the same
/// content at its new path) at `revision`, without writing a record. When
/// the version stored below `revision` now is not `listed` — an update
/// committed between the listing and this commit — re-derive the re-key from
/// the stored version: end what the staged re-key put that the stored version
/// does not index, and end the stored version's tuples at its old path. A
/// node deleted in between has every staged tuple ended. Returns whether a
/// correction was written.
pub(super) fn correct_rekey(
    db: &std::sync::Arc<DB>,
    batch: &mut WriteBatch,
    ctx: &IndexCtx<'_>,
    listed: &Node,
    moved: Option<&Node>,
    revision: &HLC,
) -> Result<bool> {
    use crate::indexing::compound::{
        tombstone_compound_for_delete, write_compound_delta, writes_compound, DefsSet,
    };
    let Some(moved) = moved else {
        return Ok(false);
    };
    let stored = crate::mvcc_read::node_version_before(
        db,
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &listed.id,
        revision,
    )?;
    let stored = match stored {
        // Nothing stored below the move: nothing listed it from storage.
        None => return Ok(false),
        Some((_, None)) => {
            tombstone_compound_for_delete(batch, db, ctx, moved, revision)?;
            return Ok(true);
        }
        Some((_, Some(stored))) => stored,
    };
    // Fields a read fills in, not content: never a reason to re-derive.
    let mut as_listed = stored.clone();
    as_listed.path = listed.path.clone();
    as_listed.workspace = listed.workspace.clone();
    as_listed.has_children = listed.has_children;
    as_listed.children = listed.children.clone();
    as_listed.tenant_id = listed.tenant_id.clone();
    if as_listed == *listed {
        return Ok(false);
    }
    let mut stored_moved = stored.clone();
    stored_moved.path = moved.path.clone();
    stored_moved.name = moved.name.clone();
    stored_moved.parent = moved.parent.clone();
    let types = [
        listed.node_type.as_str(),
        stored.node_type.as_str(),
        moved.node_type.as_str(),
    ];
    let scope = raisin_storage::BranchScope::new(ctx.tenant_id, ctx.repo_id, ctx.branch);
    let defs = DefsSet::peek(db, scope, types);
    let writes = match &defs {
        Some(defs) => writes_compound(db, ctx, defs)?,
        None => false,
    };
    match defs {
        Some(defs) if writes => {
            // What the staged re-key put that the stored version lacks …
            write_compound_delta(
                batch,
                db,
                ctx,
                &defs,
                Baseline::Full(Some(moved)),
                &stored_moved,
                revision,
            )?;
            // … and the stored version's tuples at its old path.
            write_compound_delta(
                batch,
                db,
                ctx,
                &defs,
                Baseline::Full(Some(&stored)),
                &stored_moved,
                revision,
            )?;
            Ok(true)
        }
        Some(_) => Ok(false),
        None => {
            fail_compound_closed(db, ctx, &types, StaleScope::All)?;
            Ok(true)
        }
    }
}
