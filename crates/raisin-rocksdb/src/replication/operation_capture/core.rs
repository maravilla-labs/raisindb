//! Core operation capture struct and state management
//!
//! Contains the `OperationCapture` struct, constructors, clock management,
//! and the main `capture_operation` entry points.

use crate::repositories::OpLogRepository;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_replication::{OpType, Operation, VectorClock};
use rocksdb::DB;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// Callback for pushing operations to peers after capture
type PushCallback = Arc<dyn Fn(Operation) -> Result<()> + Send + Sync>;

#[derive(Clone)]
struct RepoClockState {
    vector_clock: VectorClock,
    op_seq: u64,
}

impl RepoClockState {
    fn new() -> Self {
        Self {
            vector_clock: VectorClock::new(),
            op_seq: 0,
        }
    }
}

/// Captures operations for replication
pub struct OperationCapture {
    /// Database handle
    db: Arc<DB>,

    /// Operation log repository
    oplog_repo: OpLogRepository,

    /// Unique identifier for this cluster node (server instance)
    cluster_node_id: String,

    /// Per-(tenant,repo) vector clock + op_seq tracking
    repo_states: Arc<RwLock<HashMap<(String, String), RepoClockState>>>,

    /// Whether operation capture is enabled
    enabled: Arc<AtomicBool>,

    /// Optional callback to push operations to peers after capture
    push_callback: Arc<RwLock<Option<PushCallback>>>,
}

impl OperationCapture {
    /// Create a new operation capture instance
    ///
    /// # Arguments
    /// * `db` - RocksDB instance
    /// * `cluster_node_id` - Unique identifier for this cluster node (server instance)
    pub fn new(db: Arc<DB>, cluster_node_id: String) -> Self {
        let oplog_repo = OpLogRepository::new(db.clone());

        Self {
            db,
            oplog_repo,
            cluster_node_id: cluster_node_id.clone(),
            repo_states: Arc::new(RwLock::new(HashMap::new())),
            enabled: Arc::new(AtomicBool::new(true)),
            push_callback: Arc::new(RwLock::new(None)),
        }
    }

    /// Set callback for pushing operations to peers
    pub async fn set_push_callback<F>(&self, callback: F)
    where
        F: Fn(Operation) -> Result<()> + Send + Sync + 'static,
    {
        let mut cb = self.push_callback.write().await;
        *cb = Some(Arc::new(callback));
    }

    /// Create a new operation capture instance with async queue enabled
    ///
    /// This method creates both an OperationCapture and an OperationQueue for
    /// high-throughput asynchronous operation processing.
    ///
    /// # Arguments
    ///
    /// * `db` - RocksDB instance
    /// * `cluster_node_id` - Unique identifier for this cluster node
    /// * `queue_capacity` - Maximum number of operations in queue
    /// * `batch_size` - Number of operations to batch before writing
    /// * `batch_timeout` - Maximum time to wait for a full batch
    pub fn new_with_queue(
        db: Arc<DB>,
        cluster_node_id: String,
        queue_capacity: usize,
        batch_size: usize,
        batch_timeout: std::time::Duration,
    ) -> (Arc<Self>, Arc<crate::replication::OperationQueue>) {
        let capture = Arc::new(Self::new(db, cluster_node_id));
        let queue = Arc::new(crate::replication::OperationQueue::new(
            Arc::clone(&capture),
            queue_capacity,
            batch_size,
            batch_timeout,
        ));

        (capture, queue)
    }

    /// Create with operation capture disabled
    pub fn disabled(db: Arc<DB>) -> Self {
        Self {
            db: db.clone(),
            oplog_repo: OpLogRepository::new(db),
            cluster_node_id: "disabled".to_string(),
            repo_states: Arc::new(RwLock::new(HashMap::new())),
            enabled: Arc::new(AtomicBool::new(false)),
            push_callback: Arc::new(RwLock::new(None)),
        }
    }

