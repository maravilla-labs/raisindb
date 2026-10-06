//! The `node_delete_index` repair: derive `NODE_DELETES` from the `NODES`
//! tombstones of one branch, then stamp the branch `Ready`.
//!
//! Streaming and bounded like every repair (`BoundedWriter`: batches with
//! the resume cursor in the same write, rate limit, disk precheck). Data
//! detected and idempotent: a tombstone whose entry exists writes nothing,
//! so a re-run after a crash, an ingest or a copy only fills the gaps.
//!
//! **Readiness.** The run begins a build generation ([`super::state`]) before
//! its first key and stamps `Ready` after its last batch is committed, only
//! if no invalidation (an ingest, a copy into the branch) happened since.
//! Deletes committed while it runs write their own entries (the funnel), so a
//! scan that passes their key before or after they land loses nothing. A
//! resumed run keeps its generation only while the record still says
//! `Building` under it; otherwise it starts again from the first key.

use crate::management::async_indexing::node_key_parse::parse_node_key;
use crate::management::async_indexing::repair::BoundedWriter;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use rocksdb::{ReadOptions, DB};
use serde::{Deserialize, Serialize};

const PASS: &str = "node_deletes";

/// What the backfill found on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDeleteCounts {
    /// `NODES` delete tombstones seen.
    pub tombstones: u64,
    /// Entries written (or, in a dry run, that would be).
    pub written: u64,
    /// Whether the run stamped the branch `Ready`.
    #[serde(default)]
    pub ready: bool,
}

/// `(tenant, repo, branch)`.
pub(crate) type Scope<'a> = (&'a str, &'a str, &'a str);

/// Start (or resume) the build generation; returns it. A dry run begins
/// nothing.
pub(crate) fn begin(
    db: &DB,
    scope: Scope<'_>,
    writer: &mut BoundedWriter<'_>,
    resumed: bool,
) -> Result<Option<u64>> {
    if writer.dry_run() {
        return Ok(None);
    }
    let resume = resumed
        .then(|| writer.state().epoch.as_deref()?.parse::<u64>().ok())
        .flatten();
    let (generation, continued) = super::state::begin_build(db, scope, resume)?;
    if !continued {
        writer.clear_cursor();
    }
    writer.set_epoch(Some(generation.to_string()));
    Ok(Some(generation))
}

/// Stream the branch's `NODES` and put the entry of every tombstone that has
/// none. Returns `false` when the run must stop early (resumable).
pub(crate) fn node_delete_pass(
    db: &DB,
    (tenant_id, repo_id, branch): Scope<'_>,
    writer: &mut BoundedWriter<'_>,
    counts: &mut NodeDeleteCounts,
) -> Result<bool> {
    writer.begin_pass(PASS);
    let branch_prefix = keys::branch_prefix(tenant_id, repo_id, branch);
    let cursor = writer
        .state()
        .cursor
        .as_deref()
        .and_then(|h| hex::decode(h).ok());

    let cf_nodes = cf_handle(db, cf::NODES)?;
    let cf_deletes = cf_handle(db, cf::NODE_DELETES)?;
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.fill_cache(false);
    if let Some(upper) = crate::prefix_successor(&branch_prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf_nodes, opts);
    match cursor {
        Some(mut after) => {
            after.push(0);
            iter.seek(&after);
        }
        None => iter.seek(&branch_prefix),
    }

    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        writer.note_read((key.len() + value.len()) as u64);
        if keys::is_tombstone_value(value) {
            if let Some((workspace, node_id, revision)) = parse_node_key(&branch_prefix, key) {
                counts.tombstones += 1;
                let entry =
                    super::entry_key(tenant_id, repo_id, branch, workspace, node_id, &revision);
                let present = db
                    .get_pinned_cf(cf_deletes, &entry)
                    .map_err(|e| raisin_error::Error::storage(e.to_string()))?
                    .is_some();
                if !present {
                    writer.put(cf::NODE_DELETES, &entry, b"")?;
                    counts.written += 1;
                }
            }
        }
        if !writer.checkpoint_scan(PASS, key)? {
            return Ok(false);
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(true)
}

/// The run's last batch is committed (`done`): stamp `Ready` under the
/// generation it began with.
pub(crate) fn finish(db: &DB, scope: Scope<'_>, generation: Option<u64>) -> Result<bool> {
    match generation {
        Some(generation) => super::state::finish_build(db, scope, generation),
        None => Ok(false),
    }
}
