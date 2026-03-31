//! Garbage collection repository implementation

use raisin_error::Result;
use raisin_storage::{GarbageCollectionRepository, GarbageCollectionStats};
use rocksdb::DB;
use std::sync::Arc;

#[derive(Clone)]
pub struct GarbageCollectionRepositoryImpl {
    db: Arc<DB>,
}

impl GarbageCollectionRepositoryImpl {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }
}

impl GarbageCollectionRepository for GarbageCollectionRepositoryImpl {
    async fn garbage_collect(
        &self,
        tenant_id: &str,
        repo_id: &str,
        dry_run: bool,
    ) -> Result<GarbageCollectionStats> {
        let start = std::time::Instant::now();

        // Stub implementation - just return empty stats
        let stats = GarbageCollectionStats {
            revisions_examined: 0,
            revisions_reachable: 0,
            revisions_deleted: 0,
            snapshots_deleted: 0,
            bytes_reclaimed: 0,
            duration_ms: start.elapsed().as_millis() as u64,
        };

        Ok(stats)
    }

    async fn list_unreferenced_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<Vec<u64>> {
        // Stub implementation
        Ok(Vec::new())
    }
}