    /// Disable operation capture (thread-safe, no &mut self required)
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::SeqCst);
    }

    /// Enable operation capture (thread-safe, no &mut self required)
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }

    /// Check if operation capture is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Get the cluster node ID for this instance
    pub fn cluster_node_id(&self) -> &str {
        &self.cluster_node_id
    }

    fn repo_key(tenant_id: &str, repo_id: &str) -> (String, String) {
        (tenant_id.to_string(), repo_id.to_string())
    }

    async fn advance_repo_clock(&self, tenant_id: &str, repo_id: &str) -> (VectorClock, u64) {
        let mut states = self.repo_states.write().await;
        let key = Self::repo_key(tenant_id, repo_id);
        let state = states.entry(key).or_insert_with(RepoClockState::new);
        state.vector_clock.increment(&self.cluster_node_id);
        state.op_seq += 1;
        (state.vector_clock.clone(), state.op_seq)
    }

    /// Get a snapshot of the current vector clock for a repo
    pub async fn get_vector_clock(&self, tenant_id: &str, repo_id: &str) -> VectorClock {
        let states = self.repo_states.read().await;
        states
            .get(&Self::repo_key(tenant_id, repo_id))
            .map(|s| s.vector_clock.clone())
            .unwrap_or_else(VectorClock::new)
    }

    /// Merge a remote vector clock (for tracking what we've seen from peers)
    pub async fn merge_vector_clock(
        &self,
        tenant_id: &str,
        repo_id: &str,
        remote_vc: &VectorClock,
    ) {
        let mut states = self.repo_states.write().await;
        let state = states
            .entry(Self::repo_key(tenant_id, repo_id))
            .or_insert_with(RepoClockState::new);
        state.vector_clock.merge(remote_vc);
        if let Some(counter) = state.vector_clock.as_map().get(&self.cluster_node_id) {
            state.op_seq = *counter;
        }
    }

    /// Get the current op_seq for a repo (for testing/diagnostics)
    pub async fn get_op_seq(&self, tenant_id: &str, repo_id: &str) -> u64 {
        let states = self.repo_states.read().await;
        states
            .get(&Self::repo_key(tenant_id, repo_id))
            .map(|s| s.op_seq)
            .unwrap_or(0)
    }

    /// Capture an operation to the operation log
    ///
    /// This is the main entry point for logging operations.
    pub async fn capture_operation(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        op_type: OpType,
        actor: String,
        message: Option<String>,
        is_system: bool,
    ) -> Result<Operation> {
        self.capture_operation_with_revision(
            tenant_id, repo_id, branch, op_type, actor, message, is_system, None,
        )
        .await
    }

    /// Capture an operation with an explicit revision value
    pub async fn capture_operation_with_revision(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        op_type: OpType,
        actor: String,
        message: Option<String>,
        is_system: bool,
        revision: Option<HLC>,
    ) -> Result<Operation> {
        if !self.is_enabled() {
            return Ok(Operation {
                op_id: uuid::Uuid::new_v4(),
                op_seq: 0,
                cluster_node_id: self.cluster_node_id.clone(),
                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                vector_clock: VectorClock::new(),
                tenant_id,
                repo_id,
                branch,
                op_type,
                revision,
                actor,
                message,
                is_system,
                acknowledged_by: HashSet::new(),
            });
        }

        let (vector_clock, op_seq) = self.advance_repo_clock(&tenant_id, &repo_id).await;

        let op = Operation {
            op_id: uuid::Uuid::new_v4(),
            op_seq,
            cluster_node_id: self.cluster_node_id.clone(),
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            vector_clock,
            tenant_id: tenant_id.clone(),
            repo_id: repo_id.clone(),
            branch,
            op_type,
            revision,
            actor,
            message,
            is_system,
            acknowledged_by: HashSet::new(),
        };

        tracing::info!(
            op_id = %op.op_id,
            op_seq = op.op_seq,
            cluster_node_id = %op.cluster_node_id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            op_type = ?op.op_type,
            revision = ?op.revision,
            "CAPTURING OPERATION with revision={:?}",
            op.revision
        );

        self.oplog_repo.put_operation(&op)?;

        self.oplog_repo.increment_vector_clock_for_node(
            &tenant_id,
            &repo_id,
            &self.cluster_node_id,
            op.op_seq,
        )?;

        debug!(
            op_id = %op.op_id,
            op_seq = op.op_seq,
            cluster_node_id = %op.cluster_node_id,
            op_type = %format!("{:?}", op.op_type),
            "Captured operation and updated vector clock snapshot"
        );

        // Push to peers if callback is set (real-time replication)
        let callback_guard = self.push_callback.read().await;
        if let Some(ref callback) = *callback_guard {
            tracing::info!(
                op_id = %op.op_id,
                op_seq = op.op_seq,
                "INVOKING PUSH CALLBACK for real-time replication"
            );
            if let Err(e) = callback(op.clone()) {
                tracing::error!(
                    op_id = %op.op_id,
                    error = %e,
                    "PUSH CALLBACK FAILED"
                );
            } else {
                tracing::info!(
                    op_id = %op.op_id,
                    "PUSH CALLBACK SUCCEEDED"
                );
            }
        } else {
            tracing::warn!(
                op_id = %op.op_id,
                "NO PUSH CALLBACK SET - operation will not replicate in real-time"
            );
        }
        drop(callback_guard);

        Ok(op)
    }

    /// Capture a batch of operations (for bulk operations)
    pub async fn capture_operations_batch(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        op_types: Vec<OpType>,
        actor: String,
        message: Option<String>,
        is_system: bool,
    ) -> Result<Vec<Operation>> {
        if !self.is_enabled() {
            return Ok(Vec::new());
        }

        let mut operations = Vec::with_capacity(op_types.len());

        for op_type in op_types {
            let op = self
                .capture_operation(
                    tenant_id.clone(),
                    repo_id.clone(),
                    branch.clone(),
                    op_type,
                    actor.clone(),
                    message.clone(),
                    is_system,
                )
                .await?;
            operations.push(op);
        }

        Ok(operations)
    }

    /// Restore vector clock and op_seq from existing operation log
    ///
    /// This should be called on startup to resume from where we left off.
    pub async fn restore_from_oplog(&self, tenant_id: &str, repo_id: &str) -> Result<()> {
        let highest_seq =
            self.oplog_repo
                .get_highest_seq(tenant_id, repo_id, &self.cluster_node_id)?;

        let all_ops = self.oplog_repo.get_all_operations(tenant_id, repo_id)?;

        let mut merged_vc = VectorClock::new();
        for (_, ops) in all_ops {
            for op in ops {
                merged_vc.merge(&op.vector_clock);
            }
        }

        {
            let mut states = self.repo_states.write().await;
            let state = states
                .entry(Self::repo_key(tenant_id, repo_id))
                .or_insert_with(RepoClockState::new);
            state.op_seq = highest_seq;
            state.vector_clock = merged_vc;
        }

        debug!(
            cluster_node_id = %self.cluster_node_id,
            op_seq = highest_seq,
            "Restored operation capture state from operation log"
        );

        Ok(())
    }

    /// Get the operation log repository
    pub fn oplog_repo(&self) -> &OpLogRepository {
        &self.oplog_repo
    }
}
