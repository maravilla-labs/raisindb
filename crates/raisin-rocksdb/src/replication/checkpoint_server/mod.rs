//! Checkpoint server for serving RocksDB snapshots to catch-up nodes
//!
//! This module provides server-side functionality for the P2P cluster catch-up protocol,
//! specifically handling checkpoint creation and SST file streaming.

mod index_transfer;
#[cfg(test)]
mod tests;
mod trait_impl;

use crate::checkpoint::{CheckpointManager, CheckpointMetadata};
use raisin_error::{Error, Result};
use raisin_replication::{ReplicationMessage, SstFileInfo};
use rocksdb::DB;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tracing::{error, info};

/// Checkpoint server for serving RocksDB snapshots
pub struct CheckpointServer {
    /// Checkpoint manager for creating snapshots
    pub(super) checkpoint_manager: Arc<CheckpointManager>,

    /// Node ID
    #[allow(dead_code)]
    pub(super) node_id: String,

    /// Optional Tantivy index manager for serving fulltext indexes
    pub(super) tantivy_manager: Option<Arc<crate::TantivyIndexManager>>,

    /// Optional HNSW index manager for serving vector indexes
    pub(super) hnsw_manager: Option<Arc<crate::HnswIndexManager>>,
}

impl CheckpointServer {
    /// Create a new checkpoint server
    ///
    /// # Arguments
    /// * `db` - RocksDB database instance
    /// * `checkpoint_dir` - Directory where checkpoints are stored
    /// * `node_id` - This node's cluster ID
    /// * `tantivy_manager` - Optional Tantivy index manager
    /// * `hnsw_manager` - Optional HNSW index manager
    pub fn new(
        db: Arc<DB>,
        checkpoint_dir: PathBuf,
        node_id: String,
        tantivy_manager: Option<Arc<crate::TantivyIndexManager>>,
        hnsw_manager: Option<Arc<crate::HnswIndexManager>>,
    ) -> Self {
        let checkpoint_manager = Arc::new(CheckpointManager::new(db, checkpoint_dir));

        Self {
            checkpoint_manager,
            node_id,
            tantivy_manager,
            hnsw_manager,
        }
    }

