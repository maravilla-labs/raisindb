//! Background worker for processing embedding generation jobs.
//!
//! This worker follows the same pattern as IndexerWorker,
//! continuously polling for jobs and processing them asynchronously.

use raisin_ai::crypto::ApiKeyEncryptor;
use raisin_ai::{AIProvider, AIUseCase, EmbeddingSettings, TenantAIConfig, TenantAIConfigStore};
use raisin_embeddings::config::EmbeddingProvider;
use raisin_embeddings::models::{EmbeddingData, EmbeddingJob, EmbeddingJobKind};
use raisin_embeddings::provider::{create_provider, EmbeddingProvider as EmbeddingProviderTrait};
use raisin_embeddings::{EmbeddingJobStore, EmbeddingStorage};
use raisin_error::{Error, Result};
use raisin_hnsw::HnswIndexingEngine;
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{NodeRepository, Storage};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Configuration for the embedding worker
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub batch_size: usize,
    pub poll_interval: Duration,
    pub max_retries: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            batch_size: 5,
            poll_interval: Duration::from_secs(2),
            max_retries: 3,
        }
    }
}

/// Background worker that processes embedding generation jobs
///
/// Uses RocksDBStorage concretely (not generic) because we need access to
/// tenant_ai_config_repository() which is specific to RocksDB.
/// This is consistent with multi-tenant architecture where each job has a tenant_id
/// and we look up that tenant's configuration when processing.
pub struct EmbeddingWorker<E, J>
where
    E: EmbeddingStorage,
    J: EmbeddingJobStore,
{
    storage: Arc<RocksDBStorage>,
    embedding_storage: Arc<E>,
    job_store: Arc<J>,
    hnsw_engine: Arc<HnswIndexingEngine>,
    master_key: [u8; 32],
    config: WorkerConfig,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl<E, J> EmbeddingWorker<E, J>
where
    E: EmbeddingStorage + 'static,
    J: EmbeddingJobStore + 'static,
{
    pub fn new(
        storage: Arc<RocksDBStorage>,
        embedding_storage: Arc<E>,
        job_store: Arc<J>,
        hnsw_engine: Arc<HnswIndexingEngine>,
        master_key: [u8; 32],
        config: WorkerConfig,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            storage,
            embedding_storage,
            job_store,
            hnsw_engine,
            master_key,
            config,
            shutdown_tx,
            shutdown_rx,
        }
    }

    pub fn start(&self) -> JoinHandle<Result<()>> {
        let storage = Arc::clone(&self.storage);
        let embedding_storage = Arc::clone(&self.embedding_storage);
        let job_store = Arc::clone(&self.job_store);
        let hnsw_engine = Arc::clone(&self.hnsw_engine);
        let master_key = self.master_key;
        let config = self.config.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            Self::run_loop(
                storage,
                embedding_storage,
                job_store,
                hnsw_engine,
                master_key,
                config,
                &mut shutdown_rx,
            )
            .await
        })
    }

    pub async fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    async fn run_loop(
        storage: Arc<RocksDBStorage>,
        embedding_storage: Arc<E>,
        job_store: Arc<J>,
        hnsw_engine: Arc<HnswIndexingEngine>,
        master_key: [u8; 32],
        config: WorkerConfig,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) -> Result<()> {
        tracing::info!(
            batch_size = config.batch_size,
            poll_interval_ms = config.poll_interval.as_millis(),
            "Embedding worker started"
        );

        loop {
            if *shutdown_rx.borrow() {
                tracing::info!("Embedding worker shutting down");
                break;
            }

            let jobs = match job_store.dequeue(config.batch_size) {
                Ok(jobs) => jobs,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to dequeue embedding jobs");
                    tokio::time::sleep(config.poll_interval).await;
                    continue;
                }
            };

            if jobs.is_empty() {
                tokio::time::sleep(config.poll_interval).await;
                continue;
            }

            tracing::debug!(count = jobs.len(), "Processing embedding jobs");

            for job in jobs {
                if *shutdown_rx.borrow() {
                    tracing::info!("Shutdown requested, stopping job processing");
                    break;
                }

                Self::process_job(
                    &storage,
                    &embedding_storage,
                    &job_store,
                    &hnsw_engine,
                    master_key,
                    job,
                    config.max_retries,
                )
                .await;
            }
        }

        tracing::info!("Embedding worker stopped");
        Ok(())
    }

    async fn process_job(
        storage: &Arc<RocksDBStorage>,
        embedding_storage: &Arc<E>,
        job_store: &Arc<J>,
        hnsw_engine: &Arc<HnswIndexingEngine>,
        master_key: [u8; 32],
        job: EmbeddingJob,
        max_retries: usize,
    ) {
        let job_id = job.job_id.clone();

        tracing::debug!(
            job_id = %job_id,
            kind = ?job.kind,
            tenant_id = %job.tenant_id,
            repo_id = %job.repo_id,
            "Processing embedding job"
        );

        let mut last_error = None;

        for attempt in 0..=max_retries {
            if attempt > 0 {
                // Exponential backoff: 1s, 2s, 4s, ...
                let delay = Duration::from_secs(1 << (attempt - 1).min(5));
                tracing::warn!(
                    job_id = %job_id,
                    attempt = attempt + 1,
                    max_retries = max_retries,
                    delay_secs = delay.as_secs(),
                    "Retrying embedding job after backoff"
                );
                tokio::time::sleep(delay).await;
            }

            let result = match job.kind {
                EmbeddingJobKind::AddNode => {
                    Self::handle_add_node(
                        storage,
                        embedding_storage,
                        hnsw_engine,
                        master_key,
                        &job,
                    )
                    .await
                }
                EmbeddingJobKind::DeleteNode => {
                    Self::handle_delete_node(hnsw_engine, &job).await
                }
                EmbeddingJobKind::BranchCreated => {
                    Self::handle_branch_created(hnsw_engine, &job).await
                }
            };

            match result {
                Ok(()) => {
                    if let Err(e) = job_store.complete(&[job_id.clone()]) {
                        tracing::error!(job_id = %job_id, error = %e, "Failed to mark job as complete");
                    } else {
                        if attempt > 0 {
                            tracing::info!(
                                job_id = %job_id,
                                attempts = attempt + 1,
                                "Embedding job succeeded after retry"
                            );
                        } else {
                            tracing::debug!(job_id = %job_id, "Completed embedding job");
                        }
                    }
                    return;
                }
                Err(e) => {
                    // Don't retry validation errors (bad config, missing node, etc.)
                    if Self::is_permanent_error(&e) {
                        tracing::error!(
                            job_id = %job_id,
                            error = %e,
                            "Embedding job failed with permanent error, not retrying"
                        );
                        let _ = job_store.fail(&job_id, &e.to_string());
                        return;
                    }
                    last_error = Some(e);
                }
            }
        }

        // All retries exhausted
        if let Some(e) = last_error {
            tracing::error!(
                job_id = %job_id,
                error = %e,
                attempts = max_retries + 1,
                "Embedding job failed after all retries"
            );
            if let Err(mark_err) = job_store.fail(&job_id, &e.to_string()) {
                tracing::error!(
                    job_id = %job_id,
                    error = %mark_err,
                    "Failed to mark job as failed"
                );
            }
        }
    }

    /// Check if an error is permanent (should not be retried).
    /// Validation errors, missing configs, and not-found errors are permanent.
    /// Backend/storage errors and provider API errors are transient (retryable).
    fn is_permanent_error(e: &Error) -> bool {
        matches!(e, Error::Validation(_) | Error::NotFound(_))
    }

    async fn handle_add_node(
        storage: &Arc<RocksDBStorage>,
        embedding_storage: &Arc<E>,
        hnsw_engine: &Arc<HnswIndexingEngine>,
        master_key: [u8; 32],
        job: &EmbeddingJob,
    ) -> Result<()> {
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
            .and_then(|p| {
                p.get_default_model(AIUseCase::Embedding)
                    .map(|m| (p, m))
            })
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
                Some(job.revision),
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

        tracing::debug!(
            node_id = %node_id,
            embedding_dims = embedding.len(),
            text_length = text.len(),
            model = %model_config.model_id,
            provider = ?provider_config.provider,
            "Generated embedding"
        );

        // 10. Store in RocksDB embeddings CF
        let embedding_data = EmbeddingData {
            vector: embedding.clone(),
            model: model_config.model_id.clone(),
            provider: format!("{:?}", provider_config.provider),
            generated_at: chrono::Utc::now(),
            text_hash: hash_text(&text),
        };

        embedding_storage.store_embedding(
            &job.tenant_id,
            &job.repo_id,
            &job.branch,
            &job.workspace_id,
            node_id,
            job.revision,
            &embedding_data,
        )?;

        // 8. Add to HNSW index
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

    async fn handle_delete_node(
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
        let workspace_id = job.workspace_id.clone();

        tokio::task::spawn_blocking(move || {
            engine_clone.remove_embedding(&tenant_id, &repo_id, &branch, &workspace_id, &node_id_clone)
        })
        .await
        .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::debug!(node_id = %node_id, "Removed from HNSW index");

        Ok(())
    }

    async fn handle_branch_created(
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
        let workspace_id = job.workspace_id.clone();

        tokio::task::spawn_blocking(move || {
            engine_clone.copy_for_branch(&tenant_id, &repo_id, &source_branch_clone, &new_branch, &workspace_id)
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
}

/// Extract embeddable content from node based on settings
fn extract_embeddable_content(node: &Node, settings: &EmbeddingSettings) -> Result<String> {
    let mut parts = Vec::new();

    // 1. Include node name
    if settings.include_name {
        parts.push(node.name.clone());
    }

    // 2. Include node path
    if settings.include_path {
        parts.push(node.path.clone());
    }

    // Note: Per-node-type property selection is now handled via NodeType schema
    // This legacy worker extracts all string properties as a fallback
    for (_, prop_value) in &node.properties {
        if let Some(text) = property_value_to_text(prop_value) {
            parts.push(text);
        }
    }

    // Join all parts with newlines
    Ok(parts.join("\n"))
}

/// Convert property value to text for embedding
fn property_value_to_text(
    value: &raisin_models::nodes::properties::PropertyValue,
) -> Option<String> {
    use raisin_models::nodes::properties::PropertyValue;

    match value {
        PropertyValue::String(s) => Some(s.clone()),
        PropertyValue::Integer(i) => Some(i.to_string()),
        PropertyValue::Float(f) => Some(f.to_string()),
        PropertyValue::Boolean(b) => Some(b.to_string()),
        PropertyValue::Date(d) => Some(d.to_string()),
        PropertyValue::Url(u) => Some(u.url.clone()),
        PropertyValue::Array(arr) => {
            let texts: Vec<String> = arr.iter().filter_map(property_value_to_text).collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join(", "))
            }
        }
        PropertyValue::Object(obj) => {
            // Convert object to JSON string for embedding
            serde_json::to_string(obj).ok()
        }
        PropertyValue::Composite(bc) => {
            // Extract text from blocks
            let texts: Vec<String> = bc
                .items
                .iter()
                .filter_map(|block| {
                    // Try to extract text from block content
                    if let Some(text_prop) = block.content.get("text") {
                        property_value_to_text(text_prop)
                    } else {
                        None
                    }
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        PropertyValue::Element(block) => {
            // Extract text from single block
            if let Some(text_prop) = block.content.get("text") {
                property_value_to_text(text_prop)
            } else {
                None
            }
        }
        // Skip Reference and Resource types - they're not textual content
        _ => None,
    }
}

/// Hash text for detecting changes
fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Map AI provider enum to embedding provider enum.
///
/// This is a temporary function until raisin-ai fully supports embeddings.
/// Currently we use the new TenantAIConfig for configuration but still use
/// the old raisin-embeddings crate for actual embedding generation.
fn map_ai_provider_to_embedding_provider(provider: &AIProvider) -> Result<EmbeddingProvider> {
    match provider {
        AIProvider::OpenAI => Ok(EmbeddingProvider::OpenAI),
        AIProvider::Ollama => Ok(EmbeddingProvider::Ollama),
        AIProvider::Anthropic | AIProvider::Google | AIProvider::AzureOpenAI | AIProvider::Custom => {
            Err(Error::Validation(format!(
                "Provider {:?} does not support embeddings yet",
                provider
            )))
        }
    }
}
