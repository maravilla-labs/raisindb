//! A node's delete tombstones and generations — the evidence the translation
//! read rule ends an overlay version with (`translation_read`), and what the
//! block-overlay materializations (`translation_write`) stage their `T` at.
//!
//! One walk over `{t}\0{r}\0{b}\0{ws}\0nodes\0{id}\0{~rev}`, newest first from
//! the upper bound down; nothing is decoded (a tombstone is recognised by its
//! value alone).

use crate::{cf, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use std::ops::ControlFlow;

/// What a node's `NODES` history says about the overlay versions stored at
/// or after `from`, read at `to` (`None`: HEAD): every record in
/// `[from, to]` plus the newest one below `from`, newest first.
///
/// An overlay version at `R` is ENDED at `to` ([`Self::ends`]) when
/// - the node has a delete tombstone in `[R, to]` — the delete ended it; or
/// - the node's newest record at or before `R` is a delete tombstone — it was
///   written into a DEAD generation (a peer that had not seen the delete, a
///   merge replaying a branch's translation onto a node the target deleted,
///   a write racing the delete). Such a version stays ended after a recreate
///   under the same id: the recreate starts a new generation above it.
///
/// Built once per node and bound, it decides every version of the node: a
/// listing or the resolver walks `NODES` once, not once per overlay.
pub(crate) struct NodeLifeline {
    /// `(revision, is_tombstone)`, newest first.
    records: Vec<(HLC, bool)>,
    /// Key and value bytes the walk read (a repair charges them to its
    /// throttle).
    pub(crate) bytes_read: u64,
}

impl NodeLifeline {
    /// Walk `node_id`'s records from `to` down to the newest one below
    /// `from`.
    pub(crate) fn read(
        db: &DB,
        scope: (&str, &str, &str, &str),
        node_id: &str,
        from: &HLC,
        to: Option<&HLC>,
    ) -> Result<Self> {
        Self::read_in(&mut super::DbRead(db), scope, node_id, from, to)
    }

    /// [`Self::read`] through a read source.
    pub(crate) fn read_in(
        src: &mut impl super::VersionedRead,
        scope: (&str, &str, &str, &str),
        node_id: &str,
        from: &HLC,
        to: Option<&HLC>,
    ) -> Result<Self> {
        let mut records = Vec::new();
        let bytes_read = walk_in(src, scope, node_id, to, |at, tomb| {
            records.push((at, tomb));
            if at < *from {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })?;
        Ok(Self {
            records,
            bytes_read,
        })
    }

    /// Whether an overlay version stored at `version` (at or after the
    /// `from` this was read with) is ended at the bound (see the type).
    pub(crate) fn ends(&self, version: &HLC) -> bool {
        for (at, tomb) in &self.records {
            if at > version {
                if *tomb {
                    return true; // a delete in (R, bound]
                }
                continue;
            }
            // The newest record at or before R decides: a delete at R, or a
            // dead generation.
            return *tomb;
        }
        false
    }
}

/// Every delete tombstone of `node_id` in `[from, to]`, newest first.
pub(crate) fn deletes_in_range(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    from: &HLC,
    to: Option<&HLC>,
) -> Result<Vec<HLC>> {
    Ok(deletes_in_range_counted(db, scope, node_id, from, to)?.0)
}

/// [`deletes_in_range`] and the bytes the walk read.
pub(crate) fn deletes_in_range_counted(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    from: &HLC,
    to: Option<&HLC>,
) -> Result<(Vec<HLC>, u64)> {
    let mut found = Vec::new();
    if to.is_some_and(|to| to < from) {
        return Ok((found, 0));
    }
    let bytes = walk(db, scope, node_id, to, |at, tomb| {
        if at < *from {
            return ControlFlow::Break(()); // newest first: everything after is older
        }
        if tomb {
            found.push(at);
        }
        ControlFlow::Continue(())
    })?;
    Ok((found, bytes))
}

#[cfg(test)]
thread_local! {
    /// `NODES` walks on this thread: the batched readers' tests count them.
    pub(crate) static WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The one walk: `node_id`'s `NODES` records at or below `to`, newest first,
/// as `(revision, is_tombstone)`, until `visit` breaks. Returns the key and
/// value bytes read.
fn walk(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    to: Option<&HLC>,
    visit: impl FnMut(HLC, bool) -> ControlFlow<()>,
) -> Result<u64> {
    walk_in(&mut super::DbRead(db), scope, node_id, to, visit)
}

/// [`walk`] through a read source (a lookup's pinned iterators).
fn walk_in(
    src: &mut impl super::VersionedRead,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    to: Option<&HLC>,
    mut visit: impl FnMut(HLC, bool) -> ControlFlow<()>,
) -> Result<u64> {
    #[cfg(test)]
    WALKS.with(|walks| walks.set(walks.get() + 1));
    let (tenant_id, repo_id, branch, workspace) = scope;
    let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let seek = match to {
        Some(to) => {
            let mut seek = prefix.clone();
            seek.extend_from_slice(&to.encode_descending());
            seek
        }
        None => prefix.clone(),
    };
    let mut bytes = 0u64;
    src.scan(cf::NODES, &prefix, &seek, &mut |key, value| {
        bytes += (key.len() + value.len()) as u64;
        // Anything under the prefix that is not `{prefix}{16-byte revision}`
        // is skipped, never the end of the read.
        if key.len() == prefix.len() + 16 {
            if let Ok(at) = keys::extract_revision_from_key(key) {
                if to.is_none_or(|to| at <= *to) {
                    return visit(at, keys::is_tombstone_value(value));
                }
            }
        }
        ControlFlow::Continue(())
    })?;
    Ok(bytes)
}
