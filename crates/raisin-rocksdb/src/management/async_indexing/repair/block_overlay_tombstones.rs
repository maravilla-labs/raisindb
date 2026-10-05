//! `block_overlay_tombstones`: store the `T` every node delete owes its block
//! overlays (plan Phase 11c).
//!
//! Until Phase 11c a node delete tombstoned none of its `BLOCK_TRANSLATIONS`
//! overlays: they stayed live in storage forever, retention GC kept the
//! newest one (it is not a tombstone), and every raw scan of the CF saw them.
//! Reads no longer need this repair — the node-delete read rule ends block
//! overlays too (`translation_read`) — so it is cleanup, never the fix: it
//! writes, for every delete of every node with block overlays, exactly what
//! the delete funnel now writes at delete time
//! (`translation_write::block_deletion_keys`), at that delete's revision.
//!
//! - **Detected from the data.** One pass over the branch's
//!   `BLOCK_TRANSLATIONS`, a node at a time; per node one bounded walk of its
//!   `NODES` deletes up to the branch HEAD. A node with no delete costs that
//!   walk and nothing else. A `T` already stored is the newest version at its
//!   delete, so a re-run over clean data writes nothing.
//! - **Every delete, not only the newest.** A node deleted and recreated
//!   still owes the first delete its `T`; deletes are processed oldest first
//!   and a version a `T` of this run already ended is not tombstoned again.
//! - **Streaming, resumable, throttled.** Writes go through the bounded writer
//!   with the cursor (the last finished node's key prefix) in the same batch.
//! - Queued automatically per branch after start, like the `node_path`
//!   backfill (`auto_block_overlays`); the admin fan-out endpoint runs it too.

use super::cursor::BoundedWriter;
use crate::repositories::translations::keys as tkeys;
use crate::{cf, cf_handle, keys};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use rocksdb::DB;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The pass name in the state record.
pub(super) const PASS: &str = "block_overlays";

/// What one run over one branch found and wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockOverlayCounts {
    /// Nodes with block overlays on the branch.
    pub nodes: u64,
    /// Of those, nodes with at least one delete at or below HEAD.
    pub deleted_nodes: u64,
    /// `T`s written (in a dry run: that would be).
    pub tombstones: u64,
}

pub(super) struct Scope<'a> {
    pub(super) tenant_id: &'a str,
    pub(super) repo_id: &'a str,
    pub(super) branch: &'a str,
    /// Deletes above it are not considered (an in-flight commit's).
    pub(super) head: Option<HLC>,
}

/// Run the pass. `false`: stopped early (the crash hook), resumable.
pub(super) fn block_overlay_pass(
    db: &DB,
    scope: &Scope<'_>,
    writer: &mut BoundedWriter<'_>,
    counts: &mut BlockOverlayCounts,
) -> Result<bool> {
    writer.begin_pass(PASS);
    let branch_prefix = keys::branch_prefix(scope.tenant_id, scope.repo_id, scope.branch);
    let mut seek = match writer
        .state()
        .cursor
        .as_deref()
        .and_then(|hex| hex::decode(hex).ok())
    {
        Some(done) => match crate::prefix_successor(&done) {
            Some(next) => next,
            None => return Ok(true),
        },
        None => branch_prefix.clone(),
    };
    loop {
        let Some(key) = first_key_at_or_after(db, &branch_prefix, &seek)? else {
            return Ok(true);
        };
        let Some((workspace, node_id)) = parse_node(&branch_prefix, &key) else {
            // Not a block-overlay key of a node: step past it.
            seek = key;
            seek.push(0);
            continue;
        };
        let node_prefix = tkeys::block_translations_node_prefix(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            &workspace,
            &node_id,
        );
        counts.nodes += 1;
        let read = tombstone_node(db, scope, &workspace, &node_id, writer, counts)?;
        // The real bytes: the node's whole `NODES` history, values included.
        writer.note_read(node_prefix.len() as u64 + read);
        if !writer.checkpoint_scan(PASS, &node_prefix)? {
            return Ok(false);
        }
        match crate::prefix_successor(&node_prefix) {
            Some(next) => seek = next,
            None => return Ok(true),
        }
    }
}

/// Queue the `T`s one node's deletes owe its block overlays. Returns the
/// bytes its `NODES` walk read.
fn tombstone_node(
    db: &DB,
    scope: &Scope<'_>,
    workspace: &str,
    node_id: &str,
    writer: &mut BoundedWriter<'_>,
    counts: &mut BlockOverlayCounts,
) -> Result<u64> {
    let at = (scope.tenant_id, scope.repo_id, scope.branch, workspace);
    let (deletes, read) = crate::mvcc_read::deletes_in_range_counted(
        db,
        at,
        node_id,
        &HLC::new(0, 0),
        scope.head.as_ref(),
    )?;
    if deletes.is_empty() {
        return Ok(read);
    }
    counts.deleted_nodes += 1;
    // `(block, locale)` -> the delete this run already tombstoned it at.
    let mut ended: HashMap<(String, String), HLC> = HashMap::new();
    for deleted_at in deletes.iter().rev() {
        for deletion in crate::translation_write::block_deletion_keys(db, at, node_id, deleted_at)?
        {
            let pair = (deletion.block_uuid, deletion.locale);
            if ended
                .get(&pair)
                .is_some_and(|earlier| deletion.live_revision <= *earlier)
            {
                continue; // the earlier `T` (uncommitted yet) already ends it
            }
            writer.put(cf::BLOCK_TRANSLATIONS, &deletion.key, keys::TOMBSTONE_VALUE)?;
            counts.tombstones += 1;
            ended.insert(pair, *deleted_at);
        }
    }
    Ok(read)
}

/// The first `BLOCK_TRANSLATIONS` key at or after `seek` under `prefix`.
fn first_key_at_or_after(db: &DB, prefix: &[u8], seek: &[u8]) -> Result<Option<Vec<u8>>> {
    let cf = cf_handle(db, cf::BLOCK_TRANSLATIONS)?;
    let mut opts = rocksdb::ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.fill_cache(false);
    if let Some(upper) = crate::prefix_successor(prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf, opts);
    iter.seek(seek);
    let key = iter
        .valid()
        .then(|| iter.key().map(<[u8]>::to_vec))
        .flatten()
        .filter(|key| key.starts_with(prefix));
    iter.status().map_err(|e| Error::storage(e.to_string()))?;
    Ok(key)
}

/// `(workspace, node_id)` of `{branch_prefix}{ws}\0block_trans\0{node}\0…`.
fn parse_node(branch_prefix: &[u8], key: &[u8]) -> Option<(String, String)> {
    let mut segments = key.strip_prefix(branch_prefix)?.splitn(4, |b| *b == 0);
    let workspace = std::str::from_utf8(segments.next()?).ok()?;
    if segments.next()? != b"block_trans" {
        return None;
    }
    let node_id = std::str::from_utf8(segments.next()?).ok()?;
    segments.next()?; // the node segment must be terminated
    Some((workspace.to_string(), node_id.to_string()))
}
