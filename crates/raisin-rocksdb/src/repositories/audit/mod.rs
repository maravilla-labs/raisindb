//! RocksDB implementation of audit-log storage.
//!
//! Replaces the process-local `InMemoryAuditRepo` for the RocksDB backend, so a
//! node's audit history survives a restart.
//!
//! # Key Format
//!
//! - Entry: `{tenant}\0{repo}\0{branch}\0{workspace}\0audit\0{node_id}\0{~ts_ms}{~seq}`
//!
//! See `keys::audit_keys` for why the trailing blob is inverted and fixed-width.
//! There is no path index: both HTTP read routes and the WS query resolve a path
//! to a node through the RLS-aware `NodeService` *before* touching audit, so the
//! only lookup this store must serve is by node id — and keying by path would be
//! actively wrong, since a MOVE/RENAME changes the path while the history must
//! stay attached to the node.
//!
//! # Replication
//!
//! Audit writes are deliberately NOT captured to the OpLog. Every entry is
//! derived from a committed `NodeEvent`, and a follower applying the replicated
//! *node* operation emits that event locally, so its own `AuditEventHandler`
//! reproduces the entry. Capturing here would double-write on followers and
//! would require a new `OpType` variant — a persisted enum.

mod crud;
#[cfg(test)]
mod tests;

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use crate::cf;
use raisin_error::Result;
use rocksdb::DB;

/// Hard cap on entries returned by a single unbounded read.
///
/// The in-memory store was implicitly bounded by "whatever happened since the
/// last restart". A persistent store is not, and the read APIs (HTTP
/// `/api/audit/...`, the `audit_log` node command, the WS `audit_query`) all
/// serialize the whole result into one response. Without a cap a long-lived
/// node's history would grow into an unbounded payload.
pub const DEFAULT_AUDIT_READ_LIMIT: usize = 1000;

/// RocksDB-backed audit-log repository.
///
/// Cloneable and cheap to construct — it holds only the shared `DB` handle and a
/// counter, mirroring `TagRepositoryImpl`.
#[derive(Clone)]
pub struct RocksDBAuditRepo {
    pub(super) db: Arc<DB>,
    /// Per-process tiebreaker for entries written in the same millisecond.
    ///
    /// PROCESS-WIDE, not per-instance: `RocksDBStorage::audit_repository()`
    /// mints a fresh repo on every call and there are already two call sites, so
    /// a per-instance counter would let two repos produce an identical key in
    /// the same millisecond and silently drop one entry — the very tie this
    /// exists to break. See `keys::audit_keys`.
    pub(super) seq: Arc<AtomicU64>,
}

/// The one counter every repo in this process shares.
static AUDIT_SEQ: std::sync::OnceLock<Arc<AtomicU64>> = std::sync::OnceLock::new();

fn process_audit_seq() -> Arc<AtomicU64> {
    AUDIT_SEQ
        .get_or_init(|| Arc::new(AtomicU64::new(0)))
        .clone()
}

impl RocksDBAuditRepo {
    /// Create a new audit repository over an open database handle.
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            seq: process_audit_seq(),
        }
    }

    /// Get the column family handle for audit logs.
    pub(super) fn cf_audit(&self) -> Result<&rocksdb::ColumnFamily> {
        self.db.cf_handle(cf::AUDIT_LOG).ok_or_else(|| {
            raisin_error::Error::storage(format!("Column family '{}' not found", cf::AUDIT_LOG))
        })
    }
}
