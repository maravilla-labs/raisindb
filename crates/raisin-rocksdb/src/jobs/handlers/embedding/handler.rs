//! Embedding job handler struct and methods.
//!
//! Contains the `EmbeddingJobHandler` struct with methods for handling
//! embedding generation, deletion, and branch copy operations.

use super::content_extraction::{extract_embeddable_content, hash_text};
use crate::RocksDBStorage;
use raisin_ai::storage::TenantAIConfigStore;
use raisin_embeddings::config::{EmbeddingProvider, TenantEmbeddingConfig};
use raisin_embeddings::crypto::ApiKeyEncryptor;
use raisin_embeddings::models::EmbeddingData;
use raisin_embeddings::provider::create_provider;
use raisin_embeddings::EmbeddingStorage;
use raisin_embeddings::TenantEmbeddingConfigStore;
use raisin_error::{Error, Result};
use raisin_hnsw::HnswIndexingEngine;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::sync::Arc;

/// Handler for embedding generation jobs
///
/// This handler processes embedding-related jobs by:
/// 1. Fetching tenant embedding configuration
/// 2. Decrypting API keys
/// 3. Creating embedding providers
/// 4. Generating or managing embeddings
/// 5. Updating HNSW index
pub struct EmbeddingJobHandler {
    storage: Arc<RocksDBStorage>,
    hnsw_engine: Arc<HnswIndexingEngine>,
    master_key: [u8; 32],
}

impl EmbeddingJobHandler {
    /// Create a new embedding job handler
    pub fn new(
        storage: Arc<RocksDBStorage>,
        hnsw_engine: Arc<HnswIndexingEngine>,
        master_key: [u8; 32],
    ) -> Self {
        Self {
            storage,
            hnsw_engine,
            master_key,
        }
    }

    /// Handle embedding generation job
    ///
    /// This method:
    /// 1. Extracts node_id from JobType::EmbeddingGenerate
    /// 2. Gets tenant embedding config from storage
    /// 3. Decrypts API key using ApiKeyEncryptor
    /// 4. Creates embedding provider
    /// 5. Fetches node and extracts embeddable content
    /// 6. Generates embedding via provider
    /// 7. Stores in RocksDB embedding storage
    /// 8. Adds to HNSW index
    pub async fn handle_generate(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract node_id from job type
        let node_id = match &job.job_type {
            JobType::EmbeddingGenerate { node_id } => node_id,
            _ => {
                return Err(Error::Validation(format!(
                    "Expected EmbeddingGenerate job, got {}",
                    job.job_type
                )))
            }
        };

        tracing::debug!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            workspace_id = %context.workspace_id,
            node_id = %node_id,
            revision = %context.revision,
            "Processing embedding generation job"
        );

        // Get tenant embedding config
        let config_repo = self.storage.tenant_embedding_config_repository();
        let config = config_repo
            .get_config(&context.tenant_id)
            .map_err(|e| Error::storage(e.to_string()))?
            .ok_or_else(|| Error::NotFound("No embedding config for tenant".to_string()))?;

        if !config.enabled {
            tracing::debug!(
                tenant_id = %context.tenant_id,
                "Embeddings disabled for tenant, skipping"
            );
            return Ok(());
        }

        // Resolve provider - use unified AI config if available, otherwise legacy fields
        let (embedding_provider, api_key, model) = self
            .resolve_embedding_provider(&config, &context.tenant_id)
            .await?;

        // Create embedding provider
        let provider = create_provider(&embedding_provider, &api_key, &model)?;

        // Fetch node at exact revision
        let node = self
            .storage
            .nodes()
            .get(
                StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    &context.workspace_id,
                ),
                node_id,
                Some(&context.revision),
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node {} not found", node_id)))?;

        // Extract embeddable content using schema-driven approach
        let text =
            extract_embeddable_content(&node, &config, self.storage.clone(), context).await?;

        if text.is_empty() {
            tracing::warn!(node_id = %node_id, "No embeddable content found, skipping");
            return Ok(());
        }

        // Log the actual text being embedded (truncated for readability)
        let text_preview = if text.len() > 200 {
            format!(
                "{}... [truncated, total {} chars]",
                &text[..200],
                text.len()
            )
        } else {
            text.clone()
        };
        tracing::info!(
            node_id = %node_id,
            node_name = %node.name,
            node_type = %node.node_type,
            text_length = text.len(),
            text_preview = %text_preview,
            "About to generate embedding for this text"
        );

