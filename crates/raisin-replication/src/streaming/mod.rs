//! Reliable file streaming with CRC32 verification
//!
//! This module provides infrastructure for streaming large files in chunks
//! with checksum verification, retry logic, and flow control.

mod file_streamer;
mod orchestrator;
#[cfg(test)]
mod tests;
pub mod types;

pub use file_streamer::ReliableFileStreamer;
pub use orchestrator::ParallelTransferOrchestrator;
pub use types::{
    ChunkAck, FileChunk, FileInfo, StreamError, DEFAULT_CHUNK_SIZE, TANTIVY_CHUNK_SIZE,
};
