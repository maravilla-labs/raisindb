//! Storage traits for vector embeddings and embedding jobs.
//!
//! This module defines the storage abstractions needed for the embeddings system:
//!
//! - `EmbeddingStorage` - Store and retrieve embedding vectors
//! - `EmbeddingJobStore` - Manage background embedding generation jobs

use raisin_error::Result;
use raisin_hlc::HLC;

use crate::models::{EmbeddingData, EmbeddingJob};

/// One stored row, addressed the way the HNSW index addresses a vector.
///
/// The unit here is a CHUNK in one embedding space, not a node.
/// [`EmbeddingStorage::list_embeddings`] answers "which nodes have embeddings",
/// which is the right question for a staleness sweep and the wrong one for
/// anything that has to reproduce or count index entries: it collapses a
/// document's chunks into a single row, so a 23-chunk document looks like one
/// vector. A rebuild driven off that count re-indexed one chunk per node and
/// silently discarded the rest, and the verify that compared it against the
/// index's own count reported a permanent mismatch that no rebuild could clear.
///
/// Every field is part of the address, because a row is only re-readable with
/// all of them: [`EmbeddingStorage::get_source_chunk`] takes exactly this tuple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIndexEntry {
    /// Segment 5 of the key: which embedder wrote it.
    pub embedder_hash: String,
    /// Segment 6: `'T'` text, `'I'` image.
    pub kind: char,
    /// Segment 7: the NAMESPACED source id (`{node}` or `{node}#{spec}`).
    pub source_id: String,
    /// Segment 8: the chunk index within the source.
    pub chunk_index: usize,
    /// The newest revision this chunk exists at.
    pub revision: HLC,
}

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

    /// Every LIVE index entry in one workspace: one row per
    /// `(embedder_hash, kind, source_id, chunk_index)`, carrying that chunk's
    /// NEWEST revision.
    ///
    /// This is the unit an HNSW index stores, so it is what a rebuild must
    /// iterate and what a verify must count. See [`StoredIndexEntry`] for what
    /// goes wrong when [`Self::list_embeddings`] is used for either.
    ///
    /// Rows are returned in key order, which groups a source's chunks together
    /// and orders them ascending.
    fn list_index_entries(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
    ) -> Result<Vec<StoredIndexEntry>>;

    /// List every workspace under `{tenant}/{repo}/{branch}` that has at least
    /// one stored embedding.
    ///
    /// Embeddings are written under the workspace the NODE lives in (the
    /// embedding job passes `context.workspace_id`), so an administrative
    /// operation — rebuild, verify, health — cannot know the set of workspaces
    /// it has to cover without asking. Every such operation used to name a
    /// workspace with a hardcoded literal instead (`"staff"` in the HTTP
    /// management path, `"default"` in the SQL one), which made it a silent
    /// no-op for every deployment whose content lived anywhere else.
    ///
    /// Returns the workspace ids in first-seen (byte-sorted) order.
    fn list_workspaces(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<Vec<String>>;

    /// Read ONE stored vector, addressed exactly: which embedding space, which
    /// source, which chunk.
    ///
    /// [`Self::get_embedding`] cannot do this, and the difference is not a
    /// nicety. It takes a bare `node_id` and no embedder, so it scans the whole
    /// workspace prefix and answers with whichever embedding space's row it
    /// meets FIRST — on a branch with a text and an image partition, that is
    /// decided by base64 sort order, not by what the caller asked for. And it
    /// takes no chunk index, so for a chunked document it always answers with
    /// chunk 0 — silently, which is exactly the "do NOT pick chunk 0" case.
    ///
    /// `source_id` is the NAMESPACED id (`{node}` for the default spec,
    /// `{node}#{spec}` for a named one), built with
    /// `raisin_hnsw::namespaced_source_id`. It is not the node id when a named
    /// spec is in play, and passing the node id there reads the wrong vector.
    ///
    /// `revision` of `None` means the latest.
    #[allow(clippy::too_many_arguments)]
    fn get_source_chunk(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        embedder_hash: &str,
        kind: char,
        source_id: &str,
        chunk_idx: usize,
        revision: Option<&HLC>,
    ) -> Result<Option<EmbeddingData>>;

    /// The chunk indexes stored for one source in one embedding space, at the
    /// LATEST revision of each, ascending and deduplicated.
    ///
    /// This is what makes "similar to this node" decidable instead of
    /// ambiguous. One index means one vector and the question has an answer.
    /// Several means the caller has to say WHICH — a document's chunks are
    /// different vectors, chunk 0 is an arbitrary one to pick, and a centroid
    /// of a multi-topic document is a point that resembles none of its parts.
    /// A caller that cannot enumerate them can only guess.
    #[allow(clippy::too_many_arguments)]
    fn list_source_chunk_indexes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        embedder_hash: &str,
        kind: char,
        source_id: &str,
    ) -> Result<Vec<usize>>;
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
