//! AssetProcessingHandler struct, model management, and job handling

use raisin_ai::{BlipCaptioner, ClipEmbedder, ModelRegistry, MoondreamCaptioner};
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::captioner::CachedCaptioner;
use super::helpers::{extract_mime_type, extract_storage_key, is_image_mime, process_pdf};
use super::types::{AssetProcessingResult, BinaryRetrievalCallback};
use crate::RocksDBStorage;

/// Handler for automatic asset processing jobs.
///
/// # Deprecation Notice
///
/// This handler is deprecated in favor of user-defined trigger functions that use
/// the Resource API and `raisin.ai.*` SDK methods.
///
/// See `examples/launchpad/package/content/functions/lib/launchpad/process-asset/`
#[deprecated(
    since = "0.12.0",
    note = "Use Resource API and raisin.ai.* with user-defined triggers instead. See process-asset example."
)]
pub struct AssetProcessingHandler {
    storage: Arc<RocksDBStorage>,
    binary_callback: Option<BinaryRetrievalCallback>,
    model_registry: Arc<RwLock<Option<ModelRegistry>>>,
    clip_embedder: Arc<RwLock<Option<ClipEmbedder>>>,
    captioner_cache: Arc<RwLock<Option<CachedCaptioner>>>,
}

impl AssetProcessingHandler {
    /// Create a new asset processing handler
    pub fn new(storage: Arc<RocksDBStorage>) -> Self {
        Self {
            storage,
            binary_callback: None,
            model_registry: Arc::new(RwLock::new(None)),
            clip_embedder: Arc::new(RwLock::new(None)),
            captioner_cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the binary retrieval callback
    pub fn with_binary_callback(mut self, callback: BinaryRetrievalCallback) -> Self {
        self.binary_callback = Some(callback);
        self
    }

    /// Set the binary retrieval callback (mutable)
    pub fn set_binary_callback(&mut self, callback: BinaryRetrievalCallback) {
        self.binary_callback = Some(callback);
    }

    /// Initialize the model registry
    async fn ensure_model_registry(&self) -> Result<()> {
        let mut registry = self.model_registry.write().await;
        if registry.is_none() {
            let new_registry = ModelRegistry::new()
                .map_err(|e| Error::Backend(format!("Failed to create model registry: {}", e)))?;
            new_registry.refresh_download_status().await;
            *registry = Some(new_registry);
        }
        Ok(())
    }

    /// Get or download and load the CLIP embedder
    async fn get_or_load_clip(&self) -> Result<()> {
        {
            let embedder = self.clip_embedder.read().await;
            if embedder.is_some() {
                return Ok(());
            }
        }

        self.ensure_model_registry().await?;

        let model_id = "openai/clip-vit-base-patch32";
        let model_path: PathBuf;

        {
            let registry_guard = self.model_registry.read().await;
            let registry = registry_guard
                .as_ref()
                .ok_or_else(|| Error::Backend("Model registry not initialized".to_string()))?;

            if !registry.is_model_ready(model_id).await {
                drop(registry_guard);

                tracing::info!(model_id = %model_id, "Downloading CLIP model on-demand");
                let registry_guard = self.model_registry.read().await;
                let registry = registry_guard
                    .as_ref()
                    .ok_or_else(|| Error::Backend("Model registry not initialized".to_string()))?;

                let path = registry
                    .download_model(model_id, None)
                    .await
                    .map_err(|e| Error::Backend(format!("Failed to download CLIP model: {}", e)))?;
                model_path = path;
            } else {
                model_path = registry.model_path(model_id);
            }
        }

        let device = raisin_ai::select_device(true)
            .map_err(|e| Error::Backend(format!("Failed to select device: {}", e)))?;
        tracing::info!(model_id = %model_id, device = ?device, "Loading CLIP embedder");

        let embedder = ClipEmbedder::with_model_id(&model_path, device, model_id.to_string())
            .map_err(|e| Error::Backend(format!("Failed to load CLIP embedder: {}", e)))?;

        let mut clip_guard = self.clip_embedder.write().await;
        *clip_guard = Some(embedder);

        Ok(())
    }

    /// Get or download and load the captioner for the specified model
    async fn get_or_load_captioner(&self, requested_model_id: Option<&str>) -> Result<String> {
        let model_id = requested_model_id.unwrap_or(raisin_ai::default_caption_model());

        {
            let captioner = self.captioner_cache.read().await;
            if let Some(ref cached) = *captioner {
                if cached.model_id() == model_id {
                    return Ok(model_id.to_string());
                }
                tracing::info!(
                    cached_model = %cached.model_id(),
                    requested_model = %model_id,
                    "Captioner model changed, will reload"
                );
            }
        }

        self.ensure_model_registry().await?;

        let model_path: PathBuf;

        {
            let registry_guard = self.model_registry.read().await;
            let registry = registry_guard
                .as_ref()
                .ok_or_else(|| Error::Backend("Model registry not initialized".to_string()))?;

            if !registry.is_model_ready(model_id).await {
                drop(registry_guard);

                tracing::info!(model_id = %model_id, "Downloading captioning model on-demand");
                let registry_guard = self.model_registry.read().await;
                let registry = registry_guard
                    .as_ref()
                    .ok_or_else(|| Error::Backend("Model registry not initialized".to_string()))?;

                let path = registry.download_model(model_id, None).await.map_err(|e| {
                    Error::Backend(format!("Failed to download captioning model: {}", e))
                })?;
                model_path = path;
            } else {
                model_path = registry.model_path(model_id);
            }
        }

        let device = raisin_ai::select_device(true)
            .map_err(|e| Error::Backend(format!("Failed to select device: {}", e)))?;
        tracing::info!(model_id = %model_id, device = ?device, "Loading captioner");

        let cached = if raisin_ai::is_moondream_model(model_id) {
            let captioner =
                MoondreamCaptioner::with_model_id(&model_path, device, model_id.to_string())
                    .map_err(|e| {
                        Error::Backend(format!("Failed to load Moondream captioner: {}", e))
                    })?;
            CachedCaptioner::Moondream {
                captioner,
                model_id: model_id.to_string(),
            }
        } else if raisin_ai::is_blip_model(model_id) {
            let captioner = BlipCaptioner::with_model_id(&model_path, device, model_id.to_string())
                .map_err(|e| Error::Backend(format!("Failed to load BLIP captioner: {}", e)))?;
            CachedCaptioner::Blip {
                captioner,
                model_id: model_id.to_string(),
            }
        } else {
            return Err(Error::Backend(format!(
                "Unsupported caption model: '{}'. Supported: Moondream or BLIP.",
                model_id
            )));
        };

        let mut cache_guard = self.captioner_cache.write().await;
        *cache_guard = Some(cached);

        Ok(model_id.to_string())
    }

    /// Generate image embedding using CLIP
    async fn generate_image_embedding(&self, image_bytes: &[u8]) -> Result<Vec<f32>> {
        self.get_or_load_clip().await?;

        let clip_guard = self.clip_embedder.read().await;
        let embedder = clip_guard
            .as_ref()
            .ok_or_else(|| Error::Backend("CLIP embedder not loaded".to_string()))?;

        embedder
            .embed_image(image_bytes)
            .map_err(|e| Error::Backend(format!("CLIP embedding failed: {}", e)))
    }

    /// Generate image caption using the specified model
    async fn generate_image_caption(
        &self,
        image_bytes: &[u8],
        model_id: Option<&str>,
        alt_text_prompt: Option<&str>,
        description_prompt: Option<&str>,
    ) -> Result<(String, String, String)> {
        let actual_model = self.get_or_load_captioner(model_id).await?;

        let mut cache_guard = self.captioner_cache.write().await;
        let cached = cache_guard
            .as_mut()
            .ok_or_else(|| Error::Backend("Captioner not loaded".to_string()))?;

        let (caption, alt_text) = cached
            .generate(image_bytes, alt_text_prompt, description_prompt)
            .map_err(|e| Error::Backend(format!("Image captioning failed: {}", e)))?;

        Ok((caption, alt_text, actual_model))
    }

    /// Generate keywords for an image using the specified model
    async fn generate_image_keywords(
        &self,
        image_bytes: &[u8],
        model_id: Option<&str>,
        keywords_prompt: Option<&str>,
    ) -> Result<Vec<String>> {
        let _actual_model = self.get_or_load_captioner(model_id).await?;

        let mut cache_guard = self.captioner_cache.write().await;
        let cached = cache_guard
            .as_mut()
            .ok_or_else(|| Error::Backend("Captioner not loaded".to_string()))?;

        let keywords = cached
            .generate_keywords(image_bytes, keywords_prompt)
            .map_err(|e| Error::Backend(format!("Keyword extraction failed: {}", e)))?;

        Ok(keywords)
    }

    /// Handle an asset processing job
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let (node_id, options) = match &job.job_type {
            JobType::AssetProcessing { node_id, options } => (node_id.clone(), options.clone()),
            _ => {
                return Err(Error::Validation(format!(
                    "Expected AssetProcessing job, got: {}",
                    job.job_type
                )))
            }
        };

        tracing::info!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            node_id = %node_id,
            extract_pdf = options.extract_pdf_text,
            gen_img_embed = options.generate_image_embedding,
            gen_img_caption = options.generate_image_caption,
            "Processing asset"
        );

        let mut result = AssetProcessingResult {
            node_id: node_id.clone(),
            ..Default::default()
        };

        // Get the node from storage
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
                &node_id,
                Some(&context.revision),
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node not found: {}", node_id)))?;

        let mime_type = extract_mime_type(&node);

        // Get binary data if we have a callback
        let binary_data = self.retrieve_binary_data(job, &node_id, &node).await;

        // Process based on mime type and options
        if let Some(ref data) = binary_data {
            // PDF processing
            if mime_type.as_deref() == Some("application/pdf") && options.extract_pdf_text {
                match process_pdf(data, &options).await {
                    Ok(pdf_result) => {
                        result.extracted_text = Some(pdf_result.text);
                        result.pdf_page_count = Some(pdf_result.page_count);
                        result.used_ocr = pdf_result.used_ocr;

                        tracing::info!(
                            job_id = %job.id,
                            node_id = %node_id,
                            page_count = pdf_result.page_count,
                            used_ocr = pdf_result.used_ocr,
                            text_length = result.extracted_text.as_ref().map(|t| t.len()).unwrap_or(0),
                            "PDF text extraction complete"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            job_id = %job.id, node_id = %node_id, error = %e,
                            "PDF text extraction failed"
                        );
                    }
                }
            }

            // Image processing (CLIP embeddings)
            if is_image_mime(&mime_type) && options.generate_image_embedding {
                self.process_image_embedding(job, &node_id, data, &mut result)
                    .await;
            }

            // AI captioning is disabled - use trigger functions instead
            self.log_deprecated_captioning_warning(job, &node_id, &mime_type, &options);
        }

        tracing::info!(
            job_id = %job.id,
            node_id = %node_id,
            has_extracted_text = result.extracted_text.is_some(),
            has_caption = result.caption.is_some(),
            has_keywords = result.keywords.is_some(),
            "Asset processing complete"
        );

        Ok(Some(serde_json::to_value(result).unwrap_or_default()))
    }

