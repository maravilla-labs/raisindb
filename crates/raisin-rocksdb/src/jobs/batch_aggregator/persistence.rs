//! Durable buffer for pending batch aggregator operations.
//!
//! Single-op fulltext edits sit in the aggregator's in-memory pending map
//! until the idle-flush window fires (~2 s). Without persistence, any
//! crash/SIGKILL/host reboot in that window silently drops those edits.
//! This module mirrors every `queue()` into a RocksDB column family so
//! restart can replay them.
//!
//! # Ordering invariant
//!
//! `delete_pending_batch` runs **after** the dispatched job's Tantivy
//! `commit()` returns Ok. The reverse order would be a correctness bug
//! (crash after delete but before commit = silent loss). With this
//! ordering, the worst case is duplicate replay, which is idempotent
//! on Tantivy (`delete_term + add_document`).
//!
//! # Key encoding
//!
//! `{tenant}\0{repo}\0{branch}\0{queued_at_nanos:16 BE}\0{uuid_v4:16}`
//!
//! - Per-tenant/repo/branch prefix matches the in-memory `IndexKey`
//!   grouping used by the aggregator, enabling targeted prefix scans.
//! - Big-endian nanosecond timestamp gives chronological iteration
//!   within a key, matching the in-memory FIFO semantics.
//! - UUID v4 suffix guarantees uniqueness even when two operations
//!   share a nanosecond (tight loops on coarse clocks).

use crate::{cf, cf_handle};
use raisin_error::{Error, Result};
use raisin_storage::jobs::{IndexOperation, JobContext};
use rocksdb::{IteratorMode, WriteBatch, DB};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// One pending operation, persisted to RocksDB on `queue()` and removed
/// after a successful flush (post-Tantivy-commit).
///
/// The in-memory `PendingOperation` uses `Instant` which isn't
/// serializable; the persistent record uses wall-clock nanos so it's
/// meaningful across process restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PendingOpRecord {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
    pub node_id: String,
    pub operation: IndexOperation,
    pub context: JobContext,
    /// Wall-clock nanoseconds since UNIX epoch when the op was queued.
    pub queued_at_nanos: u128,
}

/// RocksDB key for a pending op. Returned by `put_pending` and held by
/// the in-memory PendingOperation so `flush()` can delete the right rows.
pub(super) type PendingOpKey = Vec<u8>;

/// Build the RocksDB key for a new pending op record. Uses wall-clock
/// time (not `Instant`) because the key persists across restarts.
pub(super) fn make_pending_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    queued_at_nanos: u128,
) -> PendingOpKey {
    let uuid = Uuid::new_v4();
    let mut key =
        Vec::with_capacity(tenant_id.len() + repo_id.len() + branch.len() + 3 + 16 + 1 + 16);
    key.extend_from_slice(tenant_id.as_bytes());
    key.push(0);
    key.extend_from_slice(repo_id.as_bytes());
    key.push(0);
    key.extend_from_slice(branch.as_bytes());
    key.push(0);
    key.extend_from_slice(&queued_at_nanos.to_be_bytes());
    key.push(0);
    key.extend_from_slice(uuid.as_bytes());
    key
}

/// Current wall-clock time as nanoseconds since UNIX epoch. Used both
/// for `queued_at_nanos` in the record and for key sorting.
pub(super) fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Persist one pending op. Called inline from `BatchIndexAggregator::queue()`
/// while the in-memory `pending` write-lock is held, so crash recovery
/// state always matches what the in-memory map saw.
pub(super) fn put_pending(db: &DB, key: &[u8], record: &PendingOpRecord) -> Result<()> {
    let cf = cf_handle(db, cf::PENDING_BATCH_OPS)?;
    let value =
        serde_json::to_vec(record).map_err(|e| Error::storage(format!("serialize: {}", e)))?;
    db.put_cf(cf, key, value)
        .map_err(|e| Error::storage(format!("put pending: {}", e)))
}

/// Atomically delete a batch of pending ops. Called from
/// `BatchIndexAggregator::flush()` AFTER the dispatched job's Tantivy
/// commit succeeds — never before. Single WriteBatch so the deletion is
/// atomic for one flush; partial-failure means restart replays the
/// whole batch (Tantivy ops are idempotent, so this only wastes I/O).
pub(super) fn delete_pending_batch(db: &DB, keys: &[PendingOpKey]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let cf = cf_handle(db, cf::PENDING_BATCH_OPS)?;
    let mut batch = WriteBatch::default();
    for key in keys {
        batch.delete_cf(cf, key);
    }
    db.write(batch)
        .map_err(|e| Error::storage(format!("delete pending batch: {}", e)))
}

