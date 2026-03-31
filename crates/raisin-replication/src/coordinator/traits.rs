//! Trait definitions for replication storage, checkpoint, and index operations.
//!
//! These traits define the abstractions that allow the coordinator to work
//! with any storage backend (RocksDB, PostgreSQL, etc.) and handle checkpoint
//! and index transfer operations during cluster catch-up.

use async_trait::async_trait;

use super::types::{ClusterStorageStats, CoordinatorError, StorageError};
use crate::{Operation, VectorClock};

/// Trait that storage backends must implement to support replication
///
/// This allows the coordinator to work with any storage backend (RocksDB, PostgreSQL, etc.)
#[async_trait]
pub trait OperationLogStorage: Send + Sync {
    /// Get operations that are newer than the provided vector clock
    ///
    /// This is used during pull-based sync to fetch missing operations from a peer.
    async fn get_operations_since(
        &self,
        tenant_id: &str,
        repo_id: &str,
        since_vc: &VectorClock,
        limit: usize,
    ) -> Result<Vec<Operation>, StorageError>;

    /// Store a batch of operations atomically
    ///
    /// Operations should be applied using CRDT merge rules before storage.
    async fn put_operations_batch(&self, ops: &[Operation]) -> Result<(), StorageError>;

    /// Get the current vector clock for a repository
    async fn get_vector_clock(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<VectorClock, StorageError>;

    /// Get all operations for a specific peer node (for pull requests)
    async fn get_operations_for_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        since_seq: u64,
        limit: usize,
    ) -> Result<Vec<Operation>, StorageError>;

    /// Get cluster-wide storage statistics
    ///
    /// Returns aggregated information across all tenant/repo pairs:
    /// - Aggregated vector clock (max across all tenants/repos)
    /// - Number of unique tenants
    /// - Number of repositories
    /// - List of all (tenant_id, repo_id) pairs
    async fn get_cluster_stats(&self) -> Result<ClusterStorageStats, StorageError>;
}

/// Trait for serving RocksDB checkpoints during cluster catch-up
///
/// This allows storage-agnostic ReplicationServer to delegate checkpoint
/// serving to storage-specific implementations (e.g., CheckpointServer for RocksDB).
#[async_trait]
pub trait CheckpointProvider: Send + Sync {
    /// Handle a checkpoint request from a peer during catch-up
    ///
    /// This should:
    /// 1. Create an atomic checkpoint (snapshot) of the database
    /// 2. Send checkpoint metadata (list of SST files with CRC32 checksums)
    /// 3. Stream all SST files in chunks with acknowledgment
    /// 4. Handle chunk verification and retransmission on checksum failures
    ///
    /// # Arguments
    /// * `stream` - TCP stream to send checkpoint data on
    /// * `snapshot_id` - Unique identifier for this checkpoint session
    /// * `max_parallel_files` - Maximum number of files to stream in parallel
    ///
    /// # Returns
    /// Ok(()) if checkpoint was successfully served, error otherwise
    async fn handle_checkpoint_request(
        &self,
        stream: &mut tokio::net::TcpStream,
        snapshot_id: &str,
        max_parallel_files: u8,
    ) -> Result<(), CoordinatorError>;

    /// Handle a request for list of available Tantivy fulltext indexes
    ///
    /// This should:
    /// 1. List all available Tantivy indexes (tenant_id, repo_id, branch tuples)
    /// 2. Send TantivyIndexList response message
    ///
    /// # Arguments
    /// * `stream` - TCP stream to send response on
    ///
    /// # Returns
    /// Ok(()) if list was successfully sent, error otherwise
    async fn handle_tantivy_index_list_request(
        &self,
        stream: &mut tokio::net::TcpStream,
    ) -> Result<(), CoordinatorError>;

    /// Handle a request for list of available HNSW vector indexes
    ///
    /// This should:
    /// 1. List all available HNSW indexes (tenant_id, repo_id, branch tuples)
    /// 2. Send HnswIndexList response message
    ///
    /// # Arguments
    /// * `stream` - TCP stream to send response on
    ///
    /// # Returns
    /// Ok(()) if list was successfully sent, error otherwise
    async fn handle_hnsw_index_list_request(
        &self,
        stream: &mut tokio::net::TcpStream,
    ) -> Result<(), CoordinatorError>;

