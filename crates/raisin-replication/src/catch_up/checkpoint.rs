//! RocksDB checkpoint transfer (Phase 5).
//!
//! Streams RocksDB SST files from the source peer with chunked
//! transfer, CRC32 verification, and acknowledgment protocol.

use super::types::{CatchUpSession, CheckpointTransferResult, PeerStatus};
use super::CatchUpCoordinator;
use crate::ReplicationMessage;
use raisin_error::{Error, Result};
use tokio::net::TcpStream;
use tracing::info;

impl CatchUpCoordinator {
    /// Phase 5: Transfer RocksDB checkpoint (SST files)
    pub(super) async fn transfer_checkpoint(
        &self,
        source_peer: &PeerStatus,
        session: &CatchUpSession,
    ) -> Result<CheckpointTransferResult> {
        info!(
            session_id = %session.session_id,
            "Phase 5: Transferring RocksDB checkpoint"
        );

        let start_time = std::time::Instant::now();

        // Get connection
        let mut connections = self.peer_connections.write().await;
        let stream = connections
            .get_mut(&source_peer.node_id)
            .ok_or_else(|| Error::Backend("Source peer connection lost".to_string()))?;

        // Send RequestCheckpoint
        let request = ReplicationMessage::RequestCheckpoint {
            snapshot_id: session.session_id.clone(),
            max_parallel_files: 4,
        };

        Self::send_message(stream, &request).await?;

        // Receive CheckpointMetadata
        let response = Self::receive_message(stream).await?;

        let (snapshot_id, sst_files, _column_families) = match response {
            ReplicationMessage::CheckpointMetadata {
                snapshot_id,
                sst_files,
                total_size_bytes,
                column_families,
            } => {
                info!(
                    snapshot_id = %snapshot_id,
                    num_files = sst_files.len(),
                    total_size_mb = total_size_bytes / 1_048_576,
                    "Checkpoint metadata received"
                );
                (snapshot_id, sst_files, column_families)
            }
            _ => {
                return Err(Error::Backend(
                    "Unexpected response to RequestCheckpoint".to_string(),
                ))
            }
        };

        // Create staging directory
        let staging_path = self.staging_dir.join(&snapshot_id);
        tokio::fs::create_dir_all(&staging_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to create staging directory: {}", e)))?;

        // Receive SST file chunks
        let num_files = sst_files.len();
        let mut total_bytes = 0u64;

        for file_info in &sst_files {
            let file_path = staging_path.join(&file_info.file_name);

            info!(
                file_name = %file_info.file_name,
                size_mb = file_info.size_bytes / 1_048_576,
                "Receiving SST file"
            );

            // Handle empty files specially - create empty file and skip chunk loop
            if file_info.size_bytes == 0 {
                use tokio::fs::File;
                File::create(&file_path)
                    .await
                    .map_err(|e| Error::Backend(format!("Failed to create empty file: {}", e)))?;

                info!(
                    file_name = %file_info.file_name,
                    "Created empty file (0 bytes)"
                );

                total_bytes += 0;
                continue; // Move to next file
            }

            // Receive file chunks directly from network
            // TODO: In a full implementation, this should use ParallelTransferOrchestrator
            // for concurrent file transfers. For now, we receive files sequentially.

            use tokio::fs::File;
            use tokio::io::AsyncWriteExt;

            let mut file = File::create(&file_path)
                .await
                .map_err(|e| Error::Backend(format!("Failed to create file: {}", e)))?;

            let mut expected_chunk_index = 0u32;
            let mut total_chunks_expected = None;
            let mut hasher = crc32fast::Hasher::new();

            loop {
                // Receive SstFileChunk message from network
                let message = Self::receive_message(stream).await?;

                match message {
                    ReplicationMessage::SstFileChunk {
                        snapshot_id: received_snapshot_id,
                        file_name,
                        chunk_index,
                        total_chunks,
                        data,
                        chunk_crc32,
                    } => {
                        // Verify this is the right file
                        if received_snapshot_id != snapshot_id || file_name != file_info.file_name {
                            return Err(Error::Backend(format!(
                                "Received chunk for wrong file: expected snapshot {} file {}, got snapshot {} file {}",
                                snapshot_id, file_info.file_name, received_snapshot_id, file_name
                            )));
                        }

                        // Verify chunk index sequence
                        if chunk_index != expected_chunk_index {
                            return Err(Error::Backend(format!(
                                "Chunk index out of order: expected {}, got {}",
                                expected_chunk_index, chunk_index
                            )));
                        }

                        // Verify chunk CRC32
                        let calculated_crc32 = {
                            let mut chunk_hasher = crc32fast::Hasher::new();
                            chunk_hasher.update(&data);
                            chunk_hasher.finalize()
                        };

                        let status = if calculated_crc32 == chunk_crc32 {
                            // Write chunk to file
                            file.write_all(&data).await.map_err(|e| {
                                Error::Backend(format!("Failed to write chunk: {}", e))
                            })?;

                            // Update file hasher
                            hasher.update(&data);

                            crate::TransferStatus::Success
                        } else {
                            crate::TransferStatus::ChecksumMismatch
                        };

                        // Send acknowledgment
                        let ack_message = ReplicationMessage::SstFileChunkAck {
                            file_name,
                            chunk_index,
                            status,
                        };
                        Self::send_message(stream, &ack_message).await?;

                        if status != crate::TransferStatus::Success {
                            return Err(Error::Backend(format!(
                                "Chunk {} checksum mismatch",
                                chunk_index
                            )));
                        }

                        // Track expected total chunks
                        if total_chunks_expected.is_none() {
                            total_chunks_expected = Some(total_chunks);
                        }

                        expected_chunk_index += 1;

                        // Check if this was the last chunk
                        if expected_chunk_index >= total_chunks {
                            break;
                        }
                    }
                    _ => {
                        return Err(Error::Backend(
                            "Unexpected message during file transfer".to_string(),
                        ));
                    }
                }
            }

            // Flush file to disk
            file.flush()
                .await
                .map_err(|e| Error::Backend(format!("Failed to flush file: {}", e)))?;

            // Verify final file CRC32
            let calculated_file_crc32 = hasher.finalize();
            if calculated_file_crc32 != file_info.crc32 {
                // Delete corrupted file
                let _ = tokio::fs::remove_file(&file_path).await;
                return Err(Error::Backend(format!(
                    "File CRC32 mismatch for {}: expected {}, got {}",
                    file_info.file_name, file_info.crc32, calculated_file_crc32
                )));
            }

            total_bytes += file_info.size_bytes;

            info!(
                file_name = %file_info.file_name,
                "SST file received and verified"
            );
        }

        let duration = start_time.elapsed();

        Ok(CheckpointTransferResult {
            num_files,
            total_bytes,
            duration,
        })
    }
}
