//! The run detector: given a CF's keys in order, which versions are redundant.
//!
//! Keys of one group sort newest first (the revision is descending), so a
//! single forward pass sees each version right after the newer one. The
//! detector remembers, per group, only the NEWER version just seen — grouped
//! by chunk (the key up to the revision) exactly as [`super::super::layout`]
//! groups them for retention GC.
//!
//! **Memory is bounded.** A chunk's groups interleave by revision, so none can
//! be released before the chunk ends; a hot PROPERTY_INDEX value shared by
//! millions of nodes would otherwise hold one entry per node. Past
//! [`MAX_REMEMBERED_GROUPS`] per chunk the least recently seen group is
//! forgotten (`forgotten_groups`). Forgetting only costs a deletion, never
//! correctness, and it depends only on the key sequence, so a resumed run
//! decides exactly what an uninterrupted one does.
//!
//! **Editorial order is an answer too.** ORDERED_CHILDREN readers order the
//! children sharing a label (merges copy labels verbatim) by their visible
//! entry's position in the chunk — `(~rev, child)` key order
//! (`ordering/paged.rs`, `paged_desc.rs`). Deleting a child's newer twin moves
//! that child's visible entry to the older twin, which reorders it past every
//! other child's entry lying between the two in key order. So there a newer
//! twin is deleted only when the older one is its IMMEDIATE successor in the
//! chunk (`kept_interleaved` otherwise); then no other child can cross it, at
//! any read revision, whatever else is deleted.

use super::super::layout::{self, GcTarget};
use super::CfCollapseCounts;
use crate::{cf, keys};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use std::collections::{BTreeMap, HashMap};

/// Groups remembered per open chunk before the least recently seen is
/// forgotten (~40 MB at most for one chunk).
pub(super) const MAX_REMEMBERED_GROUPS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EntryState {
    Tombstone,
    Live(Vec<u8>),
}

impl EntryState {
    pub(super) fn of(value: &[u8]) -> Self {
        if keys::is_tombstone_value(value) {
            Self::Tombstone
        } else {
            Self::Live(value.to_vec())
        }
    }
}

/// The newer version of a group, remembered until the next-older one shows.
struct Newer {
    key: Vec<u8>,
    rev: HLC,
    state: EntryState,
    size: u64,
    /// Position of this key in its chunk.
    seq: u64,
    /// The slice that read it.
    slice: u64,
}

/// A chunk (key bytes before the revision) currently being walked.
struct OpenChunk {
    prefix: Vec<u8>,
    groups: HashMap<Vec<u8>, Newer>,
    /// `seq -> tail` of every remembered group: least recently seen first.
    lru: BTreeMap<u64, Vec<u8>>,
    next_seq: u64,
}

pub(super) struct Detector<'t> {
    target: &'t GcTarget,
    watermark: HLC,
    open: Vec<OpenChunk>,
    slice: u64,
    max_groups: usize,
    /// Readers order a chunk's groups by key position (ORDERED_CHILDREN).
    order_sensitive: bool,
}

/// A version the detector decided to delete.
pub(super) struct Doomed {
    pub key: Vec<u8>,
    pub size: u64,
    /// The state it held when read.
    pub state: EntryState,
    /// Read by an EARLIER slice: something may have rewritten it since (an
    /// in-place write at its revision), so re-read it before deleting.
    pub stale: bool,
}

impl<'t> Detector<'t> {
    pub(super) fn new(target: &'t GcTarget, watermark: HLC) -> Self {
        Self {
            target,
            watermark,
            open: Vec::new(),
            slice: 0,
            max_groups: MAX_REMEMBERED_GROUPS,
            order_sensitive: target.cf == cf::ORDERED_CHILDREN,
        }
    }

    /// [`Self::new`] with a smaller memory cap (tests).
    #[cfg(test)]
    pub(super) fn with_max_groups(mut self, max_groups: usize) -> Self {
        self.max_groups = max_groups;
        self
    }

    /// Forget every remembered version (an inserter may have run).
    pub(super) fn forget(&mut self) {
        self.open.clear();
    }

    /// A new slice starts: everything remembered so far was read by an
    /// earlier one.
    pub(super) fn begin_slice(&mut self) {
        self.slice += 1;
    }

    /// Look at the next key in CF order. Returns the NEWER version this key
    /// made redundant, if it is strictly below the watermark.
    pub(super) fn visit(
        &mut self,
        key: &[u8],
        value: &[u8],
        counts: &mut CfCollapseCounts,
    ) -> Result<Option<Doomed>> {
        while let Some(top) = self.open.last() {
            let inside = key.len() > top.prefix.len()
                && key.starts_with(&top.prefix)
                && key[top.prefix.len()] == 0;
            if inside {
                break;
            }
            self.open.pop();
        }
        let Some(loc) = layout::locate(self.target, key) else {
            return Ok(None);
        };
        counts.scanned += 1;
        let rev =
            HLC::decode_descending(loc.revision(key)).map_err(|e| Error::storage(e.to_string()))?;
        let chunk_prefix = loc.chunk(key);
        let idx = match self.open.iter().rposition(|c| c.prefix == chunk_prefix) {
            Some(i) => i,
            None => {
                self.open.push(OpenChunk {
                    prefix: chunk_prefix.to_vec(),
                    groups: HashMap::new(),
                    lru: BTreeMap::new(),
                    next_seq: 0,
                });
                self.open.len() - 1
            }
        };
        let chunk = &mut self.open[idx];
        let seq = chunk.next_seq;
        chunk.next_seq += 1;
        let state = EntryState::of(value);
        let tail = loc.tail(key);
        let mut doomed = None;
        if let Some(newer) = chunk.groups.get(tail) {
            if newer.state == state {
                if newer.rev >= self.watermark {
                    counts.kept_above_watermark += 1;
                } else if self.order_sensitive && newer.seq + 1 != seq {
                    counts.kept_interleaved += 1;
                } else {
                    doomed = Some(Doomed {
                        key: newer.key.clone(),
                        size: newer.size,
                        state: newer.state.clone(),
                        stale: newer.slice != self.slice,
                    });
                }
            }
            chunk.lru.remove(&newer.seq);
        } else if chunk.groups.len() >= self.max_groups {
            if let Some((_, evicted)) = chunk.lru.pop_first() {
                chunk.groups.remove(&evicted);
                counts.forgotten_groups += 1;
            }
        }
        chunk.lru.insert(seq, tail.to_vec());
        chunk.groups.insert(
            tail.to_vec(),
            Newer {
                key: key.to_vec(),
                rev,
                state,
                size: (key.len() + value.len()) as u64,
                seq,
                slice: self.slice,
            },
        );
        Ok(doomed)
    }
}