    /// Handle a request to transfer a specific Tantivy index
    ///
    /// This should:
    /// 1. Collect index metadata (list of index files with checksums)
    /// 2. Send TantivyIndexMetadata response
    /// 3. Stream all index files in chunks with CRC32 verification
    ///
    /// # Arguments
    /// * `stream` - TCP stream to send index data on
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    ///
    /// # Returns
    /// Ok(()) if index was successfully transferred, error otherwise
    async fn handle_tantivy_index_request(
        &self,
        stream: &mut tokio::net::TcpStream,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), CoordinatorError>;

    /// Handle a request to transfer a specific HNSW vector index
    ///
    /// This should:
    /// 1. Serialize the HNSW index
    /// 2. Calculate CRC32 checksum
    /// 3. Send HnswIndexData response with serialized bytes
    ///
    /// # Arguments
    /// * `stream` - TCP stream to send index data on
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    ///
    /// # Returns
    /// Ok(()) if index was successfully transferred, error otherwise
    async fn handle_hnsw_index_request(
        &self,
        stream: &mut tokio::net::TcpStream,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), CoordinatorError>;
}

/// Trait for ingesting received checkpoints into the local database
///
/// This allows storage-agnostic CatchUpCoordinator to delegate checkpoint
/// ingestion to storage-specific implementations (e.g., RocksDB ingest_external_file).
#[async_trait]
pub trait CheckpointIngestor: Send + Sync {
    /// Ingest received checkpoint files into the local database
    ///
    /// This should:
    /// 1. Validate the checkpoint files (verify CRC32 checksums)
    /// 2. Use storage-specific APIs to ingest the files (e.g., RocksDB ingest_external_file)
    /// 3. Ensure atomic ingestion (all files or none)
    /// 4. Clean up temporary checkpoint files after successful ingestion
    ///
    /// # Arguments
    /// * `checkpoint_dir` - Directory containing the received checkpoint files
    /// * `snapshot_id` - Unique identifier for this checkpoint
    ///
    /// # Returns
    /// Ok(num_files_ingested) if successful, error otherwise
    async fn ingest_checkpoint(
        &self,
        checkpoint_dir: &std::path::Path,
        snapshot_id: &str,
    ) -> Result<usize, CoordinatorError>;
}

/// Trait for listing available indexes on a node
///
/// This allows CatchUpCoordinator to discover which indexes need to be transferred
#[async_trait]
pub trait IndexLister: Send + Sync {
    /// List all tenant/repo/branch combinations that have Tantivy indexes
    async fn list_tantivy_indexes(&self)
        -> Result<Vec<(String, String, String)>, CoordinatorError>;

    /// List all tenant/repo/branch combinations that have HNSW indexes
    async fn list_hnsw_indexes(&self) -> Result<Vec<(String, String, String)>, CoordinatorError>;
}

/// Trait for receiving and ingesting Tantivy indexes during catch-up
#[async_trait]
pub trait TantivyIndexReceiver: Send + Sync {
    /// Prepare staging directory for receiving index
    ///
    /// # Returns
    /// Path to staging directory where index files should be written
    async fn prepare_receive(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<std::path::PathBuf, CoordinatorError>;

    /// Verify received index files match expected metadata
    async fn verify_index(
        &self,
        staging_path: &std::path::Path,
        expected_files: &[crate::IndexFileInfo],
    ) -> Result<(), CoordinatorError>;

    /// Ingest verified index into permanent location
    async fn ingest_index(
        &self,
        staging_path: &std::path::Path,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), CoordinatorError>;

    /// Abort receive and clean up staging directory
    async fn abort_receive(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), CoordinatorError>;
}

/// Trait for receiving and ingesting HNSW indexes during catch-up
#[async_trait]
pub trait HnswIndexReceiver: Send + Sync {
    /// Receive complete HNSW index data
    ///
    /// # Returns
    /// Path to staging file
    async fn receive_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        data: Vec<u8>,
        expected_crc32: u32,
    ) -> Result<std::path::PathBuf, CoordinatorError>;

    /// Ingest verified index into permanent location
    async fn ingest_index(
        &self,
        staging_path: &std::path::Path,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), CoordinatorError>;

    /// Abort receive and clean up staging file
    async fn abort_receive(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), CoordinatorError>;
}
