//! The RocksDB view behind a statement's [`ReadSnapshot`].
//!
//! A statement that reads in several batches (RESOLVE's levels, an index
//! scan's chunks) must see ONE state of the database. The HLC bound every read
//! takes is not enough: a `versionable=false` write overwrites its node IN
//! PLACE at a revision at or below the bound, so without a pinned view one
//! chunk sees the old record and the next the new one. The statement opens
//! this view once, at its first batched read (after its revision was fixed),
//! and every batch reads through it. The view decides a record's CONTENT; a
//! record the live database holds at a newer revision still wins (see
//! `batch_get.rs`), so the view can never hide a commit the statement's live
//! readers already saw.
//!
//! A held snapshot pins memtables and SST files for as long as it lives. It
//! lives exactly as long as the statement's context (RESOLVE's budget bounds
//! the statements that read the most), and is released with the last clone.

use raisin_storage::ReadSnapshot;
use rocksdb::{SnapshotWithThreadMode, DB};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A RocksDB snapshot that owns the database handle it was taken from.
pub(crate) struct RocksReadSnapshot {
    // Declared BEFORE `db`: fields drop in order, so the snapshot is released
    // while the database is still open.
    snapshot: SnapshotWithThreadMode<'static, DB>,
    db: Arc<DB>,
    /// Iterator seeks every batched read through this view has issued — what
    /// the RESOLVE perf assertion counts ([`read_snapshot_seeks`]). The reads
    /// run on blocking threads, where a thread-local `PerfContext` cannot see
    /// them; the view is per statement, so the count is too.
    seeks: AtomicU64,
    /// The database's last sequence number, read just BEFORE the snapshot was
    /// taken: while it is still the latest, nothing was written since the pin
    /// and the view IS the live database (see [`Self::moved_since_pin`]).
    pinned_sequence: u64,
}

impl RocksReadSnapshot {
    /// Pin the current state of `db`.
    pub(crate) fn open(db: &Arc<DB>) -> Self {
        let db = db.clone();
        // Before the snapshot: a write in between makes the view look behind
        // when it is not (one wasted probe), never the reverse.
        let pinned_sequence = db.latest_sequence_number();
        let snapshot = db.snapshot();
        // SAFETY: the snapshot borrows the `DB` inside `db`'s heap allocation,
        // which never moves and outlives the snapshot: this struct owns a
        // clone of the `Arc` and drops the snapshot first (field order above).
        // The `'static` lifetime never escapes — `snapshot()` hands it out
        // re-borrowed for the lifetime of `&self`.
        let snapshot = unsafe {
            std::mem::transmute::<SnapshotWithThreadMode<'_, DB>, SnapshotWithThreadMode<'static, DB>>(
                snapshot,
            )
        };
        Self {
            snapshot,
            db,
            seeks: AtomicU64::new(0),
            pinned_sequence,
        }
    }

    /// Whether anything was written to the database since the view was
    /// pinned — only then can the live database hold a record the view lacks,
    /// and only then does a batched read pay for the live probe.
    pub(crate) fn moved_since_pin(&self) -> bool {
        self.db.latest_sequence_number() != self.pinned_sequence
    }

    /// Wrap a freshly pinned view of `db` as the storage-level handle.
    pub(crate) fn open_handle(db: &Arc<DB>) -> ReadSnapshot {
        ReadSnapshot::new(Self::open(db))
    }

    /// The pinned view, borrowed no longer than `self`.
    pub(crate) fn snapshot(&self) -> &SnapshotWithThreadMode<'_, DB> {
        &self.snapshot
    }

    /// Whether this view was taken of `db` (a snapshot of another database
    /// must never be handed to its reads).
    pub(crate) fn is_of(&self, db: &Arc<DB>) -> bool {
        Arc::ptr_eq(&self.db, db)
    }

    /// Account the seeks one batch issued through this view.
    pub(crate) fn add_seeks(&self, seeks: u64) {
        self.seeks.fetch_add(seeks, Ordering::Relaxed);
    }
}

/// The iterator seeks every batched read through `snapshot` has issued so far
/// — `None` when it is not a RocksDB view. For tests and diagnostics: the
/// reads run on blocking threads, out of reach of a thread-local
/// `PerfContext`, and this count is scoped to one statement's view.
#[doc(hidden)]
pub fn read_snapshot_seeks(snapshot: &ReadSnapshot) -> Option<u64> {
    snapshot
        .downcast::<RocksReadSnapshot>()
        .map(|view| view.seeks.load(Ordering::Relaxed))
}