        // Create embedder identity for multi-model support
        let embedder_id = raisin_ai::config::EmbedderId::new(
            format!("{:?}", config.provider).to_lowercase(),
            config.model.clone(),
            config.dimensions,
        );

        // Split text into chunks if chunking is configured
        let chunks: Vec<(String, usize)> = if let Some(ref chunking_config) = config.chunking {
            match raisin_ai::chunking::TextChunker::chunk_text(&text, chunking_config) {
                Ok(text_chunks) if text_chunks.len() > 1 => {
                    tracing::info!(
                        node_id = %node_id,
                        chunk_count = text_chunks.len(),
                        "Split text into {} chunks for embedding",
                        text_chunks.len()
                    );
                    text_chunks
                        .into_iter()
                        .map(|c| (c.content, c.index))
                        .collect()
                }
                Ok(_) => {
                    // Single chunk or empty - treat as whole document
                    vec![(text.clone(), 0)]
                }
                Err(e) => {
                    tracing::warn!(
                        node_id = %node_id,
                        error = %e,
                        "Chunking failed, falling back to single embedding"
                    );
                    vec![(text.clone(), 0)]
                }
            }
        } else {
            vec![(text.clone(), 0)]
        };

        let total_chunks = chunks.len();

        // Remove old chunks from HNSW before adding new ones (handles re-embedding)
        self.remove_old_chunks_from_hnsw(node_id, context, total_chunks)
            .await?;

        // Generate embeddings - use batch API if multiple chunks
        let chunk_texts: Vec<String> = chunks.iter().map(|(content, _)| content.clone()).collect();
        let embeddings = if chunk_texts.len() > 1 {
            let mut batch = provider.generate_embeddings_batch(&chunk_texts).await?;
            for emb in &mut batch {
                *emb = raisin_hnsw::normalize_vector(emb);
            }
            batch
        } else {
            let mut emb = provider.generate_embedding(&chunk_texts[0]).await?;
            emb = raisin_hnsw::normalize_vector(&emb);
            vec![emb]
        };

        tracing::info!(
            node_id = %node_id,
            embedding_dims = embeddings[0].len(),
            total_chunks = total_chunks,
            text_length = text.len(),
            "Successfully generated and normalized {} embedding(s)",
            embeddings.len()
        );

        // Get embedding storage from RocksDB
        let embedding_storage =
            crate::repositories::RocksDBEmbeddingStorage::new(self.storage.db().clone());

        // Store each chunk and add to HNSW
        for ((chunk_content, chunk_index), embedding) in
            chunks.iter().zip(embeddings.into_iter())
        {
            let chunk_node_id = if total_chunks > 1 {
                format!("{}#{}", node_id, chunk_index)
            } else {
                node_id.clone()
            };

            #[allow(deprecated)]
            let embedding_data = EmbeddingData {
                vector: embedding.clone(),
                embedder_id: embedder_id.clone(),
                embedding_kind: raisin_ai::config::EmbeddingKind::Text,
                source_id: node_id.clone(),
                chunk_index: *chunk_index,
                total_chunks,
                chunk_content: Some(chunk_content.chars().take(200).collect()),
                generated_at: chrono::Utc::now(),
                text_hash: hash_text(chunk_content),
                // Legacy fields (deprecated)
                model: config.model.clone(),
                provider: config.provider.clone(),
            };

            embedding_storage.store_embedding(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &context.workspace_id,
                &chunk_node_id,
                &context.revision,
                &embedding_data,
            )?;

            // Add to HNSW index (use spawn_blocking as HNSW operations are sync)
            let engine_clone = Arc::clone(&self.hnsw_engine);
            let chunk_id = chunk_node_id.clone();
            let tenant_id = context.tenant_id.clone();
            let repo_id = context.repo_id.clone();
            let branch = context.branch.clone();
            let workspace_id = context.workspace_id.clone();
            let revision = context.revision;

            tokio::task::spawn_blocking(move || {
                engine_clone.add_embedding(
                    &tenant_id,
                    &repo_id,
                    &branch,
                    &workspace_id,
                    &chunk_id,
                    revision,
                    embedding,
                )
            })
            .await
            .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;
        }

        tracing::debug!(
            node_id = %node_id,
            total_chunks = total_chunks,
            "Added {} chunk(s) to HNSW index",
            total_chunks
        );

