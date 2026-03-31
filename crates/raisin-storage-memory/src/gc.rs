use raisin_error::Result;
use raisin_storage::{GarbageCollectionRepository, GarbageCollectionStats};

/// Stub in-memory garbage collector for testing
#[derive(Debug, Default, Clone)]
pub struct InMemoryGarbageCollector {
    // In-memory GC state would go here
}

impl GarbageCollectionRepository for InMemoryGarbageCollector {
    async fn garbage_collect(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _dry_run: bool,
    ) -> Result<GarbageCollectionStats> {
        // TODO: Implement in-memory GC
        Ok(GarbageCollectionStats {
            revisions_examined: 0,
            revisions_reachable: 0,
            revisions_deleted: 0,
            snapshots_deleted: 0,
            bytes_reclaimed: 0,
            duration_ms: 0,
        })
    }

    async fn list_unreferenced_revisions(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
    ) -> Result<Vec<u64>> {
        // TODO: Implement in-memory unreferenced revision listing
        Ok(vec![])
    }
}
