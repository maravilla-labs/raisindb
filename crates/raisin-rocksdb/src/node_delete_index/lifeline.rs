//! The node lifeline read from `NODE_DELETES` instead of a `NODES` walk.
//!
//! # The rule (as the walk implements it)
//!
//! Let `Rec` be the node's `NODES` records the walk sees (`{prefix}{16-byte
//! revision}` keys at or below the bound `B`) and `T ⊆ Rec` the tombstones.
//! A version at `R` (`R >= from`) is ended iff
//!
//! - (a) some `t ∈ T` has `R < t <= B`, or
//! - (b) the newest record `<= R` exists and is in `T`.
//!
//! # The indexed answer
//!
//! `D` = the node's entries `<= B`, newest first; `confirm(d)` = "the newest
//! `NODES` record `<= d` is AT `d` and a tombstone", i.e. `d ∈ T` (one seek).
//!
//! ```text
//! for d in D (newest first):
//!     if d > R:  if confirm(d) { return ENDED }      // (a)
//!     else:      if !confirm(d) { continue }         // stale entry
//!                if d == R { return ENDED }           // (b): the record at R is t
//!                return newest record <= R is a tombstone   // (b), one seek
//! return NOT ENDED
//! ```
//!
//! # Why they agree
//!
//! Given **completeness**, `T ⊆ D` (the branch is `Ready`; `mod.rs`). Note
//! `D` may hold stale entries (`D ⊄ T`); `confirm` is exactly membership in
//! `T`.
//!
//! - (a) holds ⇒ some `t ∈ T ∩ (R, B]`; `t ∈ D`, and the loop visits every
//!   entry in `(R, B]` before any entry `<= R`, confirming each — it returns
//!   ENDED at the first member of `T`. Conversely it returns ENDED in that
//!   branch only for a confirmed `d ∈ T ∩ (R, B]`. So the `d > R` branch
//!   returns ENDED iff (a).
//! - (a) fails ⇒ the loop reaches the entries `<= R` and stops at the first
//!   confirmed one, `d0` = the newest `t ∈ T` with `t <= R` (every such `t`
//!   is in `D`). If there is none, no tombstone is `<= R`, so the newest
//!   record `<= R` is live or absent and (b) fails — NOT ENDED is right. If
//!   `d0 == R`, the record at `R` is that tombstone: (b). Otherwise the
//!   answer is computed by (b)'s own definition.
//!
//! Restores and re-creates need no entry: they are live records, and only
//! ever make (b) false, which the `NODES` seek in (b) sees. Stale entries
//! (history GC dropping a tombstone, a record overwritten in place) only cost
//! their `confirm` seek.
//!
//! What is read: the entries in `[from, B]` and below `from` down to the
//! first CONFIRMED one (enough for every `R >= from`), from one seek; a
//! `confirm` seek per entry the answer reaches (memoized); one `NODES` seek
//! for (b) when the node was deleted at or below `R` and not at `R`. A node
//! never deleted: one seek, nothing else.

use crate::cf;
use crate::mvcc_read::{record_at_or_before_in, VersionedRead};
use raisin_error::Result;
use raisin_hlc::HLC;
use std::cell::RefCell;
use std::ops::ControlFlow;

/// See the module docs.
pub(crate) struct IndexedLifeline {
    /// The node's `NODES` prefix, for the confirming seeks.
    nodes_prefix: Vec<u8>,
    /// Entries `<= to`, newest first: all of `[from, to]`, then those below
    /// `from` down to (and including) the first confirmed one.
    deletes: Vec<HLC>,
    /// `confirm` per entry, once asked.
    confirmed: RefCell<Vec<Option<bool>>>,
}

impl IndexedLifeline {
    pub(crate) fn read_in(
        src: &mut impl VersionedRead,
        scope: (&str, &str, &str, &str),
        node_id: &str,
        from: &HLC,
        to: Option<&HLC>,
    ) -> Result<Self> {
        let (tenant_id, repo_id, branch, workspace) = scope;
        let prefix = super::node_prefix(tenant_id, repo_id, branch, workspace, node_id);
        let mut this = Self {
            nodes_prefix: crate::keys::node_key_prefix(
                tenant_id, repo_id, branch, workspace, node_id,
            ),
            deletes: Vec::new(),
            confirmed: RefCell::new(Vec::new()),
        };
        // Phase 1: everything in [from, to] and the first entry below `from`.
        let mut seek = prefix.clone();
        if let Some(to) = to {
            seek.extend_from_slice(&to.encode_descending());
        }
        let mut below = scan_entries(src, &prefix, &seek, to, from, &mut this.deletes)?;
        // Phase 2: an entry below `from` that does not confirm is stale; the
        // one after it may be the delete (b) needs. Rare: GC'd tombstones.
        while let Some(at) = below {
            let index = this.deletes.len() - 1;
            if this.confirm(src, index)? {
                break;
            }
            let mut next = prefix.clone();
            next.extend_from_slice(&at.encode_descending());
            next.push(0); // strictly after `at`'s key
            below = scan_entries(src, &prefix, &next, to, &at, &mut this.deletes)?;
        }
        Ok(this)
    }

    /// Whether the version at `version` is ended (see the module docs).
    pub(crate) fn ends_in(&self, src: &mut impl VersionedRead, version: &HLC) -> Result<bool> {
        for (index, at) in self.deletes.iter().enumerate() {
            if at > version {
                if self.confirm(src, index)? {
                    return Ok(true);
                }
                continue;
            }
            if !self.confirm(src, index)? {
                continue;
            }
            if at == version {
                return Ok(true);
            }
            return Ok(record_at_or_before_in(src, &self.nodes_prefix, version)?
                .is_some_and(|(_, tomb)| tomb));
        }
        Ok(false)
    }

    /// Whether entry `index` names a tombstone `NODES` still holds.
    fn confirm(&self, src: &mut impl VersionedRead, index: usize) -> Result<bool> {
        if let Some(Some(known)) = self.confirmed.borrow().get(index) {
            return Ok(*known);
        }
        let at = self.deletes[index];
        let confirmed = record_at_or_before_in(src, &self.nodes_prefix, &at)?
            .is_some_and(|(rev, tomb)| rev == at && tomb);
        let mut memo = self.confirmed.borrow_mut();
        if memo.len() <= index {
            memo.resize(index + 1, None);
        }
        memo[index] = Some(confirmed);
        Ok(confirmed)
    }
}

/// Append the entries from `seek` (newest first, `<= to`) to `out` until the
/// first one strictly below `stop_below`, which is appended too and
/// returned.
fn scan_entries(
    src: &mut impl VersionedRead,
    prefix: &[u8],
    seek: &[u8],
    to: Option<&HLC>,
    stop_below: &HLC,
    out: &mut Vec<HLC>,
) -> Result<Option<HLC>> {
    let mut below = None;
    src.scan(cf::NODE_DELETES, prefix, seek, &mut |key, _| {
        if key.len() != prefix.len() + 16 {
            return ControlFlow::Continue(());
        }
        let Ok(at) = crate::keys::extract_revision_from_key(key) else {
            return ControlFlow::Continue(());
        };
        if to.is_some_and(|to| at > *to) {
            return ControlFlow::Continue(());
        }
        out.push(at);
        if at < *stop_below {
            below = Some(at);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    })?;
    Ok(below)
}
