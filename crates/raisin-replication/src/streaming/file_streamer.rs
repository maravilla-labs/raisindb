//! Reliable file streamer with CRC32 verification
//!
//! Streams large files in chunks with checksum verification and retry logic.

use crc32fast::Hasher;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::tcp_protocol::TransferStatus;

use super::types::{ChunkAck, FileChunk, StreamError, DEFAULT_CHUNK_SIZE, MAX_CHUNK_RETRIES};

/// Reliable file streamer with CRC32 verification
pub struct ReliableFileStreamer {
    /// Chunk size for streaming
    chunk_size: usize,

    /// Maximum retry attempts
    max_retries: u8,
}

impl ReliableFileStreamer {
    /// Create a new file streamer with default settings
    pub fn new() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            max_retries: MAX_CHUNK_RETRIES,
        }
    }

    /// Create a new file streamer with custom chunk size
    pub fn with_chunk_size(chunk_size: usize) -> Self {
        Self {
            chunk_size,
            max_retries: MAX_CHUNK_RETRIES,
        }
    }

    /// Stream a file in chunks with CRC32 verification
    ///
    /// # Arguments
    /// * `file_path` - Path to file to stream
    /// * `chunk_tx` - Channel to send chunks
    /// * `ack_rx` - Channel to receive acknowledgments
    ///
    /// # Returns
    /// File-level CRC32 checksum
    pub async fn stream_file(
        &self,
        file_path: &Path,
        chunk_tx: mpsc::Sender<FileChunk>,
        mut ack_rx: mpsc::Receiver<ChunkAck>,
    ) -> Result<u32, StreamError> {
        let file = File::open(file_path).await.map_err(StreamError::Io)?;

        let file_size = file.metadata().await.map_err(StreamError::Io)?.len();

        let total_chunks = (file_size as usize).div_ceil(self.chunk_size) as u32;

        info!(
            file = %file_path.display(),
            size_bytes = file_size,
            total_chunks = total_chunks,
            chunk_size = self.chunk_size,
            "Starting file stream"
        );

        let mut reader = BufReader::new(file);
        let mut buffer = vec![0u8; self.chunk_size];
        let mut file_hasher = Hasher::new();

        for chunk_index in 0..total_chunks {
            let mut retry_count = 0;

            loop {
                // Read chunk data
                let bytes_read = reader.read(&mut buffer).await.map_err(StreamError::Io)?;

                if bytes_read == 0 {
                    break; // End of file
                }

                let chunk_data = &buffer[..bytes_read];

                // Calculate chunk CRC32
                let mut chunk_hasher = Hasher::new();
                chunk_hasher.update(chunk_data);
                let chunk_crc32 = chunk_hasher.finalize();

                // Update file CRC32
                file_hasher.update(chunk_data);

                // Send chunk
                let chunk = FileChunk {
                    chunk_index,
                    total_chunks,
                    data: chunk_data.to_vec(),
                    chunk_crc32,
                };

                chunk_tx
                    .send(chunk.clone())
                    .await
                    .map_err(|_| StreamError::ChannelClosed)?;

                debug!(
                    chunk_index = chunk_index,
                    total_chunks = total_chunks,
                    bytes = bytes_read,
                    crc32 = chunk_crc32,
                    "Sent chunk"
                );

                // Wait for acknowledgment
                let ack = ack_rx.recv().await.ok_or(StreamError::ChannelClosed)?;

                match ack.status {
                    TransferStatus::Success => {
                        debug!(chunk_index = chunk_index, "Chunk acknowledged");
                        break; // Move to next chunk
                    }
                    TransferStatus::ChecksumMismatch => {
                        retry_count += 1;
                        if retry_count >= self.max_retries {
                            error!(
                                chunk_index = chunk_index,
                                retries = retry_count,
                                "Chunk checksum mismatch - max retries exceeded"
                            );
                            return Err(StreamError::ChecksumMismatch {
                                chunk_index,
                                retries: retry_count,
                            });
                        }
                        warn!(
                            chunk_index = chunk_index,
                            retry = retry_count,
                            "Chunk checksum mismatch - retrying"
                        );
                    }
                    TransferStatus::Retry => {
                        retry_count += 1;
                        if retry_count >= self.max_retries {
                            error!(
                                chunk_index = chunk_index,
                                retries = retry_count,
                                "Chunk transfer failed - max retries exceeded"
                            );
                            return Err(StreamError::MaxRetriesExceeded {
                                chunk_index,
                                retries: retry_count,
                            });
                        }
                        warn!(
                            chunk_index = chunk_index,
                            retry = retry_count,
                            "Chunk transfer retry requested"
                        );
                    }
                    TransferStatus::Failed => {
                        error!(
                            chunk_index = chunk_index,
                            "Chunk transfer permanently failed"
                        );
                        return Err(StreamError::TransferFailed { chunk_index });
                    }
                }
            }
        }

        let file_crc32 = file_hasher.finalize();

        info!(
            file = %file_path.display(),
            crc32 = file_crc32,
            "File stream complete"
        );

        Ok(file_crc32)
    }

    /// Receive a file in chunks with CRC32 verification
    ///
    /// # Arguments
    /// * `output_path` - Path where file should be written
    /// * `chunk_rx` - Channel to receive chunks
    /// * `ack_tx` - Channel to send acknowledgments
    /// * `expected_crc32` - Expected file-level CRC32 (optional verification)
    pub async fn receive_file(
        &self,
        output_path: &Path,
        mut chunk_rx: mpsc::Receiver<FileChunk>,
        ack_tx: mpsc::Sender<ChunkAck>,
        expected_crc32: Option<u32>,
    ) -> Result<u32, StreamError> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = output_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(StreamError::Io)?;
        }

        let file = File::create(output_path).await.map_err(StreamError::Io)?;

        let mut writer = BufWriter::new(file);
        let mut file_hasher = Hasher::new();
        let mut expected_index = 0u32;

        info!(
            output_path = %output_path.display(),
            "Starting file receive"
        );

        while let Some(chunk) = chunk_rx.recv().await {
            // Verify sequential order
            if chunk.chunk_index != expected_index {
                error!(
                    expected = expected_index,
                    received = chunk.chunk_index,
                    "Out-of-order chunk"
                );

                ack_tx
                    .send(ChunkAck {
                        chunk_index: chunk.chunk_index,
                        status: TransferStatus::Failed,
                    })
                    .await
                    .ok();

                return Err(StreamError::OutOfOrder {
                    expected: expected_index,
                    received: chunk.chunk_index,
                });
            }

            // Verify chunk checksum
            let mut chunk_hasher = Hasher::new();
            chunk_hasher.update(&chunk.data);
            let calculated_crc32 = chunk_hasher.finalize();

            if calculated_crc32 != chunk.chunk_crc32 {
                warn!(
                    chunk_index = chunk.chunk_index,
                    expected = chunk.chunk_crc32,
                    calculated = calculated_crc32,
                    "Chunk checksum mismatch"
                );

                ack_tx
                    .send(ChunkAck {
                        chunk_index: chunk.chunk_index,
                        status: TransferStatus::ChecksumMismatch,
                    })
                    .await
                    .ok();

                continue; // Wait for retry
            }

            // Write chunk data
            writer
                .write_all(&chunk.data)
                .await
                .map_err(StreamError::Io)?;

            // Update file checksum
            file_hasher.update(&chunk.data);

            // Send acknowledgment
            ack_tx
                .send(ChunkAck {
                    chunk_index: chunk.chunk_index,
                    status: TransferStatus::Success,
                })
                .await
                .map_err(|_| StreamError::ChannelClosed)?;

            debug!(
                chunk_index = chunk.chunk_index,
                total_chunks = chunk.total_chunks,
                bytes = chunk.data.len(),
                "Chunk received and verified"
            );

            expected_index += 1;

            // Check if this was the last chunk
            if chunk.chunk_index + 1 == chunk.total_chunks {
                break;
            }
        }

        writer.flush().await.map_err(StreamError::Io)?;

        let file_crc32 = file_hasher.finalize();

        // Verify file-level checksum if provided
        if let Some(expected) = expected_crc32 {
            if file_crc32 != expected {
                error!(
                    expected = expected,
                    calculated = file_crc32,
                    "File checksum mismatch"
                );

                // Delete corrupted file
                tokio::fs::remove_file(output_path).await.ok();

                return Err(StreamError::FileChecksumMismatch {
                    expected,
                    calculated: file_crc32,
                });
            }
        }

        info!(
            output_path = %output_path.display(),
            crc32 = file_crc32,
            "File receive complete"
        );

        Ok(file_crc32)
    }

    /// Calculate CRC32 checksum of a file
    pub async fn calculate_file_crc32(file_path: &Path) -> Result<u32, StreamError> {
        let file = File::open(file_path).await.map_err(StreamError::Io)?;

        let mut reader = BufReader::new(file);
        let mut hasher = Hasher::new();
        let mut buffer = vec![0u8; DEFAULT_CHUNK_SIZE];

        loop {
            let bytes_read = reader.read(&mut buffer).await.map_err(StreamError::Io)?;

            if bytes_read == 0 {
                break;
            }

            hasher.update(&buffer[..bytes_read]);
        }

        Ok(hasher.finalize())
    }
}

impl Default for ReliableFileStreamer {
    fn default() -> Self {
        Self::new()
    }
}
