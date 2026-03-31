//! Persistent Idempotency Tracker
//!
//! This module provides persistent storage for applied operation IDs to ensure
//! idempotency across node restarts. Without persistent tracking, operations could
//! be reapplied after a restart, causing duplicate effects.
//!
//! ## Why This Is Critical
//!
//! 1. **Crash Recovery**: After a node crash/restart, the in-memory set of applied
//!    operations is lost. Without persistence, catch-up protocol could re-deliver
//!    operations that were already applied before the crash.
//!
//! 2. **Catch-Up Protocol**: When syncing from peers, we need to know which operations
//!    we've already applied to avoid duplicates.
//!
//! 3. **True Idempotency**: Even though operations should be designed to be idempotent,
//!    storing applied IDs provides a safety net and improves performance.

use hashbrown::HashSet;
use raisin_replication::metrics::{AtomicCounter, DurationHistogram, IdempotencyMetrics};
use rocksdb::{ColumnFamily, DB};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

/// Key prefix for applied operation tracking
/// Format: applied_ops/{op_id_bytes}
const APPLIED_OPS_PREFIX: &[u8] = b"applied_ops/";

/// Persistent idempotency tracker using RocksDB
///
/// This tracks which operation IDs have been applied to prevent duplicate application.
/// The state is persisted to RocksDB so it survives node restarts.
pub struct PersistentIdempotencyTracker {
    db: Arc<DB>,
    cf_name: String,
    metrics: IdempotencyTrackerMetrics,
}

/// Metrics for persistent idempotency tracker
#[derive(Debug, Clone)]
struct IdempotencyTrackerMetrics {
    checks_total: AtomicCounter,
    hits_total: AtomicCounter,
    misses_total: AtomicCounter,
    check_duration: DurationHistogram,
    mark_duration: DurationHistogram,
    batch_sizes: DurationHistogram,
}

impl PersistentIdempotencyTracker {
    /// Create a new persistent idempotency tracker
    ///
    /// # Arguments
    /// * `db` - RocksDB instance
    /// * `cf_name` - Column family name (typically "applied_ops")
    pub fn new(db: Arc<DB>, cf_name: String) -> Self {
        Self {
            db,
            cf_name,
            metrics: IdempotencyTrackerMetrics {
                checks_total: AtomicCounter::new(),
                hits_total: AtomicCounter::new(),
                misses_total: AtomicCounter::new(),
                check_duration: DurationHistogram::new(1000),
                mark_duration: DurationHistogram::new(1000),
                batch_sizes: DurationHistogram::new(500),
            },
        }
    }

    /// Check if an operation has been applied
    ///
    /// # Arguments
    /// * `op_id` - The operation ID to check
    ///
    /// # Returns
    /// True if the operation has been applied, false otherwise
    pub fn is_applied(&self, op_id: &Uuid) -> Result<bool, String> {
        let start = Instant::now();
        self.metrics.checks_total.increment();

        let cf = self.get_cf()?;
        let key = self.make_key(op_id);

        let result = match self.db.get_cf(&cf, &key) {
            Ok(Some(_)) => {
                self.metrics.hits_total.increment();
                Ok(true)
            }
            Ok(None) => {
                self.metrics.misses_total.increment();
                Ok(false)
            }
            Err(e) => Err(format!("Failed to check applied status: {}", e)),
        };

        self.metrics.check_duration.record(start.elapsed());
        result
    }

    /// Mark an operation as applied
    ///
    /// # Arguments
    /// * `op_id` - The operation ID to mark as applied
    /// * `timestamp_ms` - Timestamp when the operation was applied (for TTL/GC)
    pub fn mark_applied(&self, op_id: &Uuid, timestamp_ms: u64) -> Result<(), String> {
        let start = Instant::now();

        let cf = self.get_cf()?;
        let key = self.make_key(op_id);

        // Store the timestamp as the value (8 bytes, big-endian)
        let value = timestamp_ms.to_be_bytes();

        let result = self
            .db
            .put_cf(&cf, &key, value)
            .map_err(|e| format!("Failed to mark operation as applied: {}", e));

        self.metrics.mark_duration.record(start.elapsed());
        result
    }

    /// Mark multiple operations as applied (batch operation)
    ///
    /// More efficient than calling mark_applied() multiple times.
    ///
    /// # Arguments
    /// * `op_ids` - Iterator of (operation ID, timestamp) pairs
    pub fn mark_applied_batch<I>(&self, op_ids: I) -> Result<(), String>
    where
        I: IntoIterator<Item = (Uuid, u64)>,
    {
        let start = Instant::now();

        let cf = self.get_cf()?;
        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0;

        for (op_id, timestamp_ms) in op_ids {
            let key = self.make_key(&op_id);
            let value = timestamp_ms.to_be_bytes();
            batch.put_cf(&cf, &key, value);
            count += 1;
        }

        let result = self
            .db
            .write(batch)
            .map_err(|e| format!("Failed to mark operations as applied: {}", e));

        self.metrics.mark_duration.record(start.elapsed());
        self.metrics
            .batch_sizes
            .record(std::time::Duration::from_millis(count));
        result
    }

