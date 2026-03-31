//! Storage traits for vector embeddings and embedding jobs.
//!
//! This module defines the storage abstractions needed for the embeddings system:
//!
//! - `EmbeddingStorage` - Store and retrieve embedding vectors
//! - `EmbeddingJobStore` - Manage background embedding generation jobs

use raisin_error::Result;
use raisin_hlc::HLC;

use crate::models::{EmbeddingData, EmbeddingJob};

/// Storage for vector embeddings.
///
/// Embeddings are stored in RocksDB for direct access and revision history.
/// The HNSW index uses these embeddings for fast KNN search.
///
/// # Key Format
///
/// `{tenant}\0{repo}\0{branch}\0{workspace}\0{node_id}\0{revision:HLC:16bytes}`
///
/// Revisions are encoded as full HLC (16 bytes) in descending ordering,
/// preserving both timestamp and counter components. Latest revisions sort first.
///
/// # Revision Handling
///
/// - `store_embedding()` - Always stores at exact revision (full HLC)
/// - `get_embedding()` - With `None` revision, returns latest (first match in prefix scan)
/// - `delete_embedding()` - With `None` revision, deletes all revisions for node
pub trait EmbeddingStorage: Send + Sync {
    /// Store an embedding for a node at a specific revision.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace_id` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `revision` - Exact revision (full HLC with timestamp and counter)
    /// * `data` - Embedding data to store
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let revision = HLC::new(1705843009213693952, 42);
    /// storage.store_embedding(
    ///     "tenant1",
    ///     "repo1",
    ///     "main",
    ///     "default",
    ///     "node123",
    ///     &revision,
    ///     &embedding_data
    /// )?;
    /// ```
    fn store_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: &HLC,
        data: &EmbeddingData,
    ) -> Result<()>;

    /// Get an embedding for a node.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace_id` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `revision` - Specific revision (full HLC), or `None` for latest
    ///
    /// # Returns
    ///
    /// - `Ok(Some(data))` - Embedding found
    /// - `Ok(None)` - No embedding exists
    /// - `Err(_)` - Storage error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Get latest embedding
    /// let latest = storage.get_embedding(
    ///     "tenant1", "repo1", "main", "default", "node123", None
    /// )?;
    ///
    /// // Get embedding at specific revision
    /// let revision = HLC::new(1705843009213693952, 42);
    /// let historical = storage.get_embedding(
    ///     "tenant1", "repo1", "main", "default", "node123", Some(&revision)
    /// )?;
    /// ```
    fn get_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: Option<&HLC>,
    ) -> Result<Option<EmbeddingData>>;

    /// Delete embeddings for a node.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace_id` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `revision` - Specific revision to delete (full HLC), or `None` to delete all
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Delete all embeddings for a node
    /// storage.delete_embedding(
    ///     "tenant1", "repo1", "main", "default", "node123", None
    /// )?;
    ///
    /// // Delete embedding at specific revision
    /// let revision = HLC::new(1705843009213693952, 42);
    /// storage.delete_embedding(
    ///     "tenant1", "repo1", "main", "default", "node123", Some(&revision)
    /// )?;
    /// ```
    fn delete_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: Option<&HLC>,
    ) -> Result<()>;

    /// List all node IDs with embeddings in a branch.
    ///
    /// This is useful for rebuilding HNSW indexes from RocksDB.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace_id` - Workspace identifier
    ///
    /// # Returns
    ///
    /// Vector of (node_id, latest_revision) tuples where revision is full HLC
    fn list_embeddings(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
    ) -> Result<Vec<(String, HLC)>>;
}

/// Storage for embedding generation jobs.
///
/// Jobs are enqueued when nodes are created/updated/deleted and processed
/// by background workers.
///
/// # Job Lifecycle
///
/// 1. **Enqueue** - Job created in response to node event
/// 2. **Dequeue** - Worker picks up job for processing
/// 3. **Complete** - Job successfully processed
/// 4. **Fail** - Job failed with error
///
/// Failed jobs can be retried or manually inspected.
pub trait EmbeddingJobStore: Send + Sync {
    /// Enqueue a new embedding job.
    ///
    /// # Arguments
    ///
    /// * `job` - Job to enqueue
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let job = EmbeddingJob::add_node(
    ///     "tenant1".to_string(),
    ///     "repo1".to_string(),
    ///     "main".to_string(),
    ///     "default".to_string(),
    ///     "node123".to_string(),
    ///     42,
    /// );
    ///
    /// job_store.enqueue(&job)?;
    /// ```
    fn enqueue(&self, job: &EmbeddingJob) -> Result<()>;

    /// Dequeue jobs for processing.
    ///
    /// Returns up to `limit` pending jobs, ordered by creation time (FIFO).
    ///
    /// # Arguments
    ///
    /// * `limit` - Maximum number of jobs to return
    ///
    /// # Returns
    ///
    /// Vector of jobs ready for processing
    fn dequeue(&self, limit: usize) -> Result<Vec<EmbeddingJob>>;

    /// Mark jobs as completed.
    ///
    /// # Arguments
    ///
    /// * `job_ids` - Job IDs to mark as complete
    fn complete(&self, job_ids: &[String]) -> Result<()>;

    /// Mark a job as failed.
    ///
    /// # Arguments
    ///
    /// * `job_id` - Job ID
    /// * `error` - Error message
    fn fail(&self, job_id: &str, error: &str) -> Result<()>;

    /// Get job by ID.
    ///
    /// Useful for debugging and monitoring.
    fn get(&self, job_id: &str) -> Result<Option<EmbeddingJob>>;

    /// List all pending jobs.
    ///
    /// Returns jobs ordered by creation time.
    fn list_pending(&self) -> Result<Vec<EmbeddingJob>>;

    /// Count pending jobs.
    ///
    /// Useful for monitoring queue depth.
    fn count_pending(&self) -> Result<usize>;
}
