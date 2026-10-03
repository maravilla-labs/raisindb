//! Repair ORDERED_CHILDREN entries a delete failed to tombstone.
//!
//! The delete tombstoner keyed its ORDERED_CHILDREN tombstone by the parent's
//! NAME (`Node.parent`) instead of its id, so every delete before the fix left
//! the child's entry live under its real parent. Readers tolerate it (the
//! `has_children` probe confirms liveness and placement); this is cleanup.
//!
//! Two passes, both data-detected and idempotent:
//!
//! 1. **NODES-tombstone pass.** For EVERY delete revision of every node — not
//!    only the latest, so a node deleted and re-created several times gets one
//!    tombstone per delete — read the version just before it, resolve its
//!    parent id and the label actually stored for it AS OF THAT VERSION (never
//!    at HEAD: in a cascade the parent is gone too, or its path now names
//!    another node), and tombstone that exact `(label, child)` key at the
//!    delete revision. A later re-creation at a newer revision stays live.
//! 2. **ORDERED_CHILDREN pass** (`ordered_pass.rs`). Every entry live at HEAD
//!    whose child has no live NODES version at HEAD — including children
//!    `history_gc` dropped every version of — is tombstoned at the child's
//!    delete revision when one is known, else just after the entry's own
//!    revision, and counted. So is an entry whose child is live but placed
//!    under ANOTHER parent (a move that left its old entry live).
//!
//! Root parents (`/`) are covered like any other. Runs per branch, so a fork
//! that copied an untombstoned entry is repaired on its own.

use super::cursor::BoundedWriter;
use super::parent_entries::ParentCache;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{ReadOptions, DB};

pub(super) const PASS_NODES: &str = "nodes";
pub(super) const PASS_ORDERED: &str = "ordered";

/// Counts the ORDERED_CHILDREN repair reports beyond what it wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OrderedChildrenCounts {
    /// Delete revisions examined in the NODES pass.
    pub deletes_seen: u64,
    /// Deletes whose pre-delete parent or label could not be resolved.
    pub unresolved: u64,
    /// Live entries whose child had no live version at HEAD.
    pub orphan_entries: u64,
    /// Of those, the ones whose child left no delete revision (GC-dropped).
    pub without_delete_revision: u64,
    /// Live entries whose child is live but placed under another parent.
    pub misplaced_entries: u64,
}

/// A raw iterator over exactly `prefix`, starting strictly after `cursor`.
pub(super) fn iterate_from<'a>(
    db: &'a DB,
    cf_name: &str,
    prefix: &[u8],
    cursor: Option<&[u8]>,
) -> Result<rocksdb::DBRawIteratorWithThreadMode<'a, DB>> {
    let cf = cf_handle(db, cf_name)?;
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.fill_cache(false);
    if let Some(upper) = crate::prefix_successor(prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf, opts);
    match cursor {
        Some(after) => {
            let mut start = after.to_vec();
            start.push(0);
            iter.seek(&start);
        }
        None => iter.seek(prefix),
    }
    Ok(iter)
}

/// `{ws}\0nodes\0{id}\0{~rev}` after the branch prefix -> (ws, id, revision).
fn parse_node_key(rest: &[u8]) -> Option<(&str, &str, HLC)> {
    let rev_start = rest.len().checked_sub(16)?;
    let head = rest.get(..rev_start.checked_sub(1)?)?;
    if rest[rev_start - 1] != 0 {
        return None;
    }
    let mut parts = head.splitn(3, |b| *b == 0);
    let ws = std::str::from_utf8(parts.next()?).ok()?;
    if parts.next()? != b"nodes" {
        return None;
    }
    let id = std::str::from_utf8(parts.next()?).ok()?;
    if id.contains('\0') {
        return None;
    }
    let revision = HLC::decode_descending(&rest[rev_start..]).ok()?;
    Some((ws, id, revision))
}

/// One step after `rev` in HLC order.
pub(super) fn successor(rev: &HLC) -> HLC {
    match rev.counter.checked_add(1) {
        Some(counter) => HLC::new(rev.timestamp_ms, counter),
        None => HLC::new(rev.timestamp_ms + 1, 0),
    }
}

/// Branch scope shared by both passes.
pub(super) struct Scope<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub head: Option<HLC>,
}

impl Scope<'_> {
    pub(super) fn branch_prefix(&self) -> Vec<u8> {
        keys::branch_prefix(self.tenant_id, self.repo_id, self.branch)
    }
}

/// Parents whose entries the NODES pass keeps in memory at once.
const PARENT_CACHE: usize = 256;

