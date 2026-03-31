// TODO(v0.2): Vector clock verification handler for replication
#![allow(dead_code)]

//! Vector clock snapshot verification job handler
//!
//! This handler performs periodic verification of vector clock snapshots to ensure
//! they remain consistent with the actual operation log. If inconsistencies are
//! detected, the snapshot is automatically rebuilt.

use crate::repositories::OpLogRepository;
use raisin_error::Result;
use raisin_storage::jobs::{JobContext, JobInfo};
use rocksdb::DB;
use std::sync::Arc;
use tracing::{info, warn};

/// Handler for vector clock snapshot verification jobs
///
/// This job should be scheduled periodically (e.g., daily) to verify that
/// the vector clock snapshot remains in sync with the operation log.
///
/// # What it does
///
/// 1. Reads the current persisted vector clock snapshot
/// 2. Rebuilds the vector clock from the operation log
/// 3. Compares the two
/// 4. If they differ, logs a warning and keeps the rebuilt version
///
/// # Scheduling
///
/// Recommended frequency: Once per day (86400 seconds)
/// Priority: Low (this is a maintenance task)
///
/// # Performance Impact
///
/// This job does a full scan of the operation log, so it can be expensive
/// for large logs. However, it only runs periodically and helps ensure
/// snapshot integrity.
pub struct VectorClockVerificationHandler {
    db: Arc<DB>,
}

impl VectorClockVerificationHandler {
    /// Create a new vector clock verification handler
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Handle a vector clock verification job
    ///
    /// # Arguments
    ///
    /// * `job` - The job information
    /// * `context` - The job context containing tenant and repo IDs
    ///
    /// # Returns
    ///
    /// - `Ok(())` - Verification completed (snapshot was correct or has been corrected)
    /// - `Err(_)` - If verification failed due to an error
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        let tenant_id = &context.tenant_id;
        let repo_id = &context.repo_id;

        info!(
            job_id = %job.id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            "Starting vector clock snapshot verification"
        );

        let oplog_repo = OpLogRepository::new(self.db.clone());

        // Verify the snapshot (this also rebuilds if different)
        let is_consistent = oplog_repo.verify_vector_clock_snapshot(tenant_id, repo_id)?;

        if is_consistent {
            info!(
                job_id = %job.id,
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                "Vector clock snapshot verified successfully - snapshot is consistent"
            );
        } else {
            warn!(
                job_id = %job.id,
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                "Vector clock snapshot mismatch detected - snapshot has been rebuilt from operations"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RocksDBConfig;
    use raisin_replication::{OpType, Operation, VectorClock};
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_vector_clock_verification_basic() {
        // Create temporary database
        let dir = tempdir().unwrap();
        let config = RocksDBConfig {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };
        let db = Arc::new(crate::open_db_with_config(&config).unwrap());

        let oplog_repo = OpLogRepository::new(db.clone());

        // Create and store some operations
        let mut vc = VectorClock::new();
        vc.increment("node1");

        let op1 = Operation::new(
            1,
            "node1".to_string(),
            vc.clone(),
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            OpType::CreateNode {
                node_id: "test123".to_string(),
                name: "Test Article".to_string(),
                node_type: "article".to_string(),
                archetype: None,
                parent_id: None,
                order_key: "a".to_string(),
                properties: std::collections::HashMap::new(),
                owner_id: None,
                workspace: None,
                path: String::new(),
            },
            "test@example.com".to_string(),
        );

        oplog_repo.put_operation(&op1).unwrap();

        // Initialize snapshot
        oplog_repo
            .rebuild_vector_clock_snapshot("tenant1", "repo1")
            .unwrap();

        // Verify it's consistent
        let is_consistent = oplog_repo
            .verify_vector_clock_snapshot("tenant1", "repo1")
            .unwrap();

        assert!(is_consistent);
    }
}