    /// Load all applied operation IDs into memory
    ///
    /// This is useful for initializing an in-memory cache at startup.
    /// For large datasets, consider using is_applied() directly instead.
    ///
    /// # Returns
    /// Set of all applied operation IDs
    pub fn load_all_applied(&self) -> Result<HashSet<Uuid>, String> {
        let cf = self.get_cf()?;
        let mut applied = HashSet::new();

        let iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, _value) =
                item.map_err(|e| format!("Failed to iterate applied ops: {}", e))?;

            // Extract UUID from key
            if let Some(op_id) = self.parse_key(&key) {
                applied.insert(op_id);
            }
        }

        Ok(applied)
    }

    /// Count the number of applied operations
    ///
    /// Useful for monitoring and diagnostics
    pub fn count_applied(&self) -> Result<usize, String> {
        let cf = self.get_cf()?;
        let mut count = 0;

        let iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
        for item in iter {
            item.map_err(|e| format!("Failed to count applied ops: {}", e))?;
            count += 1;
        }

        Ok(count)
    }

    /// Remove old applied operation IDs (garbage collection)
    ///
    /// Operations older than `ttl_ms` milliseconds will be removed.
    /// This prevents unbounded growth of the applied ops set.
    ///
    /// # Arguments
    /// * `current_time_ms` - Current timestamp in milliseconds
    /// * `ttl_ms` - Time-to-live in milliseconds (e.g., 30 days = 30 * 24 * 60 * 60 * 1000)
    ///
    /// # Returns
    /// Number of operation IDs removed
    pub fn gc_old_operations(&self, current_time_ms: u64, ttl_ms: u64) -> Result<usize, String> {
        let cf = self.get_cf()?;
        let cutoff_time = current_time_ms.saturating_sub(ttl_ms);

        let mut to_delete = Vec::new();

        // Find operations older than cutoff
        let iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item.map_err(|e| format!("Failed to iterate for GC: {}", e))?;

            // Parse timestamp from value
            if value.len() >= 8 {
                let timestamp_bytes: [u8; 8] = value[0..8].try_into().unwrap();
                let timestamp = u64::from_be_bytes(timestamp_bytes);

                if timestamp < cutoff_time {
                    to_delete.push(key.to_vec());
                }
            }
        }

        let count = to_delete.len();

        // Delete in batch
        if !to_delete.is_empty() {
            let mut batch = rocksdb::WriteBatch::default();
            for key in to_delete {
                batch.delete_cf(&cf, &key);
            }

            self.db
                .write(batch)
                .map_err(|e| format!("Failed to delete old applied ops: {}", e))?;
        }

        Ok(count)
    }

    /// Get metrics for this idempotency tracker
    pub fn get_metrics(&self) -> IdempotencyMetrics {
        let checks = self.metrics.checks_total.get();
        let hits = self.metrics.hits_total.get();
        let hit_rate = if checks > 0 {
            (hits as f64 / checks as f64) * 100.0
        } else {
            0.0
        };

        // Count total operations tracked
        let tracked_ops = self.count_applied().unwrap_or(0) as u64;

        // Estimate disk usage: key (16 bytes UUID + prefix) + value (8 bytes timestamp) + overhead (~32 bytes per entry)
        let disk_bytes = tracked_ops * 56;

        IdempotencyMetrics {
            checks_total: checks,
            hits_total: hits,
            misses_total: self.metrics.misses_total.get(),
            hit_rate_percent: hit_rate,
            tracked_operations: tracked_ops,
            memory_bytes: 0, // Persistent tracker uses disk, not memory
            disk_bytes,
            avg_check_duration_ms: self.metrics.check_duration.avg_ms(),
            avg_mark_duration_ms: self.metrics.mark_duration.avg_ms(),
            avg_batch_size: self.metrics.batch_sizes.avg_ms(),
            p99_check_latency_ms: self.metrics.check_duration.percentile(99.0).as_millis() as u64,
            timestamp: raisin_replication::metrics::current_timestamp_ms(),
        }
    }

    /// Helper: Get column family handle
    fn get_cf(&self) -> Result<&ColumnFamily, String> {
        self.db
            .cf_handle(&self.cf_name)
            .ok_or_else(|| format!("Column family '{}' not found", self.cf_name))
    }

    /// Helper: Create key for an operation ID
    fn make_key(&self, op_id: &Uuid) -> Vec<u8> {
        let mut key = Vec::with_capacity(APPLIED_OPS_PREFIX.len() + 16);
        key.extend_from_slice(APPLIED_OPS_PREFIX);
        key.extend_from_slice(op_id.as_bytes());
        key
    }

    /// Helper: Parse UUID from key
    fn parse_key(&self, key: &[u8]) -> Option<Uuid> {
        if key.len() >= APPLIED_OPS_PREFIX.len() + 16 {
            let uuid_bytes = &key[APPLIED_OPS_PREFIX.len()..APPLIED_OPS_PREFIX.len() + 16];
            Uuid::from_slice(uuid_bytes).ok()
        } else {
            None
        }
    }
}

