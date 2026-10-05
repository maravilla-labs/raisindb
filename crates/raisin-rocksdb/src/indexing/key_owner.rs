//! Whether a stale PATH_INDEX placement is still THIS node's to end — the one
//! check every writer that retires a node's old path makes (plan Phase 13a).
//!
//! PATH_INDEX keys carry no node id (`{path}\0{~revision}`, the owner in the
//! value), so a tombstone for a node's OLD path at revision R lands on the
//! same key as whatever else maps that path at R, and masks every mapping
//! below R. Ending the old path is therefore only right while the path's
//! newest entry at or before R still names the node. Otherwise the path
//! already belongs to another node (a node created or moved into it, a
//! stranded duplicate that owns the path while this node's record still
//! claims it) or is already ended.
//!
//! Three writers retire an old path, and all three ask here:
//!
//! - the replicated upsert (`replication::application::applicator`), for a
//!   moved node's old path;
//! - cross-branch promotion (`cross_branch::helpers::tombstone_stale_placement`)
//!   on the origin, so the origin and its replicas keep the same mapping;
//! - a merge resolution (`branches::merge::merged_view`), which asks about
//!   the MERGED view — the target's entries at or before the merge revision
//!   and the source's entries the merge copy is about to replay below it.

use crate::indexing::IndexCtx;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{AsColumnFamilyRef, DB};

/// One branch's entries of a key, bounded by a revision.
pub(crate) struct KeyView<'a> {
    pub prefix: Vec<u8>,
    pub at: &'a HLC,
}

/// The newest entry across `views` (each at or before its own bound). On a
/// revision tie the EARLIER view wins: a merge copy never overwrites a key
/// the target already holds (`branches::copy_existing`).
pub(crate) fn newest_across(
    db: &DB,
    cf: &impl AsColumnFamilyRef,
    views: &[KeyView<'_>],
) -> Result<Option<(HLC, Vec<u8>)>> {
    let mut newest: Option<(HLC, Vec<u8>)> = None;
    for view in views {
        if let Some((revision, value)) =
            crate::mvcc_read::newest_at_or_before(db, cf, &view.prefix, Some(view.at))?
        {
            if newest.as_ref().is_none_or(|(best, _)| revision > *best) {
                newest = Some((revision, value));
            }
        }
    }
    Ok(newest)
}

/// Whether the newest PATH_INDEX entry across `views` maps the path to
/// `node_id`.
pub(crate) fn path_names_node(db: &DB, views: &[KeyView<'_>], node_id: &str) -> Result<bool> {
    let cf_path = cf_handle(db, cf::PATH_INDEX)?;
    Ok(newest_across(db, cf_path, views)?.is_some_and(|(_, value)| value == node_id.as_bytes()))
}

/// The PATH_INDEX prefix of `path` on `ctx`'s branch and workspace.
pub(crate) fn path_prefix(ctx: &IndexCtx<'_>, path: &str) -> Vec<u8> {
    keys::path_index_key_prefix(ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace, path)
}

/// Whether `path`'s newest PATH_INDEX entry at or before `at` maps to
/// `node_id` — i.e. whether a tombstone at `at` would end THIS node's entry.
pub(crate) fn path_owned_at(
    db: &DB,
    ctx: &IndexCtx<'_>,
    path: &str,
    node_id: &str,
    at: &HLC,
) -> Result<bool> {
    path_names_node(
        db,
        &[KeyView {
            prefix: path_prefix(ctx, path),
            at,
        }],
        node_id,
    )
}