/// Pass 1. Returns `false` when the run must stop early.
pub(super) fn nodes_pass(
    db: &DB,
    scope: &Scope<'_>,
    writer: &mut BoundedWriter<'_>,
    counts: &mut OrderedChildrenCounts,
) -> Result<bool> {
    writer.begin_pass(PASS_NODES);
    let prefix = scope.branch_prefix();
    let cursor = writer
        .state()
        .cursor
        .as_deref()
        .and_then(|h| hex::decode(h).ok());
    let mut iter = iterate_from(db, cf::NODES, &prefix, cursor.as_deref())?;
    let mut parents = ParentCache::new(PARENT_CACHE);

    // The node whose versions are being walked (newest first), and a delete
    // revision still waiting for the version just below it.
    let mut group: Option<(String, String)> = None;
    let mut pending_delete: Option<HLC> = None;
    let mut last_key: Option<Vec<u8>> = None;

    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        let Some((ws, id, revision)) = parse_node_key(&key[prefix.len()..]) else {
            iter.next();
            continue;
        };
        let this = (ws.to_string(), id.to_string());
        if group.as_ref() != Some(&this) {
            // A group boundary: the previous node is fully processed.
            if let Some(done) = last_key.take() {
                if !writer.checkpoint(PASS_NODES, &done)? {
                    return Ok(false);
                }
            }
            group = Some(this);
            pending_delete = None;
        }

        if keys::is_tombstone_value(value) {
            // Two deletes in a row: the newer one had nothing live below it.
            pending_delete = Some(revision);
            counts.deletes_seen += 1;
        } else if let Some(deleted_at) = pending_delete.take() {
            let (ws, id) = group.clone().unwrap();
            let delete = Delete {
                workspace: &ws,
                node_id: &id,
                blob: value,
                before: revision,
                deleted_at,
            };
            repair_one_delete(db, scope, &delete, &mut parents, writer, counts)?;
        }
        last_key = Some(key.to_vec());
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    if let Some(done) = last_key {
        if !writer.checkpoint(PASS_NODES, &done)? {
            return Ok(false);
        }
    }
    // Commit before the ORDERED_CHILDREN pass reads: it must see this pass's
    // tombstones, or it would tombstone the same entries a second time.
    writer.commit("running")?;
    Ok(true)
}

/// One delete found by the NODES pass.
struct Delete<'a> {
    workspace: &'a str,
    node_id: &'a str,
    /// The version just before the delete.
    blob: &'a [u8],
    before: HLC,
    deleted_at: HLC,
}

/// Tombstone, at the delete revision, the ORDERED_CHILDREN entry the version
/// just before it occupied — the entry its delete should have tombstoned.
fn repair_one_delete(
    db: &DB,
    scope: &Scope<'_>,
    delete: &Delete<'_>,
    parents: &mut ParentCache,
    writer: &mut BoundedWriter<'_>,
    counts: &mut OrderedChildrenCounts,
) -> Result<()> {
    let (t, r, b, ws) = (
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        delete.workspace,
    );
    let Ok(node) = crate::mvcc_read::deserialize_node_with_path(
        db,
        delete.blob,
        t,
        r,
        b,
        ws,
        delete.node_id,
        &delete.before,
    ) else {
        counts.unresolved += 1;
        return Ok(());
    };
    // Parent as of the version before the delete — the one resolver every
    // delete-path writer shares.
    let Some(parent_id) = crate::repositories::nodes::parent_index_id(
        db,
        t,
        r,
        b,
        ws,
        &node.path,
        Some(&delete.before),
    )?
    else {
        counts.unresolved += 1;
        return Ok(());
    };

    let prefix = keys::ordered_children_prefix(t, r, b, ws, &parent_id);
    let entries = parents.entries(db, &prefix)?;
    // Every label live for this child as of the version before the delete
    // (usually one; a merge's verbatim copy can leave two) that is STILL live
    // at the delete revision — i.e. the delete's tombstone is missing.
    let still_live = entries.live_labels(delete.node_id, &delete.deleted_at);
    for label in entries.live_labels(delete.node_id, &delete.before) {
        if !still_live.contains(&label) {
            continue;
        }
        let key = keys::ordered_child_key_versioned(
            t,
            r,
            b,
            ws,
            &parent_id,
            &label,
            &delete.deleted_at,
            delete.node_id,
        );
        writer.put(cf::ORDERED_CHILDREN, &key, keys::TOMBSTONE_VALUE)?;
        entries.record_tombstone(delete.node_id, &label, delete.deleted_at);
    }
    Ok(())
}
