//! Where a revision-bounded point read gets its iterator.
//!
//! The path rule and the node decoder read three CFs (`NODES`, `NODE_PATH`,
//! `PATH_INDEX`) with the same "newest at or before R" seek. A single read
//! takes a fresh, prefix-bounded iterator per seek ([`DbRead`], what every
//! reader did). A batch ([`SnapshotRead`]) holds ONE iterator per CF, all
//! pinned to one RocksDB snapshot, and seeks it per item — so a level of
//! RESOLVE or a chunk of an index scan pays iterator and superversion setup
//! once per CF, not once per key, and every read of the batch sees one view.
//!
//! Both answer through [`VersionedRead`], so the decoder and the path rule
//! (`node_decode.rs`, `node_path.rs`) are ONE implementation for both.

use crate::keys;
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{DBRawIteratorWithThreadMode, ReadOptions, SnapshotWithThreadMode, DB};
use std::ops::ControlFlow;

/// "The newest entry under `prefix` at or before `max_revision`" over one CF.
pub(crate) trait VersionedRead {
    /// The database the reads go to (for the readers that still need it).
    fn db(&self) -> &DB;

    /// See [`super::newest_at_or_before_with`]; `cf` names the column family.
    fn newest_at_or_before_with<R>(
        &mut self,
        cf: &'static str,
        prefix: &[u8],
        max_revision: Option<&HLC>,
        read: impl FnOnce(HLC, &[u8]) -> R,
    ) -> Result<Option<R>>;

    /// Visit every entry under `prefix`, in key order from `seek` (a key at
    /// or after `prefix`), until `visit` breaks — the walk behind the
    /// grouped translation scan, the node lifeline and the localized-name
    /// claims, so a lookup that reads several of them reuses one iterator per
    /// CF too (plan Phase 13d).
    fn scan(
        &mut self,
        cf: &'static str,
        prefix: &[u8],
        seek: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> ControlFlow<()>,
    ) -> Result<()>;

    /// One key's value in `cf`, through the same view as every other read
    /// of this source — so a decision record (an index's build state, the
    /// repository config) and the rows it vouches for come from ONE view.
    fn get(&mut self, cf: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>>;

    /// The snapshot this source reads (`None`: the live database), for a
    /// reader that must open its own bounded iterator — the ordered-children
    /// scan — and still see the same view as the rest of the batch.
    fn snapshot(&self) -> Option<&SnapshotWithThreadMode<'_, DB>> {
        None
    }

    /// [`Self::newest_at_or_before_with`], copying the value out.
    fn newest_at_or_before(
        &mut self,
        cf: &'static str,
        prefix: &[u8],
        max_revision: Option<&HLC>,
    ) -> Result<Option<(HLC, Vec<u8>)>> {
        self.newest_at_or_before_with(cf, prefix, max_revision, |rev, value| (rev, value.to_vec()))
    }
}

/// A fresh prefix-bounded iterator per read, on the live database.
pub(crate) struct DbRead<'a>(pub(crate) &'a DB);

impl VersionedRead for DbRead<'_> {
    fn db(&self) -> &DB {
        self.0
    }

    fn newest_at_or_before_with<R>(
        &mut self,
        cf: &'static str,
        prefix: &[u8],
        max_revision: Option<&HLC>,
        read: impl FnOnce(HLC, &[u8]) -> R,
    ) -> Result<Option<R>> {
        let handle = crate::cf_handle(self.0, cf)?;
        super::newest_at_or_before_with(self.0, handle, prefix, max_revision, read)
    }

    fn scan(
        &mut self,
        cf: &'static str,
        prefix: &[u8],
        seek: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> ControlFlow<()>,
    ) -> Result<()> {
        let handle = crate::cf_handle(self.0, cf)?;
        let mut opts = ReadOptions::default();
        opts.set_total_order_seek(true);
        if let Some(upper) = crate::prefix_successor(prefix) {
            opts.set_iterate_upper_bound(upper);
        }
        let mut iter = self.0.raw_iterator_cf_opt(handle, opts);
        iter.seek(seek);
        scan_loop(&mut iter, prefix, visit)
    }

    fn get(&mut self, cf: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let handle = crate::cf_handle(self.0, cf)?;
        self.0
            .get_cf(handle, key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))
    }
}

/// The loop every [`VersionedRead::scan`] runs on its iterator.
fn scan_loop(
    iter: &mut DBRawIteratorWithThreadMode<'_, DB>,
    prefix: &[u8],
    visit: &mut dyn FnMut(&[u8], &[u8]) -> ControlFlow<()>,
) -> Result<()> {
    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        if !key.starts_with(prefix) || visit(key, value).is_break() {
            break;
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))
}

/// One iterator per CF, reused for every read of a batch — pinned to a
/// RocksDB snapshot ([`Self::new`]), or to the database as it stands when the
/// iterator is created ([`Self::live`]: an iterator without a snapshot reads
/// an implicit one taken at its creation).
///
/// The iterators carry no upper bound (they move between prefixes), so every
/// read stops at the first key outside its prefix. Total-order seek is on, as
/// for every other prefix read, so a CF's prefix extractor cannot hide keys.
pub(crate) struct SnapshotRead<'a> {
    db: &'a DB,
    snapshot: Option<&'a SnapshotWithThreadMode<'a, DB>>,
    iters: Vec<(&'static str, DBRawIteratorWithThreadMode<'a, DB>)>,
    seeks: u64,
}

