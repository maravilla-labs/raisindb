//! What a staged write records about a node's stored versions, and how the
//! commit tells an INDEX-ONLY write from a record write (plan Phase 7b).
//!
//! The markers are the node's newest NODES version and its newest NODE_PATH
//! entry, each as its revision plus a hash of the stored bytes (so an
//! in-place `versionable=false` rewrite at the same revision counts as a
//! change).
//!
//! Every record write (`write_node_record`, the replication applicator, a
//! move's rewritten records, the `node_path` backfill for a version already
//! stored) writes its NODE_PATH entry at a revision that ALSO holds a NODES
//! version. Only an index-only re-key — an ancestor move re-pathing a
//! descendant it does not rewrite — writes a NODE_PATH entry with no NODES
//! version at its revision. That is the one change the NODES-driven
//! correction cannot repair (the compound entries it wrote were derived from
//! the version that move read), so it is the one change that fails the
//! workspace's compound indexes closed ([`index_only_path_write`]). Treating
//! every NODE_PATH change as one marked a hot node's whole workspace
//! `NotBuilt` on every ordinary write race.

use crate::indexing::IndexCtx;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{ReadOptions, DB};
use std::hash::{Hash, Hasher};

/// A newest stored entry: its revision and a hash of its bytes.
pub(super) type Marker = Option<(HLC, u64)>;
/// `(newest NODES version, newest NODE_PATH entry)`.
pub(super) type Markers = (Marker, Marker);

fn hash_bytes(value: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// `node_id`'s newest stored NODES version and newest NODE_PATH entry.
pub(super) fn markers(db: &DB, ctx: &IndexCtx<'_>, node_id: &str) -> Result<Markers> {
    let newest = |cf_name: &str, prefix: Vec<u8>| {
        crate::mvcc_read::newest_at_or_before_with(
            db,
            cf_handle(db, cf_name)?,
            &prefix,
            None,
            |revision, value| (revision, hash_bytes(value)),
        )
    };
    let (t, r, b, w) = (ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace);
    Ok((
        newest(cf::NODES, keys::node_key_prefix(t, r, b, w, node_id))?,
        newest(
            cf::NODE_PATH,
            keys::node_path_key_prefix(t, r, b, w, node_id),
        )?,
    ))
}

/// Whether a NODE_PATH entry written since `at_stage` (the newest entry the
/// write recorded) has no NODES version of the node at its own revision —
/// an index-only re-key, not a record write. Newest first; stops at the
/// recorded entry. Normally one or two entries are read.
pub(super) fn index_only_path_write(
    db: &DB,
    ctx: &IndexCtx<'_>,
    node_id: &str,
    at_stage: &Marker,
) -> Result<bool> {
    let (t, r, b, w) = (ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace);
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let nodes_prefix = keys::node_key_prefix(t, r, b, w, node_id);
    let path_prefix = keys::node_path_key_prefix(t, r, b, w, node_id);
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    if let Some(upper) = crate::prefix_successor(&path_prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf_handle(db, cf::NODE_PATH)?, opts);
    iter.seek(&path_prefix);
    while iter.valid() {
        let Some(key) = iter.key() else { break };
        if !key.starts_with(&path_prefix) {
            break;
        }
        let Ok(revision) = keys::extract_revision_from_key(key) else {
            iter.next();
            continue;
        };
        if let Some((stage_revision, stage_hash)) = at_stage {
            let unchanged = revision == *stage_revision
                && hash_bytes(iter.value().unwrap_or_default()) == *stage_hash;
            if revision < *stage_revision || unchanged {
                break;
            }
        }
        let record_at = crate::mvcc_read::newest_at_or_before_with(
            db,
            cf_nodes,
            &nodes_prefix,
            Some(&revision),
            |at, _| at,
        )?;
        if record_at != Some(revision) {
            return Ok(true);
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(false)
}