    /// Retrieve binary data from storage using the callback
    async fn retrieve_binary_data(
        &self,
        job: &JobInfo,
        node_id: &str,
        node: &raisin_models::nodes::Node,
    ) -> Option<Vec<u8>> {
        let callback = self.binary_callback.as_ref()?;

        match extract_storage_key(node) {
            Ok(storage_key) => match callback(storage_key.clone()).await {
                Ok(data) => {
                    tracing::debug!(
                        job_id = %job.id, node_id = %node_id,
                        storage_key = %storage_key, data_size = data.len(),
                        "Retrieved binary data"
                    );
                    Some(data)
                }
                Err(e) => {
                    tracing::warn!(
                        job_id = %job.id, node_id = %node_id,
                        storage_key = %storage_key, error = %e,
                        "Failed to retrieve binary data"
                    );
                    None
                }
            },
            Err(e) => {
                tracing::debug!(
                    job_id = %job.id, node_id = %node_id, error = %e,
                    "No storage key found in node"
                );
                None
            }
        }
    }

    /// Process image embedding using CLIP
    async fn process_image_embedding(
        &self,
        job: &JobInfo,
        node_id: &str,
        data: &[u8],
        result: &mut AssetProcessingResult,
    ) {
        tracing::info!(
            job_id = %job.id, node_id = %node_id,
            "Generating image embedding with CLIP"
        );

        match self.generate_image_embedding(data).await {
            Ok(embedding) => {
                result.image_embedding_generated = true;
                result.image_embedding_dim = Some(embedding.len());
                result.image_embedding = Some(embedding);

                tracing::info!(
                    job_id = %job.id, node_id = %node_id,
                    embedding_dim = result.image_embedding_dim,
                    "Image embedding generated successfully"
                );
            }
            Err(e) => {
                tracing::error!(
                    job_id = %job.id, node_id = %node_id, error = %e,
                    "Failed to generate image embedding"
                );
            }
        }
    }

    /// Log deprecation warning for captioning
    fn log_deprecated_captioning_warning(
        &self,
        job: &JobInfo,
        node_id: &str,
        mime_type: &Option<String>,
        options: &raisin_storage::jobs::AssetProcessingOptions,
    ) {
        let would_caption = is_image_mime(mime_type) && options.generate_image_caption;
        let would_generate_keywords = is_image_mime(mime_type) && options.generate_keywords;

        if would_caption || would_generate_keywords {
            tracing::warn!(
                job_id = %job.id, node_id = %node_id,
                "AI captioning/keywords generation is disabled in AssetProcessingHandler. \
                 Use trigger functions with raisin.ai.completion() instead."
            );
        }
    }
}