impl<'a> SnapshotRead<'a> {
    pub(crate) fn new(db: &'a DB, snapshot: &'a SnapshotWithThreadMode<'a, DB>) -> Self {
        Self {
            db,
            snapshot: Some(snapshot),
            iters: Vec::with_capacity(3),
            seeks: 0,
        }
    }

    /// Iterators over the database as it stands now, not a held snapshot.
    pub(crate) fn live(db: &'a DB) -> Self {
        Self {
            db,
            snapshot: None,
            iters: Vec::with_capacity(3),
            seeks: 0,
        }
    }

    /// Seeks issued so far — what the RESOLVE perf assertion counts.
    pub(crate) fn seeks(&self) -> u64 {
        self.seeks
    }

    fn iter(&mut self, cf: &'static str) -> Result<&mut DBRawIteratorWithThreadMode<'a, DB>> {
        let at = match self.iters.iter().position(|(name, _)| *name == cf) {
            Some(at) => at,
            None => {
                let handle = crate::cf_handle(self.db, cf)?;
                let mut opts = ReadOptions::default();
                opts.set_total_order_seek(true);
                if let Some(snapshot) = self.snapshot {
                    opts.set_snapshot(snapshot);
                }
                let iter = self.db.raw_iterator_cf_opt(handle, opts);
                self.iters.push((cf, iter));
                self.iters.len() - 1
            }
        };
        Ok(&mut self.iters[at].1)
    }
}

impl VersionedRead for SnapshotRead<'_> {
    fn db(&self) -> &DB {
        self.db
    }

    fn newest_at_or_before_with<R>(
        &mut self,
        cf: &'static str,
        prefix: &[u8],
        max_revision: Option<&HLC>,
        read: impl FnOnce(HLC, &[u8]) -> R,
    ) -> Result<Option<R>> {
        self.seeks += 1;
        let iter = self.iter(cf)?;
        match max_revision {
            Some(max) => {
                let mut seek = Vec::with_capacity(prefix.len() + 16);
                seek.extend_from_slice(prefix);
                seek.extend_from_slice(&max.encode_descending());
                iter.seek(&seek);
            }
            None => iter.seek(prefix),
        }

        // The same loop as `newest_at_or_before_with`: an unparseable revision
        // is stepped over, never the end of the read.
        while iter.valid() {
            let Some(key) = iter.key() else {
                break;
            };
            if !key.starts_with(prefix) {
                break;
            }
            match keys::extract_revision_from_key(key) {
                Ok(revision) if max_revision.is_none_or(|max| &revision <= max) => {
                    return Ok(Some(read(revision, iter.value().unwrap_or_default())));
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    target: "rocksb::nodes::revision_lookup",
                    key_len = key.len(),
                    "Skipping versioned key with invalid revision: {}",
                    e
                ),
            }
            iter.next();
        }
        iter.status()
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        Ok(None)
    }

    fn scan(
        &mut self,
        cf: &'static str,
        prefix: &[u8],
        seek: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> ControlFlow<()>,
    ) -> Result<()> {
        self.seeks += 1;
        let iter = self.iter(cf)?;
        iter.seek(seek);
        scan_loop(iter, prefix, visit)
    }

    fn get(&mut self, cf: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let handle = crate::cf_handle(self.db, cf)?;
        match self.snapshot {
            Some(snapshot) => snapshot.get_cf(handle, key),
            None => self.db.get_cf(handle, key),
        }
        .map_err(|e| raisin_error::Error::storage(e.to_string()))
    }

    fn snapshot(&self) -> Option<&SnapshotWithThreadMode<'_, DB>> {
        self.snapshot
    }
}

/// A read source that remembers the newest revision it answered with — the
/// newest record a decode through it depended on. The batched reader compares
/// it against the live database's ([`super::node_record_revision_in`]).
pub(crate) struct Recorded<'s, S> {
    inner: &'s mut S,
    newest: Option<HLC>,
}

impl<'s, S: VersionedRead> Recorded<'s, S> {
    pub(crate) fn new(inner: &'s mut S) -> Self {
        Self {
            inner,
            newest: None,
        }
    }

    /// The newest revision any read through this source returned.
    pub(crate) fn newest(&self) -> Option<HLC> {
        self.newest
    }
}

impl<S: VersionedRead> VersionedRead for Recorded<'_, S> {
    fn db(&self) -> &DB {
        self.inner.db()
    }

    fn newest_at_or_before_with<R>(
        &mut self,
        cf: &'static str,
        prefix: &[u8],
        max_revision: Option<&HLC>,
        read: impl FnOnce(HLC, &[u8]) -> R,
    ) -> Result<Option<R>> {
        let newest = &mut self.newest;
        self.inner
            .newest_at_or_before_with(cf, prefix, max_revision, |revision, value| {
                *newest = (*newest).max(Some(revision));
                read(revision, value)
            })
    }

    fn scan(
        &mut self,
        cf: &'static str,
        prefix: &[u8],
        seek: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> ControlFlow<()>,
    ) -> Result<()> {
        self.inner.scan(cf, prefix, seek, visit)
    }

    fn get(&mut self, cf: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.inner.get(cf, key)
    }

    fn snapshot(&self) -> Option<&SnapshotWithThreadMode<'_, DB>> {
        self.inner.snapshot()
    }
}
