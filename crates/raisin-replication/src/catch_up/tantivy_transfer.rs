//! Tantivy fulltext index transfer.
//!
//! Handles chunked streaming of Tantivy index files with CRC32 verification,
//! staging, and ingestion into the local Tantivy instance.

use super::CatchUpCoordinator;
use crate::{IndexFileInfo, ReplicationMessage};
use raisin_error::{Error, Result};
use std::path::{Path, PathBuf};
use tokio::net::TcpStream;
use tracing::info;

impl CatchUpCoordinator {
    /// Transfer a single Tantivy index with chunked streaming
    pub(super) async fn transfer_single_tantivy_index(
        &self,
        stream: &mut TcpStream,
        receiver: &dyn crate::TantivyIndexReceiver,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<usize> {
        // Request index metadata
        let request = ReplicationMessage::RequestTantivyIndex {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
        };
        Self::send_message(stream, &request).await?;

        // Receive metadata
        let metadata_response = Self::receive_message(stream).await?;

        let (files, total_size) = match metadata_response {
            ReplicationMessage::TantivyIndexMetadata {
                tenant_id: _,
                repo_id: _,
                branch: _,
                files,
                total_size_bytes,
            } => {
                info!(
                    num_files = files.len(),
                    total_size_mb = total_size_bytes / 1_048_576,
                    "Tantivy index metadata received"
                );
                (files, total_size_bytes)
            }
            _ => {
                return Err(Error::Backend(
                    "Unexpected response to RequestTantivyIndex".to_string(),
                ))
            }
        };

        if files.is_empty() {
            info!("No Tantivy index files to transfer");
            return Ok(0);
        }

        // Prepare staging directory
        let staging_path = receiver
            .prepare_receive(tenant_id, repo_id, branch)
            .await
            .map_err(|e| Error::Backend(format!("Failed to prepare staging: {}", e)))?;

        // Receive each file
        for file_info in &files {
            info!(
                file_name = %file_info.file_name,
                size_mb = file_info.size_bytes / 1_048_576,
                "Receiving Tantivy file"
            );

            self.receive_tantivy_file(stream, &staging_path, file_info)
                .await?;
        }

        // Verify all files
        receiver
            .verify_index(&staging_path, &files)
            .await
            .map_err(|e| Error::Backend(format!("Index verification failed: {}", e)))?;

        // Ingest index
        receiver
            .ingest_index(&staging_path, tenant_id, repo_id, branch)
            .await
            .map_err(|e| Error::Backend(format!("Index ingestion failed: {}", e)))?;

        Ok(files.len())
    }

    /// Receive a single Tantivy file with chunked streaming
    async fn receive_tantivy_file(
        &self,
        stream: &mut TcpStream,
        staging_path: &Path,
        file_info: &IndexFileInfo,
    ) -> Result<()> {
        use tokio::fs::File;
        use tokio::io::AsyncWriteExt;

        let file_path = staging_path.join(&file_info.file_name);

        // Handle empty files
        if file_info.size_bytes == 0 {
            File::create(&file_path)
                .await
                .map_err(|e| Error::Backend(format!("Failed to create empty file: {}", e)))?;
            return Ok(());
        }

        let mut file = File::create(&file_path)
            .await
            .map_err(|e| Error::Backend(format!("Failed to create file: {}", e)))?;

        let mut expected_chunk_index = 0u32;
        let mut hasher = crc32fast::Hasher::new();

        loop {
            // Receive chunk
            let message = Self::receive_message(stream).await?;

            match message {
                ReplicationMessage::TantivyFileChunk {
                    tenant_id: _,
                    repo_id: _,
                    branch: _,
                    file_name,
                    chunk_index,
                    total_chunks,
                    data,
                    chunk_crc32,
                } => {
                    // Verify chunk belongs to this file
                    if file_name != file_info.file_name {
                        return Err(Error::Backend(format!(
                            "Received chunk for wrong file: expected {}, got {}",
                            file_info.file_name, file_name
                        )));
                    }

                    // Verify chunk sequence
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
                        file.write_all(&data)
                            .await
                            .map_err(|e| Error::Backend(format!("Failed to write chunk: {}", e)))?;
                        hasher.update(&data);
                        crate::TransferStatus::Success
                    } else {
                        crate::TransferStatus::ChecksumMismatch
                    };

                    // Send acknowledgment
                    let ack = ReplicationMessage::TantivyFileChunkAck {
                        file_name,
                        chunk_index,
                        status,
                    };
                    Self::send_message(stream, &ack).await?;

                    if status != crate::TransferStatus::Success {
                        return Err(Error::Backend(format!(
                            "Chunk {} checksum mismatch",
                            chunk_index
                        )));
                    }

                    expected_chunk_index += 1;

                    if expected_chunk_index >= total_chunks {
                        break;
                    }
                }
                _ => {
                    return Err(Error::Backend(
                        "Unexpected message during Tantivy file transfer".to_string(),
                    ));
                }
            }
        }

        // Flush and verify final file CRC32
        file.flush()
            .await
            .map_err(|e| Error::Backend(format!("Failed to flush file: {}", e)))?;

        let calculated_file_crc32 = hasher.finalize();
        if calculated_file_crc32 != file_info.crc32 {
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err(Error::Backend(format!(
                "File CRC32 mismatch for {}: expected {}, got {}",
                file_info.file_name, file_info.crc32, calculated_file_crc32
            )));
        }

        Ok(())
    }
}
