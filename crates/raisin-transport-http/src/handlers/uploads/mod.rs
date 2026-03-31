// SPDX-License-Identifier: BSL-1.1

//! Resumable file upload API handlers
//!
//! Provides endpoints for managing resumable chunked file uploads:
//! - Create upload session
//! - Upload chunks with progress tracking
//! - Query upload status
//! - Complete/cancel uploads
//!
//! Upload sessions are stored in-memory (RocksDB storage will be added later).
//! Chunks are written to temporary files and reassembled on completion.

mod chunks;
mod completion;
mod create;
mod status;
mod types;

// Re-export all public types and handler functions
pub use chunks::upload_chunk;
pub use completion::{cancel_upload, complete_upload};
pub use create::create_upload;
pub use status::{get_upload_progress, get_upload_status};
pub use types::{
    ChunkUploadResponse, CompleteUploadRequest, CompleteUploadResponse, CreateUploadRequest,
    CreateUploadResponse, UploadSession, UploadSessionStatus, UploadSessionStore,
};

#[cfg(test)]
mod tests {
    use super::chunks::parse_content_range;

    #[test]
    fn test_parse_content_range() {
        let result = parse_content_range("bytes 0-10485759/10737418240").unwrap();
        assert_eq!(result, (0, 10485759, 10737418240));

        let result = parse_content_range("bytes 10485760-20971519/10737418240").unwrap();
        assert_eq!(result, (10485760, 20971519, 10737418240));
    }

    #[test]
    fn test_parse_content_range_invalid() {
        assert!(parse_content_range("invalid").is_err());
        assert!(parse_content_range("bytes abc-def/ghi").is_err());
        assert!(parse_content_range("bytes 0-100").is_err());
    }
}