    /// Handle a RequestCheckpoint message
    ///
    /// Creates a checkpoint and streams SST files to the requesting node
    pub async fn handle_checkpoint_request(
        &self,
        stream: &mut TcpStream,
        snapshot_id: &str,
        max_parallel_files: u8,
    ) -> Result<()> {
        info!(
            snapshot_id = %snapshot_id,
            max_parallel = max_parallel_files,
            "Handling checkpoint request"
        );

        // Create checkpoint
        let metadata = self
            .checkpoint_manager
            .create_checkpoint(snapshot_id)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to create checkpoint");
                e
            })?;

        // Send checkpoint metadata
        self.send_checkpoint_metadata(stream, &metadata).await?;

        // Stream all SST files
        self.stream_sst_files(stream, &metadata, max_parallel_files)
            .await?;

        info!(
            snapshot_id = %snapshot_id,
            num_files = metadata.sst_files.len(),
            "Checkpoint streaming completed"
        );

        Ok(())
    }

    /// Send checkpoint metadata to requesting node
    async fn send_checkpoint_metadata(
        &self,
        stream: &mut TcpStream,
        metadata: &CheckpointMetadata,
    ) -> Result<()> {
        let message = ReplicationMessage::CheckpointMetadata {
            snapshot_id: metadata.snapshot_id.clone(),
            sst_files: metadata.sst_files.clone(),
            total_size_bytes: metadata.total_size_bytes,
            column_families: metadata.column_families.clone(),
        };

        raisin_replication::tcp_helpers::send_message(stream, &message).await
    }

    /// Stream SST files to requesting node
    ///
    /// Sends files sequentially with chunking and CRC32 verification
    async fn stream_sst_files(
        &self,
        stream: &mut TcpStream,
        metadata: &CheckpointMetadata,
        _max_parallel_files: u8,
    ) -> Result<()> {
        // TODO: Implement parallel file streaming using max_parallel_files
        // For now, stream files sequentially

        for file_info in &metadata.sst_files {
            self.stream_single_file(stream, metadata, file_info)
                .await
                .map_err(|e| {
                    error!(
                        file_name = %file_info.file_name,
                        error = %e,
                        "Failed to stream SST file"
                    );
                    e
                })?;
        }

        Ok(())
    }

    /// Stream a single SST file with chunking
    async fn stream_single_file(
        &self,
        stream: &mut TcpStream,
        metadata: &CheckpointMetadata,
        file_info: &SstFileInfo,
    ) -> Result<()> {
        let file_path = metadata.checkpoint_path.join(&file_info.file_name);

        info!(
            file_name = %file_info.file_name,
            size_mb = file_info.size_bytes / 1_048_576,
            "Streaming SST file"
        );

        // Open file
        let mut file = File::open(&file_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to open SST file: {}", e)))?;

        // Chunk size: 1MB
        const CHUNK_SIZE: usize = 1_048_576;

        let mut buffer = vec![0u8; CHUNK_SIZE];
        let mut chunk_index = 0u32;
        let total_chunks = file_info.size_bytes.div_ceil(CHUNK_SIZE as u64) as u32;

        loop {
            // Read chunk
            let bytes_read = file
                .read(&mut buffer)
                .await
                .map_err(|e| Error::storage(format!("Failed to read file chunk: {}", e)))?;

            if bytes_read == 0 {
                break; // EOF
            }

            let chunk_data = buffer[..bytes_read].to_vec();

            // Calculate chunk CRC32
            let mut hasher = crc32fast::Hasher::new();
            hasher.update(&chunk_data);
            let chunk_crc32 = hasher.finalize();

            // Send chunk message
            let chunk_message = ReplicationMessage::SstFileChunk {
                snapshot_id: metadata.snapshot_id.clone(),
                file_name: file_info.file_name.clone(),
                chunk_index,
                total_chunks,
                data: chunk_data,
                chunk_crc32,
            };

            raisin_replication::tcp_helpers::send_message(stream, &chunk_message).await?;

            // Wait for acknowledgment
            let ack_message = raisin_replication::tcp_helpers::receive_message(stream).await?;

            match ack_message {
                ReplicationMessage::SstFileChunkAck {
                    file_name: ack_file_name,
                    chunk_index: ack_chunk_index,
                    status,
                } => {
                    // Verify ACK matches what we sent
                    if ack_file_name != file_info.file_name || ack_chunk_index != chunk_index {
                        return Err(Error::storage(format!(
                            "ACK mismatch: expected {}:{}, got {}:{}",
                            file_info.file_name, chunk_index, ack_file_name, ack_chunk_index
                        )));
                    }

                    // Check status
                    match status {
                        raisin_replication::TransferStatus::Success => {
                            // Continue to next chunk
                        }
                        raisin_replication::TransferStatus::ChecksumMismatch => {
                            // TODO: Implement retry logic
                            return Err(Error::storage(format!(
                                "Chunk {} checksum mismatch reported by peer",
                                chunk_index
                            )));
                        }
                        _ => {
                            return Err(Error::storage(format!(
                                "Chunk {} transfer failed: {:?}",
                                chunk_index, status
                            )));
                        }
                    }
                }
                _ => {
                    return Err(Error::storage(
                        "Expected SstFileChunkAck, got different message".to_string(),
                    ));
                }
            }

            chunk_index += 1;

            if chunk_index >= total_chunks {
                break;
            }
        }

        info!(
            file_name = %file_info.file_name,
            chunks = chunk_index,
            "SST file streaming completed"
        );

        Ok(())
    }

    /// Clean up old checkpoints, keeping only the latest N
    pub async fn cleanup_old_checkpoints(&self, keep_latest: usize) -> Result<usize> {
        self.checkpoint_manager
            .cleanup_old_checkpoints(keep_latest)
            .await
    }
}
