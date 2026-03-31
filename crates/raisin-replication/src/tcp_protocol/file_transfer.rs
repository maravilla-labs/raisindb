//! File transfer types for TCP replication protocol

use serde::{Deserialize, Serialize};

/// Transfer status for chunked file transfers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    /// Chunk received and verified successfully
    Success,

    /// Checksum mismatch - retry needed
    ChecksumMismatch,

    /// Retry requested for other reason
    Retry,

    /// Transfer failed permanently
    Failed,
}

/// SST file information for checkpoint transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SstFileInfo {
    /// File name (e.g., "000123.sst")
    pub file_name: String,

    /// File size in bytes
    pub size_bytes: u64,

    /// CRC32 checksum of entire file
    pub crc32: u32,
}

/// Index file information for Tantivy transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexFileInfo {
    /// File name (e.g., "meta.json", "000001.idx")
    pub file_name: String,

    /// File size in bytes
    pub size_bytes: u64,

    /// CRC32 checksum of entire file
    pub crc32: u32,
}
