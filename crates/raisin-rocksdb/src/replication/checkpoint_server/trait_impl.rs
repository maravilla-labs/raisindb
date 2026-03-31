//! CheckpointProvider trait implementation for CheckpointServer.
//!
//! Allows CheckpointServer to be used as a pluggable checkpoint provider
//! for the storage-agnostic ReplicationServer.

use super::CheckpointServer;

#[async_trait::async_trait]
impl raisin_replication::CheckpointProvider for CheckpointServer {
    async fn handle_checkpoint_request(
        &self,
        stream: &mut tokio::net::TcpStream,
        snapshot_id: &str,
        max_parallel_files: u8,
    ) -> Result<(), raisin_replication::CoordinatorError> {
        // Delegate to the existing implementation method
        self.handle_checkpoint_request(stream, snapshot_id, max_parallel_files)
            .await
            .map_err(|e| {
                raisin_replication::CoordinatorError::Storage(format!(
                    "Checkpoint serving failed: {}",
                    e
                ))
            })
    }

    async fn handle_tantivy_index_list_request(
        &self,
        stream: &mut tokio::net::TcpStream,
    ) -> Result<(), raisin_replication::CoordinatorError> {
        self.handle_tantivy_index_list_request(stream)
            .await
            .map_err(|e| {
                raisin_replication::CoordinatorError::Storage(format!(
                    "Tantivy index list request failed: {}",
                    e
                ))
            })
    }

    async fn handle_hnsw_index_list_request(
        &self,
        stream: &mut tokio::net::TcpStream,
    ) -> Result<(), raisin_replication::CoordinatorError> {
        self.handle_hnsw_index_list_request(stream)
            .await
            .map_err(|e| {
                raisin_replication::CoordinatorError::Storage(format!(
                    "HNSW index list request failed: {}",
                    e
                ))
            })
    }

    async fn handle_tantivy_index_request(
        &self,
        stream: &mut tokio::net::TcpStream,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), raisin_replication::CoordinatorError> {
        self.handle_tantivy_index_request(stream, tenant_id, repo_id, branch)
            .await
            .map_err(|e| {
                raisin_replication::CoordinatorError::Storage(format!(
                    "Tantivy index transfer failed: {}",
                    e
                ))
            })
    }

    async fn handle_hnsw_index_request(
        &self,
        stream: &mut tokio::net::TcpStream,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), raisin_replication::CoordinatorError> {
        self.handle_hnsw_index_request(stream, tenant_id, repo_id, branch)
            .await
            .map_err(|e| {
                raisin_replication::CoordinatorError::Storage(format!(
                    "HNSW index transfer failed: {}",
                    e
                ))
            })
    }
}