/// Implement the IdempotencyTracker trait for persistent storage
impl raisin_replication::IdempotencyTracker for PersistentIdempotencyTracker {
    fn is_applied(&self, op_id: &Uuid) -> Result<bool, String> {
        PersistentIdempotencyTracker::is_applied(self, op_id)
    }

    fn mark_applied(&mut self, op_id: &Uuid, timestamp_ms: u64) -> Result<(), String> {
        // Note: RocksDB operations don't require mut self, but trait requires it
        // This is fine since we have interior mutability through Arc<DB>
        PersistentIdempotencyTracker::mark_applied(self, op_id, timestamp_ms)
    }

    fn mark_applied_batch(&mut self, op_ids: &[(Uuid, u64)]) -> Result<(), String> {
        PersistentIdempotencyTracker::mark_applied_batch(self, op_ids.iter().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_db() -> (TempDir, Arc<DB>) {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cf_descriptor =
            rocksdb::ColumnFamilyDescriptor::new("applied_ops", rocksdb::Options::default());

        let db = DB::open_cf_descriptors(&opts, path, vec![cf_descriptor]).unwrap();

        (temp_dir, Arc::new(db))
    }

    #[test]
    fn test_mark_and_check_applied() {
        let (_temp_dir, db) = create_test_db();
        let tracker = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());

        let op_id = Uuid::new_v4();

        // Initially not applied
        assert!(!tracker.is_applied(&op_id).unwrap());

        // Mark as applied
        tracker.mark_applied(&op_id, 1000).unwrap();

        // Now should be applied
        assert!(tracker.is_applied(&op_id).unwrap());
    }

    #[test]
    fn test_batch_mark_applied() {
        let (_temp_dir, db) = create_test_db();
        let tracker = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());

        let op_ids: Vec<(Uuid, u64)> = (0..10).map(|i| (Uuid::new_v4(), 1000 + i * 100)).collect();

        // Mark all as applied
        tracker.mark_applied_batch(op_ids.iter().copied()).unwrap();

        // Check all are marked
        for (op_id, _) in &op_ids {
            assert!(tracker.is_applied(op_id).unwrap());
        }

        assert_eq!(tracker.count_applied().unwrap(), 10);
    }

    #[test]
    fn test_load_all_applied() {
        let (_temp_dir, db) = create_test_db();
        let tracker = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());

        let op_ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();

        // Mark ops as applied
        for (i, op_id) in op_ids.iter().enumerate() {
            tracker.mark_applied(op_id, 1000 + i as u64 * 100).unwrap();
        }

        // Load all
        let loaded = tracker.load_all_applied().unwrap();

        assert_eq!(loaded.len(), 5);
        for op_id in &op_ids {
            assert!(loaded.contains(op_id));
        }
    }

    #[test]
    fn test_gc_old_operations() {
        let (_temp_dir, db) = create_test_db();
        let tracker = PersistentIdempotencyTracker::new(db, "applied_ops".to_string());

        // Mark operations with different timestamps
        let old_op = Uuid::new_v4();
        let recent_op = Uuid::new_v4();

        tracker.mark_applied(&old_op, 1000).unwrap(); // Old
        tracker.mark_applied(&recent_op, 100_000).unwrap(); // Recent

        // GC with cutoff at 50,000 (should remove old_op)
        let current_time = 110_000;
        let ttl = 60_000; // 60 seconds

        let removed = tracker.gc_old_operations(current_time, ttl).unwrap();

        assert_eq!(removed, 1);
        assert!(!tracker.is_applied(&old_op).unwrap());
        assert!(tracker.is_applied(&recent_op).unwrap());
    }

    #[test]
    fn test_persistence_across_reopens() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().to_path_buf();

        let op_id = Uuid::new_v4();

        // First session: create and mark operation
        {
            let mut opts = rocksdb::Options::default();
            opts.create_if_missing(true);
            opts.create_missing_column_families(true);

            let cf_descriptor =
                rocksdb::ColumnFamilyDescriptor::new("applied_ops", rocksdb::Options::default());

            let db = DB::open_cf_descriptors(&opts, &path, vec![cf_descriptor]).unwrap();
            let tracker =
                PersistentIdempotencyTracker::new(Arc::new(db), "applied_ops".to_string());

            tracker.mark_applied(&op_id, 1000).unwrap();
        }

        // Second session: reopen and check
        {
            let mut opts = rocksdb::Options::default();
            let cf_descriptor =
                rocksdb::ColumnFamilyDescriptor::new("applied_ops", rocksdb::Options::default());

            let db = DB::open_cf_descriptors(&opts, &path, vec![cf_descriptor]).unwrap();
            let tracker =
                PersistentIdempotencyTracker::new(Arc::new(db), "applied_ops".to_string());

            // Operation should still be marked as applied
            assert!(tracker.is_applied(&op_id).unwrap());
        }
    }
}
