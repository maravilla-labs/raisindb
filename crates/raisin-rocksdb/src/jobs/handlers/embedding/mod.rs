//! Embedding job handler for processing embedding generation jobs.
//!
//! This handler processes three types of embedding jobs:
//! - EmbeddingGenerate: Generate embeddings for a node
//! - EmbeddingDelete: Remove embeddings for a node
//! - EmbeddingBranchCopy: Copy embeddings when creating a new branch

mod content_extraction;
mod handler;

pub use handler::EmbeddingJobHandler;
