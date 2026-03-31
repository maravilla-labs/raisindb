//! Replication garbage collection job handler
//!
//! This handler manages periodic cleanup of the operation log using
//! the GarbageCollector from raisin-replication.

use crate::repositories::OpLogRepository;
use raisin_error::Result;
use raisin_replication::{GarbageCollector, GcConfig, GcStrategy};
use raisin_storage::jobs::{JobContext, JobInfo};
use rocksdb::DB;
use std::sync::Arc;
use tracing::{info, warn};

/// Handler for replication GC jobs
pub struct ReplicationGCHandler {
    db: Arc<DB>,
    gc_config: GcConfig,
}

impl ReplicationGCHandler {
    /// Create a new replication GC handler
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            gc_config: GcConfig::default(),
        }
    }

    /// Create handler with custom GC configuration
    pub fn with_config(db: Arc<DB>, gc_config: GcConfig) -> Self {
        Self { db, gc_config }
    }

    /// Handle a replication GC job
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        let tenant_id = &context.tenant_id;
        let repo_id = &context.repo_id;

        info!(
            job_id = %job.id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            "Starting replication GC"
        );

        let oplog_repo = OpLogRepository::new(self.db.clone());

        // Create garbage collector (in production, watermarks would be loaded from storage)
        let gc = GarbageCollector::with_config(self.gc_config.clone());

        // Perform garbage collection
        let result = oplog_repo.garbage_collect(tenant_id, repo_id, &gc)?;

        // Log results
        match result.strategy {
            GcStrategy::NoOp => {
                info!(
                    job_id = %job.id,
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    "No operations to garbage collect"
                );
            }
            GcStrategy::AcknowledgmentBased => {
                info!(
                    job_id = %job.id,
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    deleted_count = result.deleted_count,
                    bytes_reclaimed = result.bytes_reclaimed,
                    "Garbage collected operations (acknowledgment-based)"
                );
            }
            GcStrategy::TimeBasedFailsafe => {
                warn!(
                    job_id = %job.id,
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    deleted_count = result.deleted_count,
                    bytes_reclaimed = result.bytes_reclaimed,
                    "Garbage collected operations (time-based fail-safe triggered)"
                );
            }
            GcStrategy::Emergency => {
                warn!(
                    job_id = %job.id,
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    deleted_count = result.deleted_count,
                    bytes_reclaimed = result.bytes_reclaimed,
                    "Emergency garbage collection performed (size limit exceeded)"
                );
            }
        }

        // Log per-node statistics
        for (node_id, count) in &result.deleted_by_node {
            info!(
                job_id = %job.id,
                node_id = %node_id,
                deleted_count = count,
                "Deleted operations from node"
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
    use raisin_storage::jobs::{JobId, JobStatus, JobType};
    use std::collections::HashSet;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_replication_gc_handler() {
        // Create temporary database
        let dir = tempdir().unwrap();
        let config = RocksDBConfig {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };
        let db = Arc::new(crate::open_db_with_config(&config).unwrap());

        // Create handler
        let handler = ReplicationGCHandler::new(db.clone());

        // Create some test operations
        let oplog_repo = OpLogRepository::new(db.clone());

        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let old_ms = now_ms - (31 * 24 * 60 * 60 * 1000); // 31 days ago

        // Create old operation (should be GC'd by time-based fail-safe)
        let mut vc1 = VectorClock::new();
        vc1.increment("node1");

        let op1 = Operation {
            op_id: uuid::Uuid::new_v4(),
            op_seq: 1,
            cluster_node_id: "node1".to_string(),
            timestamp_ms: old_ms,
            vector_clock: vc1,
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::SetProperty {
                node_id: "test_node".to_string(),
                property_name: "title".to_string(),
                value: raisin_models::nodes::properties::PropertyValue::String(
                    "Old Value".to_string(),
                ),
            },
            revision: None,
            actor: "test".to_string(),
            message: None,
            is_system: false,
            acknowledged_by: HashSet::new(),
        };

        oplog_repo.put_operation(&op1).unwrap();

        // Create job info
        let job = JobInfo {
            id: JobId::new(),
            job_type: JobType::ReplicationGC {
                tenant_id: "tenant1".to_string(),
                repo_id: "repo1".to_string(),
            },
            status: JobStatus::Running,
            tenant: Some("tenant1".to_string()),
            started_at: chrono::Utc::now(),
            completed_at: None,
            progress: None,
            error: None,
            result: None,
            retry_count: 0,
            max_retries: 3,
            last_heartbeat: None,
            timeout_seconds: 300,
            next_retry_at: None,
        };

        let context = JobContext {
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            workspace_id: "default".to_string(),
            revision: raisin_hlc::HLC::new(1, 0),
            metadata: std::collections::HashMap::new(),
        };

        // Run GC
        handler.handle(&job, &context).await.unwrap();

        // Verify operation was deleted (time-based GC should have removed the 31-day-old op)
        let ops = oplog_repo
            .get_operations_from_node("tenant1", "repo1", "node1")
            .unwrap();
        assert_eq!(
            ops.len(),
            0,
            "Old operation should have been garbage collected"
        );
    }
}
