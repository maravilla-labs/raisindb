//! "Does the target branch already hold this exact key?" — answered by a
//! merge-join, not a point lookup per copied entry.
//!
//! The copier writes `{tenant}\0{repo}\0{target}\0{rest}` for every source key
//! `{tenant}\0{repo}\0{source}\0{rest}`, in source order. Replacing one shared
//! prefix with another preserves order, so the keys it asks about ascend, and
//! one forward cursor over the target's keys answers every question in a
//! single sequential pass.
//!
//! Why the copier asks at all: an index key carries its REVISION, so a key the
//! target already holds is the target's own record of that revision — written
//! by the fork (byte-identical), by an earlier merge (byte-identical), or by an
//! in-place `versionable=false` rewrite on the target AFTER the fork, which is
//! the one case where the bytes differ and the target's are the truth. A merge
//! re-copies every source entry up to the source HEAD, pre-fork keys included;
//! overwriting the target's key there reverted that in-place write — the node
//! record read the old content again, and the old value's property entry came
//! back live beside the new one's.

use rocksdb::{ColumnFamily, DBRawIteratorWithThreadMode, ReadOptions, DB};
use std::cmp::Ordering;

pub(super) struct ExistingKeys<'a> {
    it: DBRawIteratorWithThreadMode<'a, DB>,
    /// Whether the target held ANY key of this CF when the copy began (a
    /// fork's fresh target holds none).
    pub(super) target_had_keys: bool,
}

impl<'a> ExistingKeys<'a> {
    /// A cursor over `cf` positioned at `target_prefix`. Total-order seek: a
    /// CF with a custom prefix extractor would otherwise consult its bloom
    /// filter for a prefix shorter than the extractor's domain.
    pub(super) fn new(db: &'a DB, cf: &'a ColumnFamily, target_prefix: &[u8]) -> Self {
        let mut opts = ReadOptions::default();
        opts.set_total_order_seek(true);
        let mut it = db.raw_iterator_cf_opt(cf, opts);
        it.seek(target_prefix);
        let target_had_keys = it.valid() && it.key().is_some_and(|k| k.starts_with(target_prefix));
        Self {
            it,
            target_had_keys,
        }
    }

    /// Whether the target holds exactly `key`. Callers ask in ascending order.
    ///
    /// The cursor's snapshot predates the copy's own writes, so what it sees
    /// is the target as it stood before this copy began.
    pub(super) fn contains(&mut self, key: &[u8]) -> bool {
        while self.it.valid() {
            match self.it.key().map(|k| k.cmp(key)) {
                Some(Ordering::Less) => self.it.next(),
                Some(Ordering::Equal) => return true,
                Some(Ordering::Greater) | None => return false,
            }
        }
        false
    }
}
