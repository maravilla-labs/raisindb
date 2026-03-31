//! HNSW vector index transfer for cluster catch-up
//!
//! This module provides functionality to transfer HNSW vector indexes
//! between nodes during cluster catch-up operations.
//!
//! HNSW indexes are typically small enough to be transferred as complete files
//! rather than chunked streaming.

mod manager;
mod receiver;
#[cfg(test)]
mod tests;
mod types;

pub use manager::HnswIndexManager;
pub use receiver::HnswIndexReceiver;
pub use types::HnswIndexMetadata;