/// Scan all persisted pending ops. Called once at startup from
/// `BatchIndexAggregator::replay_pending()` to refill the in-memory map.
/// Full CF iteration is acceptable here because (a) it's startup-only
/// and (b) the CF is bounded by the count of un-flushed ops since the
/// last clean shutdown — typically zero, at worst a few seconds' worth.
pub(super) fn scan_pending(db: &DB) -> Result<Vec<(PendingOpKey, PendingOpRecord)>> {
    let cf = cf_handle(db, cf::PENDING_BATCH_OPS)?;
    let mut out = Vec::new();
    for item in db.iterator_cf(cf, IteratorMode::Start) {
        let (key, value) = item.map_err(|e| Error::storage(format!("scan pending: {}", e)))?;
        let record: PendingOpRecord = serde_json::from_slice(&value)
            .map_err(|e| Error::storage(format!("deserialize pending: {}", e)))?;
        out.push((key.to_vec(), record));
    }
    Ok(out)
}

/// Holder for the persistence side of the aggregator. Bundles the DB
/// handle so `BatchIndexAggregator` can stay focused on flush logic
/// without leaking `Arc<DB>` into its public surface.
#[derive(Clone)]
pub(super) struct PendingPersistence {
    db: Arc<DB>,
}

impl PendingPersistence {
    pub(super) fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    pub(super) fn put(&self, key: &[u8], record: &PendingOpRecord) -> Result<()> {
        put_pending(&self.db, key, record)
    }

    pub(super) fn delete_batch(&self, keys: &[PendingOpKey]) -> Result<()> {
        delete_pending_batch(&self.db, keys)
    }

    pub(super) fn scan(&self) -> Result<Vec<(PendingOpKey, PendingOpRecord)>> {
        scan_pending(&self.db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_hlc::HLC;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn temp_db() -> (TempDir, Arc<DB>) {
        let dir = TempDir::new().unwrap();
        let config = crate::config::RocksDBConfig::development().with_path(dir.path());
        let db = crate::open_db_with_config(&config).unwrap();
        (dir, Arc::new(db))
    }

    fn sample_record(node_id: &str) -> PendingOpRecord {
        PendingOpRecord {
            tenant_id: "t".into(),
            repo_id: "r".into(),
            branch: "main".into(),
            node_id: node_id.into(),
            operation: IndexOperation::AddOrUpdate,
            context: JobContext {
                tenant_id: "t".into(),
                repo_id: "r".into(),
                branch: "main".into(),
                workspace_id: "staff".into(),
                revision: HLC::new(1, 0),
                metadata: HashMap::new(),
            },
            queued_at_nanos: now_nanos(),
        }
    }

    #[test]
    fn put_scan_roundtrip() {
        let (_dir, db) = temp_db();
        let persist = PendingPersistence::new(db);

        let rec = sample_record("node-1");
        let key = make_pending_key(
            &rec.tenant_id,
            &rec.repo_id,
            &rec.branch,
            rec.queued_at_nanos,
        );
        persist.put(&key, &rec).unwrap();

        let scanned = persist.scan().unwrap();
        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].1.node_id, "node-1");
    }

    #[test]
    fn delete_batch_removes_keys() {
        let (_dir, db) = temp_db();
        let persist = PendingPersistence::new(db);

        let mut keys = Vec::new();
        for i in 0..5 {
            let rec = sample_record(&format!("node-{}", i));
            let k = make_pending_key(
                &rec.tenant_id,
                &rec.repo_id,
                &rec.branch,
                rec.queued_at_nanos,
            );
            persist.put(&k, &rec).unwrap();
            keys.push(k);
        }
        assert_eq!(persist.scan().unwrap().len(), 5);

        persist.delete_batch(&keys[..3]).unwrap();
        assert_eq!(persist.scan().unwrap().len(), 2);
    }

    #[test]
    fn empty_delete_is_noop() {
        let (_dir, db) = temp_db();
        let persist = PendingPersistence::new(db);
        persist.delete_batch(&[]).unwrap();
    }
}
