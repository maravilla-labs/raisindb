//! Vector embedding storage implementation using RocksDB.
//!
//! This module provides persistent storage for node embeddings and embedding jobs.
//! Embeddings are stored with revision awareness, allowing time-travel queries.

mod job_store;
mod storage;

#[cfg(test)]
mod tests;

pub use job_store::RocksDBEmbeddingJobStore;
pub use storage::RocksDBEmbeddingStorage;
