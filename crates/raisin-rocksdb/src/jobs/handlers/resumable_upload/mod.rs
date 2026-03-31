//! Resumable upload completion job handler
//!
//! This module handles the background processing of resumable chunked uploads.
//! When all chunks have been uploaded, this job:
//! 1. Validates all chunk files exist
//! 2. Streams chunks to BinaryStorage via put_stream()
//! 3. Creates/updates the node with Resource property
//! 4. Deletes temporary chunk files
//! 5. Updates session status to Completed or Failed

mod cleanup;
mod handler;
mod node_operations;

pub use cleanup::UploadSessionCleanupHandler;
pub use handler::{BinaryUploadCallback, ResumableUploadHandler};
