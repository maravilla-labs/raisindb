//! Types and constants for reliable file streaming

use std::path::PathBuf;

use crate::tcp_protocol::TransferStatus;

/// Default chunk size for file streaming (1MB)
pub const DEFAULT_CHUNK_SIZE: usize = 1_048_576;

/// Default chunk size for Tantivy index files (256KB - smaller for many small files)
pub const TANTIVY_CHUNK_SIZE: usize = 262_144;

/// Maximum retry attempts for failed chunks
pub const MAX_CHUNK_RETRIES: u8 = 3;

/// File chunk with checksum
#[derive(Debug, Clone)]
pub struct FileChunk {
    /// Chunk index (0-based)
    pub chunk_index: u32,

    /// Total number of chunks for this file
    pub total_chunks: u32,

    /// Chunk data
    pub data: Vec<u8>,

    /// CRC32 checksum of this chunk
    pub chunk_crc32: u32,
}

/// File metadata with checksum
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// File path
    pub path: PathBuf,

    /// Expected CRC32 of entire file
    pub expected_crc32: u32,

    /// File size in bytes
    pub size_bytes: u64,
}

/// Chunk acknowledgment
#[derive(Debug, Clone)]
pub struct ChunkAck {
    /// Chunk index being acknowledged
    pub chunk_index: u32,

    /// Transfer status
    pub status: TransferStatus,
}

/// Stream errors
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Channel closed unexpectedly")]
    ChannelClosed,

    #[error("Chunk {chunk_index} checksum mismatch after {retries} retries")]
    ChecksumMismatch { chunk_index: u32, retries: u8 },

    #[error("Chunk {chunk_index} max retries exceeded ({retries} attempts)")]
    MaxRetriesExceeded { chunk_index: u32, retries: u8 },

    #[error("Chunk {chunk_index} transfer failed permanently")]
    TransferFailed { chunk_index: u32 },

    #[error("Out of order chunk: expected {expected}, received {received}")]
    OutOfOrder { expected: u32, received: u32 },

    #[error("File checksum mismatch: expected {expected}, calculated {calculated}")]
    FileChecksumMismatch { expected: u32, calculated: u32 },

    #[error("Semaphore error")]
    SemaphoreError,

    #[error("Task join error: {0}")]
    TaskJoinError(String),
}
