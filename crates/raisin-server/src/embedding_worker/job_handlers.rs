//! Job handlers for different embedding job types.

use raisin_ai::config::{EmbedderId, EmbeddingKind};
use raisin_ai::crypto::ApiKeyEncryptor;
use raisin_ai::{AIUseCase, TenantAIConfigStore};
use raisin_embeddings::config::EmbeddingProvider;
use raisin_embeddings::models::{EmbeddingData, EmbeddingJob, EmbeddingJobKind};
use raisin_embeddings::provider::{create_provider, EmbeddingProvider as EmbeddingProviderTrait};
use raisin_embeddings::EmbeddingStorage;
use raisin_error::{Error, Result};
use raisin_hnsw::HnswIndexingEngine;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{NodeRepository, Storage};
use std::sync::Arc;

use super::helpers::{
    extract_embeddable_content, hash_text, map_ai_provider_to_embedding_provider,
};

/// Handle AddNode job type - generate embedding for a new node
#[allow(deprecated)]
pub async fn handle_add_node<E>(
    storage: &Arc<RocksDBStorage>,
    embedding_storage: &Arc<E>,
    hnsw_engine: &Arc<HnswIndexingEngine>,
    master_key: [u8; 32],
    job: &EmbeddingJob,
) -> Result<()>
where
    E: EmbeddingStorage,
{
    // 1. Get tenant AI config (multi-tenant: each job has tenant_id)
    let config_repo = storage.tenant_ai_config_repository();
    let ai_config = config_repo
        .get_config(&job.tenant_id)
        .await
        .map_err(|e| Error::storage(e.to_string()))?;

    // 2. Get embedding settings
    let embedding_settings = ai_config
        .embedding_settings
        .as_ref()
        .ok_or_else(|| Error::NotFound("No embedding settings configured for tenant".to_string()))?;

    if !embedding_settings.enabled {
        tracing::debug!(
            tenant_id = %job.tenant_id,
            "Embeddings disabled for tenant, skipping"
        );
        return Ok(());
    }

    // 3. Find embedding provider and model
    let (provider_config, model_config) = ai_config
        .get_default_provider(AIUseCase::Embedding)
        .and_then(|p| p.get_default_model(AIUseCase::Embedding).map(|m| (p, m)))
        .ok_or_else(|| Error::NotFound("No embedding provider configured".to_string()))?;

    // 4. Decrypt API key
    let encryptor = ApiKeyEncryptor::new(&master_key);
    let api_key = if let Some(encrypted) = &provider_config.api_key_encrypted {
        encryptor
            .decrypt(encrypted)
            .map_err(|e| Error::Backend(format!("Failed to decrypt API key: {}", e)))?
    } else if provider_config.provider.requires_api_key() {
        return Err(Error::Validation(
            "No API key configured for provider".to_string(),
        ));
    } else {
        String::new() // Ollama doesn't need API key
    };

    // 5. Map AI provider to embedding provider (temporary until raisin-ai supports embeddings)
    let embedding_provider = map_ai_provider_to_embedding_provider(&provider_config.provider)?;

    // 6. Create embedding provider
    let provider = create_provider(&embedding_provider, &api_key, &model_config.model_id)?;

    // 7. Fetch node at exact revision
    let node_id = job
        .node_id
        .as_ref()
        .ok_or_else(|| Error::Validation("node_id required for AddNode".to_string()))?;

    let node = storage
        .nodes()
        .get(
            &job.tenant_id,
            &job.repo_id,
            &job.branch,
            &job.workspace_id,
            node_id,
            Some(&job.revision),
        )
        .await?
        .ok_or_else(|| Error::NotFound(format!("Node {} not found", node_id)))?;

    // 8. Extract embeddable content
    let text = extract_embeddable_content(&node, embedding_settings)?;

    if text.is_empty() {
        tracing::warn!(node_id = %node_id, "No embeddable content found, skipping");
        return Ok(());
    }

    // 9. Generate embedding
    let embedding = provider.generate_embedding(&text).await?;
    let dimensions = embedding.len();

    tracing::debug!(
        node_id = %node_id,
        embedding_dims = dimensions,
        text_length = text.len(),
        model = %model_config.model_id,
        provider = ?provider_config.provider,
        "Generated embedding"
    );

    // 10. Store in RocksDB embeddings CF
    let embedder_id = EmbedderId::new(
        format!("{:?}", provider_config.provider).to_lowercase(),
        model_config.model_id.clone(),
        dimensions,
    );

    let embedding_data = EmbeddingData {
        vector: embedding.clone(),
        embedder_id,
        embedding_kind: EmbeddingKind::Text,
        source_id: node_id.clone(),
        chunk_index: 0,
        total_chunks: 1,
        chunk_content: Some(text.chars().take(100).collect::<String>()),
        generated_at: chrono::Utc::now(),
        text_hash: hash_text(&text),
        // Legacy fields (deprecated)
        model: model_config.model_id.clone(),
        provider: embedding_provider,
    };

    embedding_storage.store_embedding(
        &job.tenant_id,
        &job.repo_id,
        &job.branch,
        &job.workspace_id,
        node_id,
        &job.revision,
        &embedding_data,
    )?;

    // 11. Add to HNSW index
    let engine_clone = Arc::clone(hnsw_engine);
    let node_id_clone = node_id.clone();
    let tenant_id = job.tenant_id.clone();
    let repo_id = job.repo_id.clone();
    let branch = job.branch.clone();
    let workspace_id = job.workspace_id.clone();
    let revision = job.revision;

    tokio::task::spawn_blocking(move || {
        engine_clone.add_embedding(
            &tenant_id,
            &repo_id,
            &branch,
            &workspace_id,
            &node_id_clone,
            revision,
            embedding,
        )
    })
    .await
    .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

    tracing::debug!(node_id = %node_id, "Added to HNSW index");

    Ok(())
}

/// Handle DeleteNode job type - remove embedding for a deleted node
pub async fn handle_delete_node(
    hnsw_engine: &Arc<HnswIndexingEngine>,
    job: &EmbeddingJob,
) -> Result<()> {
    let node_id = job
        .node_id
        .as_ref()
        .ok_or_else(|| Error::Validation("node_id required".to_string()))?;

    let engine_clone = Arc::clone(hnsw_engine);
    let node_id_clone = node_id.clone();
    let tenant_id = job.tenant_id.clone();
    let repo_id = job.repo_id.clone();
    let branch = job.branch.clone();

    tokio::task::spawn_blocking(move || {
        engine_clone.remove_embedding(&tenant_id, &repo_id, &branch, &node_id_clone)
    })
    .await
    .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

    tracing::debug!(node_id = %node_id, "Removed from HNSW index");

    Ok(())
}

/// Handle BranchCreated job type - copy HNSW index for new branch
pub async fn handle_branch_created(
    hnsw_engine: &Arc<HnswIndexingEngine>,
    job: &EmbeddingJob,
) -> Result<()> {
    let source_branch = job
        .source_branch
        .as_ref()
        .ok_or_else(|| Error::Validation("source_branch required".to_string()))?;

    let engine_clone = Arc::clone(hnsw_engine);
    let tenant_id = job.tenant_id.clone();
    let repo_id = job.repo_id.clone();
    let new_branch = job.branch.clone();
    let source_branch_clone = source_branch.clone();

    tokio::task::spawn_blocking(move || {
        engine_clone.copy_for_branch(&tenant_id, &repo_id, &source_branch_clone, &new_branch)
    })
    .await
    .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

    tracing::info!(
        branch = %job.branch,
        source_branch = %source_branch,
        "Copied HNSW index for new branch"
    );

    Ok(())
}