        Ok(())
    }

    /// Handle embedding deletion job
    ///
    /// This method:
    /// 1. Extracts node_id from JobType::EmbeddingDelete
    /// 2. Checks for existing chunk count via embedding storage
    /// 3. Removes all chunks (or single embedding) from HNSW index
    pub async fn handle_delete(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract node_id from job type
        let node_id = match &job.job_type {
            JobType::EmbeddingDelete { node_id } => node_id,
            _ => {
                return Err(Error::Validation(format!(
                    "Expected EmbeddingDelete job, got {}",
                    job.job_type
                )))
            }
        };

        tracing::debug!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            workspace_id = %context.workspace_id,
            node_id = %node_id,
            "Processing embedding deletion job"
        );

        // Check if this node had chunked embeddings by reading chunk 0's data
        let total_chunks = self.get_existing_chunk_count(node_id, context);

        let engine_clone = Arc::clone(&self.hnsw_engine);
        let node_id_clone = node_id.clone();
        let tenant_id = context.tenant_id.clone();
        let repo_id = context.repo_id.clone();
        let branch = context.branch.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // Remove the base node_id (for non-chunked or legacy embeddings)
            engine_clone.remove_embedding(&tenant_id, &repo_id, &branch, &node_id_clone)?;

            // Remove all chunk IDs if this was a chunked document
            if total_chunks > 1 {
                for i in 0..total_chunks {
                    let chunk_id = format!("{}#{}", node_id_clone, i);
                    engine_clone.remove_embedding(&tenant_id, &repo_id, &branch, &chunk_id)?;
                }
            }

            Ok(())
        })
        .await
        .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::debug!(
            node_id = %node_id,
            total_chunks = total_chunks,
            "Removed {} embedding(s) from HNSW index",
            total_chunks.max(1)
        );

        Ok(())
    }

    /// Look up the existing chunk count for a node from embedding storage.
    /// Returns 1 if no chunked embedding is found (single or legacy).
    fn get_existing_chunk_count(&self, node_id: &str, context: &JobContext) -> usize {
        let embedding_storage =
            crate::repositories::RocksDBEmbeddingStorage::new(self.storage.db().clone());

        // Try chunk 0 first (chunked format: {node_id}#0)
        if let Ok(Some(data)) = embedding_storage.get_embedding(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            &context.workspace_id,
            &format!("{}#0", node_id),
            None,
        ) {
            return data.total_chunks;
        }

        // Try base node_id (legacy non-chunked format)
        if let Ok(Some(data)) = embedding_storage.get_embedding(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            &context.workspace_id,
            node_id,
            None,
        ) {
            return data.total_chunks;
        }

        1
    }

    /// Remove old chunks from HNSW index before re-embedding.
    /// This handles the case where a node previously had N chunks but now has M.
    async fn remove_old_chunks_from_hnsw(
        &self,
        node_id: &str,
        context: &JobContext,
        _new_chunk_count: usize,
    ) -> Result<()> {
        let old_total = self.get_existing_chunk_count(node_id, context);

        if old_total <= 1 {
            // Single embedding or no prior embedding - the HNSW add() handles replacement
            return Ok(());
        }

        // Remove all old chunk entries from HNSW
        let engine_clone = Arc::clone(&self.hnsw_engine);
        let node_id_clone = node_id.to_string();
        let tenant_id = context.tenant_id.clone();
        let repo_id = context.repo_id.clone();
        let branch = context.branch.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // Remove base node_id (backward compat)
            engine_clone.remove_embedding(&tenant_id, &repo_id, &branch, &node_id_clone)?;
            // Remove all old chunks
            for i in 0..old_total {
                let chunk_id = format!("{}#{}", node_id_clone, i);
                engine_clone.remove_embedding(&tenant_id, &repo_id, &branch, &chunk_id)?;
            }
            Ok(())
        })
        .await
        .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::debug!(
            node_id = %node_id,
            old_chunks = old_total,
            "Removed old chunks from HNSW index before re-embedding"
        );

        Ok(())
    }

    /// Handle branch copy job
    ///
    /// This method:
    /// 1. Extracts source_branch from JobType::EmbeddingBranchCopy
    /// 2. Copies HNSW index for the new branch
    pub async fn handle_branch_copy(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract source_branch from job type
        let source_branch = match &job.job_type {
            JobType::EmbeddingBranchCopy { source_branch } => source_branch,
            _ => {
                return Err(Error::Validation(format!(
                    "Expected EmbeddingBranchCopy job, got {}",
                    job.job_type
                )))
            }
        };

        tracing::debug!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            source_branch = %source_branch,
            new_branch = %context.branch,
            workspace_id = %context.workspace_id,
            "Processing embedding branch copy job"
        );

        let engine_clone = Arc::clone(&self.hnsw_engine);
        let tenant_id = context.tenant_id.clone();
        let repo_id = context.repo_id.clone();
        let new_branch = context.branch.clone();
        let source_branch_clone = source_branch.clone();

        tokio::task::spawn_blocking(move || {
            engine_clone.copy_for_branch(&tenant_id, &repo_id, &source_branch_clone, &new_branch)
        })
        .await
        .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::info!(
            branch = %context.branch,
            source_branch = %source_branch,
            "Copied HNSW index for new branch"
        );

        Ok(())
    }

    /// Resolve the embedding provider configuration.
    ///
    /// This method supports two modes:
    /// 1. **Unified Provider (preferred)**: If `ai_provider_ref` is set in the embedding config,
    ///    look up the provider and API key from `TenantAIConfig`.
    /// 2. **Legacy Mode**: Use the `provider`, `model`, and `api_key_encrypted` fields directly
    ///    from `TenantEmbeddingConfig`.
    ///
    /// Returns: (EmbeddingProvider, api_key, model)
    async fn resolve_embedding_provider(
        &self,
        config: &TenantEmbeddingConfig,
        tenant_id: &str,
    ) -> Result<(EmbeddingProvider, String, String)> {
        let encryptor = ApiKeyEncryptor::new(&self.master_key);

        if config.uses_unified_provider() {
            // Unified provider mode - look up from TenantAIConfig
            let provider_ref = config.ai_provider_ref.as_ref().unwrap();
            let model_ref = config.ai_model_ref.as_ref().cloned().unwrap_or_else(|| {
                // Default model based on provider
                match provider_ref.as_str() {
                    "openai" => "text-embedding-3-small".to_string(),
                    "ollama" => "nomic-embed-text".to_string(),
                    _ => "text-embedding-3-small".to_string(),
                }
            });

            // Get TenantAIConfig
            let ai_config_repo = self.storage.tenant_ai_config_repository();
            let ai_config = ai_config_repo.get_config(tenant_id).await.map_err(|e| {
                Error::Backend(format!(
                    "Failed to get AI config for unified provider: {}",
                    e
                ))
            })?;

            // Find the provider in TenantAIConfig
            let ai_provider = ai_config
                .providers
                .iter()
                .find(|p| format!("{:?}", p.provider).to_lowercase() == *provider_ref)
                .ok_or_else(|| {
                    Error::Validation(format!(
                        "AI provider '{}' not found in tenant config",
                        provider_ref
                    ))
                })?;

            // Decrypt API key from TenantAIConfig
            let api_key = if let Some(encrypted) = &ai_provider.api_key_encrypted {
                encryptor
                    .decrypt(encrypted)
                    .map_err(|e| Error::Backend(format!("Failed to decrypt API key: {}", e)))?
            } else {
                return Err(Error::Validation(format!(
                    "No API key configured for provider '{}'",
                    provider_ref
                )));
            };

            // Map TenantAIConfig provider to EmbeddingProvider
            let embedding_provider = match provider_ref.as_str() {
                "openai" => EmbeddingProvider::OpenAI,
                "anthropic" | "claude" => EmbeddingProvider::Claude,
                "ollama" => EmbeddingProvider::Ollama,
                _ => {
                    return Err(Error::Validation(format!(
                        "Provider '{}' does not support embeddings",
                        provider_ref
                    )))
                }
            };

            tracing::debug!(
                provider = %provider_ref,
                model = %model_ref,
                "Using unified AI provider for embeddings"
            );

            Ok((embedding_provider, api_key, model_ref))
        } else {
            // Legacy mode - use fields directly from TenantEmbeddingConfig
            let api_key_encrypted = config.api_key_encrypted.as_ref().ok_or_else(|| {
                Error::Validation("No API key configured for embeddings".to_string())
            })?;
            let api_key = encryptor
                .decrypt(api_key_encrypted)
                .map_err(|e| Error::Backend(format!("Failed to decrypt API key: {}", e)))?;

            tracing::debug!(
                provider = ?config.provider,
                model = %config.model,
                "Using legacy embedding provider configuration"
            );

            Ok((config.provider.clone(), api_key, config.model.clone()))
        }
    }
}
