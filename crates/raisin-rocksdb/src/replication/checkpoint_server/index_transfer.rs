//! Index transfer handlers for Tantivy and HNSW indexes.
//!
//! Handles listing and streaming of fulltext (Tantivy) and vector (HNSW) indexes
//! to requesting nodes during catch-up replication.

use super::CheckpointServer;
use raisin_error::{Error, Result};
use raisin_replication::ReplicationMessage;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tracing::{info, warn};

impl CheckpointServer {
    /// Handle request for list of available Tantivy indexes
    pub async fn handle_tantivy_index_list_request(&self, stream: &mut TcpStream) -> Result<()> {
        info!("Handling Tantivy index list request");

        let indexes = if let Some(ref manager) = self.tantivy_manager {
            manager
                .list_all_indexes()
                .await
                .map_err(|e| Error::storage(format!("Failed to list Tantivy indexes: {}", e)))?
        } else {
            warn!("No Tantivy manager configured");
            Vec::new()
        };

        let response = ReplicationMessage::TantivyIndexList { indexes };
        raisin_replication::tcp_helpers::send_message(stream, &response).await?;

        Ok(())
    }

    /// Handle request for list of available HNSW indexes
    pub async fn handle_hnsw_index_list_request(&self, stream: &mut TcpStream) -> Result<()> {
        info!("Handling HNSW index list request");

        let indexes = if let Some(ref manager) = self.hnsw_manager {
            manager
                .list_all_indexes()
                .await
                .map_err(|e| Error::storage(format!("Failed to list HNSW indexes: {}", e)))?
        } else {
            warn!("No HNSW manager configured");
            Vec::new()
        };

        let response = ReplicationMessage::HnswIndexList { indexes };
        raisin_replication::tcp_helpers::send_message(stream, &response).await?;

        Ok(())
    }

    /// Handle request for a specific Tantivy index
    pub async fn handle_tantivy_index_request(
        &self,
        stream: &mut TcpStream,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<()> {
        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            "Handling Tantivy index request"
        );

        let manager = self
            .tantivy_manager
            .as_ref()
            .ok_or_else(|| Error::storage("No Tantivy manager configured".to_string()))?;

        // Collect index metadata
        let metadata = manager
            .collect_index_metadata(tenant_id, repo_id, branch)
            .await?;

        // Send metadata
        let metadata_msg = ReplicationMessage::TantivyIndexMetadata {
            tenant_id: metadata.tenant_id.clone(),
            repo_id: metadata.repo_id.clone(),
            branch: metadata.branch.clone(),
            files: metadata.files.clone(),
            total_size_bytes: metadata.total_size_bytes,
        };
        raisin_replication::tcp_helpers::send_message(stream, &metadata_msg).await?;

        // Stream each file
        let index_dir = manager.get_index_dir(tenant_id, repo_id, branch);
        for file_info in &metadata.files {
            self.stream_tantivy_file(stream, &index_dir, tenant_id, repo_id, branch, file_info)
                .await?;
        }

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            num_files = metadata.files.len(),
            "Tantivy index transfer complete"
        );

        Ok(())
    }

    /// Stream a single Tantivy file with chunking
    async fn stream_tantivy_file(
        &self,
        stream: &mut TcpStream,
        index_dir: &std::path::Path,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        file_info: &raisin_replication::IndexFileInfo,
    ) -> Result<()> {
        let file_path = index_dir.join(&file_info.file_name);

        info!(
            file_name = %file_info.file_name,
            size_mb = file_info.size_bytes / 1_048_576,
            "Streaming Tantivy file"
        );

        let mut file = File::open(&file_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to open file: {}", e)))?;

        const CHUNK_SIZE: usize = 1_048_576; // 1MB chunks
        let mut buffer = vec![0u8; CHUNK_SIZE];
        let mut chunk_index = 0u32;
        let total_chunks = file_info.size_bytes.div_ceil(CHUNK_SIZE as u64) as u32;

        loop {
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

            // Send chunk
            let chunk_msg = ReplicationMessage::TantivyFileChunk {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                branch: branch.to_string(),
                file_name: file_info.file_name.clone(),
                chunk_index,
                total_chunks,
                data: chunk_data,
                chunk_crc32,
            };
            raisin_replication::tcp_helpers::send_message(stream, &chunk_msg).await?;

            // Wait for acknowledgment
            let ack_msg = raisin_replication::tcp_helpers::receive_message(stream).await?;

            match ack_msg {
                ReplicationMessage::TantivyFileChunkAck {
                    file_name: ack_file_name,
                    chunk_index: ack_chunk_index,
                    status,
                } => {
                    if ack_file_name != file_info.file_name || ack_chunk_index != chunk_index {
                        return Err(Error::storage(format!(
                            "ACK mismatch: expected {}:{}, got {}:{}",
                            file_info.file_name, chunk_index, ack_file_name, ack_chunk_index
                        )));
                    }

                    match status {
                        raisin_replication::TransferStatus::Success => {
                            // Continue
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
                    return Err(Error::storage("Expected TantivyFileChunkAck".to_string()));
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
            "Tantivy file streaming completed"
        );

        Ok(())
    }

    /// Handle request for a specific HNSW index
    pub async fn handle_hnsw_index_request(
        &self,
        stream: &mut TcpStream,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<()> {
        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            "Handling HNSW index request"
        );

        let manager = self
            .hnsw_manager
            .as_ref()
            .ok_or_else(|| Error::storage("No HNSW manager configured".to_string()))?;

        // Load index data
        let index_data = manager.load_index_data(tenant_id, repo_id, branch).await?;

        match index_data {
            Some((data, crc32)) => {
                info!(size_mb = data.len() / 1_048_576, "Sending HNSW index data");

                // Send index data
                let data_msg = ReplicationMessage::HnswIndexData {
                    tenant_id: tenant_id.to_string(),
                    repo_id: repo_id.to_string(),
                    branch: branch.to_string(),
                    data,
                    crc32,
                };
                raisin_replication::tcp_helpers::send_message(stream, &data_msg).await?;

                // Wait for acknowledgment
                let ack_msg = raisin_replication::tcp_helpers::receive_message(stream).await?;

                match ack_msg {
                    ReplicationMessage::HnswIndexAck { status } => {
                        if status != raisin_replication::TransferStatus::Success {
                            return Err(Error::storage(format!(
                                "HNSW index transfer failed: {:?}",
                                status
                            )));
                        }
                    }
                    _ => {
                        return Err(Error::storage("Expected HnswIndexAck".to_string()));
                    }
                }

                info!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    branch = %branch,
                    "HNSW index transfer complete"
                );
            }
            None => {
                warn!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    branch = %branch,
                    "No HNSW index found, sending empty data"
                );

                // Send empty data
                let data_msg = ReplicationMessage::HnswIndexData {
                    tenant_id: tenant_id.to_string(),
                    repo_id: repo_id.to_string(),
                    branch: branch.to_string(),
                    data: Vec::new(),
                    crc32: 0,
                };
                raisin_replication::tcp_helpers::send_message(stream, &data_msg).await?;
            }
        }

        Ok(())
    }
}
