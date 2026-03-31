//! Operation log compaction job handler
//!
//! This handler manages periodic compaction of the operation log to reduce
//! storage footprint and improve synchronization efficiency.

use crate::repositories::OpLogRepository;
use raisin_error::Result;
use raisin_replication::{CompactionConfig, OperationLogCompactor};
use raisin_storage::jobs::{JobContext, JobInfo};
use rocksdb::DB;
use std::sync::Arc;
use tracing::info;

/// Handler for operation log compaction jobs
pub struct OpLogCompactionHandler {
    db: Arc<DB>,
    compaction_config: CompactionConfig,
}

impl OpLogCompactionHandler {
    /// Create a new operation log compaction handler with default configuration
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            compaction_config: CompactionConfig::default(),
        }
    }

    /// Create handler with custom compaction configuration
    pub fn with_config(db: Arc<DB>, compaction_config: CompactionConfig) -> Self {
        Self {
            db,
            compaction_config,
        }
    }

    /// Handle an operation log compaction job
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        let tenant_id = &context.tenant_id;
        let repo_id = &context.repo_id;

        info!(
            job_id = %job.id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            min_age_secs = self.compaction_config.min_age_secs,
            merge_property_updates = self.compaction_config.merge_property_updates,
            "Starting operation log compaction"
        );

        let oplog_repo = OpLogRepository::new(self.db.clone());
        let compactor = OperationLogCompactor::new(self.compaction_config.clone());

        // Perform compaction
        let results = oplog_repo.compact_oplog(tenant_id, repo_id, &compactor)?;

        // Aggregate statistics
        let total_original: usize = results.values().map(|r| r.original_count).sum();
        let total_compacted: usize = results.values().map(|r| r.compacted_count).sum();
        let total_merged: usize = results.values().map(|r| r.merged_count).sum();
        let total_bytes_saved: usize = results.values().map(|r| r.bytes_saved).sum();

        if total_merged == 0 {
            info!(
                job_id = %job.id,
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                total_operations = total_original,
                "No operations eligible for compaction"
            );
        } else {
            let reduction_percent = if total_original > 0 {
                (total_merged as f64 / total_original as f64) * 100.0
            } else {
                0.0
            };

            info!(
                job_id = %job.id,
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                nodes_compacted = results.len(),
                original_count = total_original,
                compacted_count = total_compacted,
                merged_count = total_merged,
                bytes_saved = total_bytes_saved,
                reduction_percent = format!("{:.1}%", reduction_percent),
                "Operation log compaction completed successfully"
            );

            // Log per-node statistics
            for (cluster_node_id, result) in &results {
                if result.merged_count > 0 {
                    info!(
                        job_id = %job.id,
                        cluster_node_id = %cluster_node_id,
                        original_count = result.original_count,
                        compacted_count = result.compacted_count,
                        merged_count = result.merged_count,
                        bytes_saved = result.bytes_saved,
                        "Compacted operations from cluster node"
                    );

                    // Log per-node stats if available
                    if let Some(stats) = result.per_node_stats.get(cluster_node_id) {
                        info!(
                            job_id = %job.id,
                            cluster_node_id = %cluster_node_id,
                            property_sequences_merged = stats.property_sequences_merged,
                            "Property update sequences merged"
                        );
                    }
                }
            }
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
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn make_test_operation(
        cluster_node_id: &str,
        op_seq: u64,
        timestamp_ms: u64,
        storage_node_id: &str,
        property: &str,
        value: &str,
    ) -> Operation {
        let mut vc = VectorClock::new();
        vc.increment(cluster_node_id);

        let mut op = Operation::new(
            op_seq,
            cluster_node_id.to_string(),
            vc,
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            OpType::SetProperty {
                node_id: storage_node_id.to_string(),
                property_name: property.to_string(),
                value: raisin_models::nodes::properties::PropertyValue::String(value.to_string()),
            },
            "test@example.com".to_string(),
        );

        // Override timestamp for testing
        op.timestamp_ms = timestamp_ms;
        op
    }

    #[tokio::test]
    async fn test_oplog_compaction_handler() {
        // Create temporary database
        let dir = tempdir().unwrap();
        let config = RocksDBConfig {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };
        let db = Arc::new(crate::open_db_with_config(&config).unwrap());

        // Create handler with short min_age for testing
        let compaction_config = CompactionConfig {
            min_age_secs: 1, // 1 second for testing
            merge_property_updates: true,
            batch_size: 100_000,
        };
        let handler = OpLogCompactionHandler::with_config(db.clone(), compaction_config);

        // Create some test operations (sequence of property updates)
        let oplog_repo = OpLogRepository::new(db.clone());

        let base_time = chrono::Utc::now().timestamp_millis() as u64 - 10_000; // 10 seconds ago

        let ops = vec![
            make_test_operation("cluster1", 1, base_time, "doc123", "title", "v1"),
            make_test_operation("cluster1", 2, base_time + 1000, "doc123", "title", "v2"),
            make_test_operation("cluster1", 3, base_time + 2000, "doc123", "title", "v3"),
            make_test_operation("cluster1", 4, base_time + 3000, "doc456", "title", "v1"),
        ];

        for op in &ops {
            oplog_repo.put_operation(op).unwrap();
        }

        // Create job info
        let job = JobInfo {
            id: JobId::new(),
            job_type: JobType::OpLogCompaction {
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
            metadata: HashMap::new(),
        };

        // Run compaction
        handler.handle(&job, &context).await.unwrap();

        // Verify compaction happened
        let remaining_ops = oplog_repo
            .get_operations_from_node("tenant1", "repo1", "cluster1")
            .unwrap();

        // Should have merged ops 1,2,3 into just op 3, plus op 4 = 2 total
        assert_eq!(
            remaining_ops.len(),
            2,
            "Should have compacted 3 property updates into 1"
        );

        // Verify the remaining operations are the correct ones
        assert!(
            remaining_ops.iter().any(|op| op.op_seq == 3),
            "Should keep the last operation from merged sequence"
        );
        assert!(
            remaining_ops.iter().any(|op| op.op_seq == 4),
            "Should keep non-merged operation"
        );
    }

    #[tokio::test]
    async fn test_no_compaction_when_recent() {
        // Create temporary database
        let dir = tempdir().unwrap();
        let config = RocksDBConfig {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };
        let db = Arc::new(crate::open_db_with_config(&config).unwrap());

        // Create handler with long min_age
        let compaction_config = CompactionConfig {
            min_age_secs: 3600, // 1 hour
            merge_property_updates: true,
            batch_size: 100_000,
        };
        let handler = OpLogCompactionHandler::with_config(db.clone(), compaction_config);

        // Create recent operations
        let oplog_repo = OpLogRepository::new(db.clone());
        let now = chrono::Utc::now().timestamp_millis() as u64;

        let ops = vec![
            make_test_operation("cluster1", 1, now - 1000, "doc123", "title", "v1"),
            make_test_operation("cluster1", 2, now - 500, "doc123", "title", "v2"),
        ];

        for op in &ops {
            oplog_repo.put_operation(op).unwrap();
        }

        // Create job and context
        let job = JobInfo {
            id: JobId::new(),
            job_type: JobType::OpLogCompaction {
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
            metadata: HashMap::new(),
        };

        // Run compaction
        handler.handle(&job, &context).await.unwrap();

        // Verify no compaction (operations too recent)
        let remaining_ops = oplog_repo
            .get_operations_from_node("tenant1", "repo1", "cluster1")
            .unwrap();

        assert_eq!(
            remaining_ops.len(),
            2,
            "Recent operations should not be compacted"
        );
    }
}
