//! What a compound build must establish BEFORE it clears anything, and what
//! keeps it from stamping `Ready` afterwards (`build.rs` is the pass itself).

use super::build::{dry_run, Wanted};
use crate::cf;
use crate::indexing::IndexCtx;
use crate::management::async_indexing::repair;
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use rocksdb::DB;

/// What one pass saw.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuildOutcome {
    /// Nodes with at least one version of a wanted type.
    pub nodes: usize,
    /// Live entries written.
    pub entries: usize,
    /// Nodes that could not be decoded or placed (no path).
    pub unplaceable: usize,
    /// Nodes an index wants but cannot hold: a version with every leading
    /// column and no system timestamp in the ORDER column (legacy data,
    /// `entries::unrepresentable`). The index would list them nowhere while
    /// the row scan lists them with a NULL, so the build refuses `Ready`.
    pub unindexable: usize,
    /// Bytes the pass writes (or, dry, would write): keys, values and a
    /// per-entry overhead — what the precheck sizes free space against.
    pub estimated_bytes: u64,
}

impl BuildOutcome {
    /// Whether every wanted node was written: the only outcome a build may
    /// stamp `Ready` on.
    pub fn complete(&self) -> bool {
        self.unplaceable == 0 && self.unindexable == 0
    }
}

/// Before anything is cleared: the disk headroom check and a read-only pass
/// that refuses when any node cannot be placed or indexed — the clear would
/// delete entries the build cannot write back — and when the volume cannot
/// take what the pass would write.
///
/// Headroom is checked twice. First the long-standing rule (2x the
/// COMPOUND_INDEX column family, which the compaction after a rebuild
/// rewrites), cheaply, before the scan. Then against THIS build's output,
/// which the first rule knows nothing about: an index never built before
/// (the built-in one on every workspace, plan Phase 13f) can be many times the
/// current column family. It needs `2 x estimate` free, and must leave
/// [`BUILD_FREE_FLOOR`] free after writing.
pub fn precheck(db: &DB, ctx: &IndexCtx<'_>, wanted: &Wanted, floor: &HLC) -> Result<()> {
    precheck_assuming(db, ctx, wanted, floor, None)
}

/// [`precheck`] with the free space taken from `free_bytes` when given (a
/// TEST hook: a test cannot shrink the volume it runs on).
pub fn precheck_assuming(
    db: &DB,
    ctx: &IndexCtx<'_>,
    wanted: &Wanted,
    floor: &HLC,
    free_bytes: Option<u64>,
) -> Result<()> {
    repair::check_headroom_assuming(db, cf::COMPOUND_INDEX, free_bytes)?;
    let seen = dry_run(db, ctx, wanted, floor)?;
    refuse_unplaceable(ctx, &seen)?;
    repair::check_output_headroom(db, seen.estimated_bytes, BUILD_FREE_FLOOR, free_bytes).map_err(
        |e| {
            Error::Validation(format!(
                "refusing to build compound indexes for {}/{}/{}/{}: {e}",
                ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace
            ))
        },
    )
}

/// Free space a compound build must leave on the data volume after writing
/// its estimated output: the database must never be driven to a full disk by
/// a background build (RocksDB goes read-only there).
pub const BUILD_FREE_FLOOR: u64 = 256 * 1024 * 1024;

/// The error a build returns when nodes could not be placed or indexed
/// ([`BuildOutcome::complete`] is false).
pub fn refuse_unplaceable(ctx: &IndexCtx<'_>, seen: &BuildOutcome) -> Result<()> {
    if seen.unplaceable > 0 {
        return Err(Error::storage(format!(
            "refusing to build compound indexes for {}/{}/{}/{}: {} node(s) could not be \
             decoded or placed in the tree (no path); the index stays unusable (scan) until \
             that is repaired",
            ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace, seen.unplaceable
        )));
    }
    if seen.unindexable > 0 {
        return Err(Error::storage(format!(
            "refusing to build compound indexes for {}/{}/{}/{}: {} node(s) have no value for \
             an index's order column (a version written before created_at/updated_at were \
             stamped); the index could not list them, so it stays unusable (scan) until they \
             are rewritten",
            ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace, seen.unindexable
        )));
    }
    Ok(())
}
