// SPDX-License-Identifier: BSL-1.1

//! Vector similarity search using HNSW.
//!
//! This module:
//! 1. Gets tenant embedding configuration
//! 2. Decrypts API key
//! 3. Generates embedding for query text using the provider
//! 4. Searches HNSW index with the generated embedding

use crate::state::AppState;

use super::types::{HybridSearchQuery, HybridSearchResult};

/// Perform vector search using HNSW.
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn perform_vector_search(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    params: &HybridSearchQuery,
) -> Result<Vec<HybridSearchResult>, Box<dyn std::error::Error>> {
    use raisin_embeddings::crypto::ApiKeyEncryptor;
    use raisin_embeddings::provider::create_provider;
    use raisin_embeddings::TenantEmbeddingConfigStore;

    let hnsw_engine = state
        .hnsw_engine
        .as_ref()
        .ok_or("Vector search not available")?;

    // Get tenant embedding config to generate query embedding
    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or("RocksDB storage required for vector search")?;

    let config_repo = rocksdb_storage.tenant_embedding_config_repository();
    let config = config_repo
        .get_config(tenant_id)
        .map_err(|e| format!("Failed to get embedding config: {}", e))?
        .ok_or("No embedding configuration found for tenant")?;

    if !config.enabled {
        return Err("Embeddings not enabled for this tenant".into());
    }

    // Decrypt API key
    let master_key = std::env::var("RAISIN_MASTER_KEY").map_err(|_| "RAISIN_MASTER_KEY not set")?;
    let master_key_bytes: [u8; 32] = hex::decode(&master_key)
        .map_err(|e| format!("Invalid master key hex: {}", e))?
        .try_into()
        .map_err(|_| "Master key must be 32 bytes")?;

    let encryptor = ApiKeyEncryptor::new(&master_key_bytes);
    let api_key_encrypted = config
        .api_key_encrypted
        .as_ref()
        .ok_or("No API key configured for embeddings")?;
    let api_key = encryptor
        .decrypt(api_key_encrypted)
        .map_err(|e| format!("Failed to decrypt API key: {}", e))?;

    // Create embedding provider and generate embedding for query
    let provider = create_provider(&config.provider, &api_key, &config.model)?;

    tracing::info!(
        query_text = %params.q,
        provider = ?config.provider,
        model = %config.model,
        "Generating query embedding for vector search"
    );

    let mut query_embedding = provider.generate_embedding(&params.q).await?;

    // Normalize query to unit length for cosine-like distance
    // This must match the normalization done when storing embeddings
    query_embedding = raisin_hnsw::normalize_vector(&query_embedding);

    tracing::info!(
        query = %params.q,
        embedding_dims = query_embedding.len(),
        "Successfully generated and normalized query embedding for vector search"
    );

    // Perform vector search with generated embedding
    // Use Some() to filter by workspace, or None for global search
    let search_results = hnsw_engine.search(
        tenant_id,
        repo,
        &params.branch,
        Some(&params.workspace), // Filter by workspace
        &query_embedding,
        params.limit * 2, // Get more results for RRF merging
    )?;

    // Get node metadata from storage
    use raisin_storage::{NodeRepository, Storage, StorageScope};

    let mut results = Vec::new();
    for search_result in search_results.into_iter() {
        // Fetch node from storage to get metadata
        let node_repo = state.storage.nodes();
        match node_repo
            .get(
                StorageScope::new(tenant_id, repo, &params.branch, &search_result.workspace_id),
                &search_result.node_id,
                Some(&search_result.revision),
            )
            .await
        {
            Ok(Some(node)) => {
                results.push(HybridSearchResult {
                    node_id: search_result.node_id,
                    name: node.name,
                    node_type: node.node_type,
                    path: node.path,
                    workspace_id: search_result.workspace_id,
                    score: 0.0, // Will be set by RRF
                    fulltext_rank: None,
                    vector_distance: Some(search_result.distance),
                    revision: search_result.revision,
                });
            }
            Ok(None) => {
                // Node not found (maybe deleted) - skip it
                tracing::warn!(
                    node_id = %search_result.node_id,
                    "Node in HNSW index not found in storage, skipping"
                );
            }
            Err(e) => {
                tracing::error!(
                    node_id = %search_result.node_id,
                    error = %e,
                    "Failed to fetch node metadata"
                );
            }
        }
    }

    tracing::debug!(count = results.len(), "Vector search completed");

    Ok(results)
}
