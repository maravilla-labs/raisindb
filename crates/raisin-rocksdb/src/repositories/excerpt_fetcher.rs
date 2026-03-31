//! RocksDB-based implementation of ExcerptFetcher for HNSW search results.

use crate::{cf, cf_handle};
use raisin_embeddings::EmbeddingData;
use raisin_hnsw::{ExcerptFetcher, ExcerptRequest};
use rocksdb::DB;
use std::sync::Arc;

/// RocksDB implementation of ExcerptFetcher.
///
/// Fetches text excerpts from the `embeddings` column family by reading
/// the `chunk_content` field from stored `EmbeddingData`.
#[derive(Clone)]
pub struct RocksDBExcerptFetcher {
    db: Arc<DB>,
}

impl RocksDBExcerptFetcher {
    /// Create a new RocksDB excerpt fetcher.
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Build embedding key prefix for a specific chunk.
    ///
    /// Format: `{tenant}\0{repo}\0{branch}\0{workspace}\0{embedder_hash}\0{kind}\0{source_id}\0{chunk_idx:04}\0`
    ///
    /// Since we don't know the embedder_hash or exact revision, we'll use a prefix
    /// that matches all embedders/revisions for this chunk and take the latest.
    fn build_chunk_prefix(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        source_id: &str,
    ) -> Vec<u8> {
        let mut prefix = Vec::new();
        prefix.extend_from_slice(tenant_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(repo_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(branch.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(workspace_id.as_bytes());
        prefix.push(0);
        // Don't include embedder_hash, kind, or chunk - we'll filter during iteration
        prefix
    }

    /// Build a more specific prefix including source_id pattern.
    ///
    /// This still needs iteration because we don't know embedder_hash.
    fn build_source_prefix(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
    ) -> Vec<u8> {
        let mut prefix = Vec::new();
        prefix.extend_from_slice(tenant_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(repo_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(branch.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(workspace_id.as_bytes());
        prefix.push(0);
        prefix
    }

    /// Parse key components to extract source_id and chunk_index.
    ///
    /// New format key: `{tenant}\0{repo}\0{branch}\0{workspace}\0{embedder_hash}\0{kind}\0{source_id}\0{chunk_idx:04}\0{revision}`
    fn parse_key(key: &[u8]) -> Option<(String, usize)> {
        let key_str = String::from_utf8_lossy(key);
        let parts: Vec<&str> = key_str.split('\0').collect();

        if parts.len() >= 8 {
            // New format
            let source_id = parts[6].to_string();
            let chunk_idx = parts[7].parse().ok()?;
            Some((source_id, chunk_idx))
        } else {
            None
        }
    }

    /// Fetch excerpt synchronously (used internally).
    fn get_excerpt_sync(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        source_id: &str,
        chunk_index: usize,
    ) -> Option<String> {
        let cf = match cf_handle(&self.db, cf::EMBEDDINGS) {
            Ok(cf) => cf,
            Err(e) => {
                tracing::warn!("Failed to get embeddings CF: {}", e);
                return None;
            }
        };

        // Build prefix for this workspace
        let prefix = Self::build_source_prefix(tenant_id, repo_id, branch, workspace_id);
        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        // Scan for matching source_id and chunk_index
        // We take the first match (latest revision) since revisions are in descending order
        for result in iter {
            let (key, value) = match result {
                Ok(kv) => kv,
                Err(e) => {
                    tracing::warn!("Failed to iterate embeddings: {}", e);
                    continue;
                }
            };

            // Stop if we've gone past our prefix
            if !key.starts_with(&prefix) {
                break;
            }

            // Parse key to check if it matches our target
            if let Some((key_source_id, key_chunk_idx)) = Self::parse_key(&key) {
                if key_source_id == source_id && key_chunk_idx == chunk_index {
                    // Found a match - deserialize and extract chunk_content
                    match rmp_serde::from_slice::<EmbeddingData>(&value) {
                        Ok(embedding_data) => {
                            return embedding_data.chunk_content;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to deserialize embedding data: {}", e);
                            return None;
                        }
                    }
                }
            }
        }

        None
    }
}

#[async_trait::async_trait]
impl ExcerptFetcher for RocksDBExcerptFetcher {
    async fn get_excerpt(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        source_id: &str,
        chunk_index: usize,
    ) -> Option<String> {
        // RocksDB operations are synchronous, so we just call the sync version
        // This is executed in the tokio runtime but doesn't actually block async tasks
        // since it's fast and CPU-bound, not I/O-bound
        self.get_excerpt_sync(
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            source_id,
            chunk_index,
        )
    }

    async fn get_excerpts_batch(&self, requests: Vec<ExcerptRequest>) -> Vec<Option<String>> {
        // Process batch synchronously
        // For better performance, we could group requests by workspace and do a single scan
        // But for simplicity, we'll just fetch each one individually
        requests
            .into_iter()
            .map(|req| {
                self.get_excerpt_sync(
                    &req.tenant_id,
                    &req.repo_id,
                    &req.branch,
                    &req.workspace_id,
                    &req.source_id,
                    req.chunk_index,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use raisin_ai::config::{EmbedderId, EmbeddingKind};
    use raisin_embeddings::{EmbeddingData, EmbeddingProvider, EmbeddingStorage};
    use raisin_hlc::HLC;
    use tempfile::TempDir;

    fn create_test_db() -> (Arc<DB>, TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cfs = vec![cf::EMBEDDINGS];
        let db = Arc::new(DB::open_cf(&opts, temp_dir.path(), cfs).unwrap());
        (db, temp_dir)
    }

    fn create_test_embedding(source_id: &str, chunk_index: usize, content: &str) -> EmbeddingData {
        let embedder_id = EmbedderId::new("openai", "text-embedding-3-small", 3);

        #[allow(deprecated)]
        EmbeddingData {
            vector: vec![0.1, 0.2, 0.3],
            embedder_id,
            embedding_kind: EmbeddingKind::Text,
            source_id: source_id.to_string(),
            chunk_index,
            total_chunks: 3,
            chunk_content: Some(content.to_string()),
            generated_at: Utc::now(),
            text_hash: 12345,
            model: "text-embedding-3-small".to_string(),
            provider: EmbeddingProvider::OpenAI,
        }
    }

    #[tokio::test]
    async fn test_excerpt_fetcher() {
        let (db, _temp_dir) = create_test_db();
        let storage =
            crate::repositories::embedding_storage::RocksDBEmbeddingStorage::new(Arc::clone(&db));
        let fetcher = RocksDBExcerptFetcher::new(db);

        // Store some embeddings with different chunks
        let embedding1 = create_test_embedding("doc1", 0, "First chunk content");
        let embedding2 = create_test_embedding("doc1", 1, "Second chunk content");
        let embedding3 = create_test_embedding("doc2", 0, "Another document");

        let revision = HLC::new(100, 0);

        storage
            .store_embedding(
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "doc1",
                &revision,
                &embedding1,
            )
            .unwrap();

        storage
            .store_embedding(
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "doc1",
                &revision,
                &embedding2,
            )
            .unwrap();

        storage
            .store_embedding(
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "doc2",
                &revision,
                &embedding3,
            )
            .unwrap();

        // Test single fetch
        let excerpt = fetcher
            .get_excerpt("tenant1", "repo1", "main", "ws1", "doc1", 0)
            .await;
        assert_eq!(excerpt, Some("First chunk content".to_string()));

        let excerpt = fetcher
            .get_excerpt("tenant1", "repo1", "main", "ws1", "doc1", 1)
            .await;
        assert_eq!(excerpt, Some("Second chunk content".to_string()));

        // Test batch fetch
        let requests = vec![
            ExcerptRequest::new(
                "tenant1".to_string(),
                "repo1".to_string(),
                "main".to_string(),
                "ws1".to_string(),
                "doc1".to_string(),
                0,
            ),
            ExcerptRequest::new(
                "tenant1".to_string(),
                "repo1".to_string(),
                "main".to_string(),
                "ws1".to_string(),
                "doc2".to_string(),
                0,
            ),
            ExcerptRequest::new(
                "tenant1".to_string(),
                "repo1".to_string(),
                "main".to_string(),
                "ws1".to_string(),
                "nonexistent".to_string(),
                0,
            ),
        ];

        let excerpts = fetcher.get_excerpts_batch(requests).await;
        assert_eq!(excerpts.len(), 3);
        assert_eq!(excerpts[0], Some("First chunk content".to_string()));
        assert_eq!(excerpts[1], Some("Another document".to_string()));
        assert_eq!(excerpts[2], None);
    }
}
