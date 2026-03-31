// SPDX-License-Identifier: BSL-1.1

//! Excerpt fetching support for HNSW search results.

/// Request to fetch an excerpt for a specific embedding.
#[derive(Debug, Clone)]
pub struct ExcerptRequest {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
    pub workspace_id: String,
    pub source_id: String,
    pub chunk_index: usize,
}

impl ExcerptRequest {
    /// Create a new excerpt request.
    pub fn new(
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace_id: String,
        source_id: String,
        chunk_index: usize,
    ) -> Self {
        Self {
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            source_id,
            chunk_index,
        }
    }
}

/// Trait for fetching text excerpts from storage.
///
/// This trait provides both synchronous and asynchronous methods for fetching
/// excerpts from the underlying storage layer (typically RocksDB).
#[async_trait::async_trait]
pub trait ExcerptFetcher: Send + Sync {
    /// Fetch excerpt for a specific embedding.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace_id` - Workspace identifier
    /// * `source_id` - Source identifier (node ID or asset ID)
    /// * `chunk_index` - Zero-based chunk index
    ///
    /// # Returns
    ///
    /// Optional excerpt text if found
    async fn get_excerpt(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        source_id: &str,
        chunk_index: usize,
    ) -> Option<String>;

    /// Batch fetch excerpts for multiple embeddings.
    ///
    /// # Arguments
    ///
    /// * `requests` - Vector of excerpt requests
    ///
    /// # Returns
    ///
    /// Vector of optional excerpts in the same order as requests
    async fn get_excerpts_batch(&self, requests: Vec<ExcerptRequest>) -> Vec<Option<String>>;
}
