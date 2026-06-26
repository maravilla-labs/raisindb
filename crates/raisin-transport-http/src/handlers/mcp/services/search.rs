// SPDX-License-Identifier: BSL-1.1

//! [`SearchProvider`] backed by the Tantivy full-text and HNSW vector engines.

use std::future::Future;
use std::pin::Pin;

use raisin_mcp::{McpError, McpIdentity, SearchHit, SearchMode, SearchProvider, SearchQuery};

use crate::state::AppState;

/// Full-text + semantic search over RaisinDB content for the MCP `search_nodes`
/// tool.
///
/// Full-text queries use the Tantivy [`IndexingEngine`](raisin_storage::IndexingEngine);
/// semantic queries embed the query string with the tenant's configured provider
/// and search the HNSW index. Both engines are synchronous/CPU-bound, so the work
/// is offloaded to a blocking pool.
#[derive(Clone)]
pub(in crate::handlers::mcp) struct HttpSearchProvider {
    state: AppState,
    tenant_id: String,
    repo: String,
}

impl HttpSearchProvider {
    /// Bind a search provider to the request's tenant / repo scope.
    pub(in crate::handlers::mcp) fn new(
        state: AppState,
        tenant_id: impl Into<String>,
        repo: impl Into<String>,
    ) -> Self {
        Self {
            state,
            tenant_id: tenant_id.into(),
            repo: repo.into(),
        }
    }

    /// Synchronous Tantivy search (run inside `spawn_blocking`).
    fn fulltext(&self, query: &SearchQuery) -> raisin_mcp::Result<Vec<SearchHit>> {
        use raisin_storage::{FullTextSearchQuery, IndexingEngine};

        let engine = self
            .state
            .indexing_engine
            .as_ref()
            .ok_or_else(|| McpError::not_found("full-text search is not enabled"))?;

        let ftq = FullTextSearchQuery {
            tenant_id: self.tenant_id.clone(),
            repo_id: self.repo.clone(),
            workspace_ids: Some(vec![query.workspace.clone()]),
            branch: query.branch.clone(),
            language: "en".to_string(),
            query: query.query.clone(),
            limit: query.limit.max(1),
            revision: None,
            shape_type: query.node_type.clone(),
        };

        let results = engine
            .search(&ftq)
            .map_err(|e| McpError::protocol(format!("full-text search failed: {e}")))?;

        Ok(results
            .into_iter()
            .map(|r| SearchHit {
                node_id: r.node_id,
                workspace: r.workspace_id,
                score: r.score,
                path: r.path,
                node_type: r.node_type,
            })
            .collect())
    }

    /// Semantic search: embed the query, then search the HNSW index.
    async fn vector(&self, query: &SearchQuery) -> raisin_mcp::Result<Vec<SearchHit>> {
        use raisin_embeddings::crypto::ApiKeyEncryptor;
        use raisin_embeddings::provider::create_provider;
        use raisin_embeddings::TenantEmbeddingConfigStore;

        let engine = self
            .state
            .hnsw_engine
            .as_ref()
            .ok_or_else(|| McpError::not_found("vector search is not enabled"))?
            .clone();

        let rocksdb = self
            .state
            .rocksdb_storage
            .as_ref()
            .ok_or_else(|| McpError::not_found("vector search requires the RocksDB backend"))?;

        let config = rocksdb
            .tenant_embedding_config_repository()
            .get_config(&self.tenant_id)
            .map_err(|e| McpError::protocol(format!("embedding config lookup failed: {e}")))?
            .ok_or_else(|| McpError::not_found("no embedding configuration for tenant"))?;

        if !config.enabled {
            return Err(McpError::not_found("embeddings are not enabled for tenant"));
        }

        let master_key = self.state.get_master_key().map_err(McpError::from)?;
        let encryptor = ApiKeyEncryptor::new(&master_key);
        let api_key_encrypted = config
            .api_key_encrypted
            .as_ref()
            .ok_or_else(|| McpError::not_found("no embedding API key configured"))?;
        let api_key = encryptor
            .decrypt(api_key_encrypted)
            .map_err(|e| McpError::protocol(format!("failed to decrypt embedding key: {e}")))?;

        let provider = create_provider(&config.provider, &api_key, &config.model)
            .map_err(|e| McpError::protocol(format!("embedding provider error: {e}")))?;
        let embedding = provider
            .generate_embedding(&query.query)
            .await
            .map_err(|e| McpError::protocol(format!("query embedding failed: {e}")))?;
        let embedding = raisin_hnsw::normalize_vector(&embedding);

        let tenant = self.tenant_id.clone();
        let repo = self.repo.clone();
        let branch = query.branch.clone();
        let workspace = query.workspace.clone();
        let limit = query.limit.max(1);

        let results = tokio::task::spawn_blocking(move || {
            engine.search(&tenant, &repo, &branch, Some(&workspace), &embedding, limit)
        })
        .await
        .map_err(|e| McpError::protocol(format!("vector search task failed: {e}")))?
        .map_err(|e| McpError::protocol(format!("vector search failed: {e}")))?;

        Ok(results
            .into_iter()
            .map(|r| SearchHit {
                node_id: r.node_id,
                workspace: r.workspace_id,
                // Lower distance is closer; expose similarity as a descending score.
                score: 1.0 - r.distance,
                path: None,
                node_type: None,
            })
            .collect())
    }
}

impl SearchProvider for HttpSearchProvider {
    fn search<'a>(
        &'a self,
        _identity: &'a McpIdentity,
        query: SearchQuery,
    ) -> Pin<Box<dyn Future<Output = raisin_mcp::Result<Vec<SearchHit>>> + Send + 'a>> {
        Box::pin(async move {
            match query.mode {
                SearchMode::Fulltext => {
                    let provider = self.clone();
                    tokio::task::spawn_blocking(move || provider.fulltext(&query))
                        .await
                        .map_err(|e| {
                            McpError::protocol(format!("full-text search task failed: {e}"))
                        })?
                }
                SearchMode::Vector => self.vector(&query).await,
            }
        })
    }
}
