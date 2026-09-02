//! Model download functionality for the HuggingFace registry.

use std::path::PathBuf;

use super::super::types::*;
use super::ModelRegistry;

#[cfg(feature = "huggingface")]
use super::calculate_dir_size;

impl ModelRegistry {
    /// Download a model from HuggingFace Hub.
    ///
    /// # Arguments
    ///
    /// * `model_id` - The HuggingFace model ID (e.g., "openai/clip-vit-base-patch32")
    /// * `progress_callback` - Optional callback for progress updates
    ///
    /// # Returns
    ///
    /// Returns the path to the downloaded model directory.
    #[cfg(feature = "huggingface")]
    pub async fn download_model(
        &self,
        model_id: &str,
        progress_callback: Option<ProgressCallback>,
    ) -> ModelResult<PathBuf> {
        use hf_hub::api::tokio::ApiBuilder;
        use hf_hub::Cache;

        // Check if model is in our registry
        {
            let models = self.models.read().await;
            if !models.contains_key(model_id) {
                return Err(ModelError::NotFound(model_id.to_string()));
            }
        }

        // Update status to downloading
        {
            let mut models = self.models.write().await;
            if let Some(model) = models.get_mut(model_id) {
                if model.status.is_downloading() {
                    return Err(ModelError::AlreadyDownloading(model_id.to_string()));
                }
                model.status = DownloadStatus::Downloading {
                    progress: 0.0,
                    downloaded_bytes: 0,
                    total_bytes: model.estimated_size_bytes,
                };
            }
        }

        tracing::info!(model_id = %model_id, "Starting model download");

        let model_path = self.model_path(model_id);
        std::fs::create_dir_all(&model_path)?;

        // Check if this is a quantized model
        let (is_quantized, gguf_filename, tokenizer_repo) = {
            let models = self.models.read().await;
            models
                .get(model_id)
                .map(|m| {
                    (
                        m.is_quantized,
                        m.gguf_filename.clone(),
                        m.tokenizer_repo.clone(),
                    )
                })
                .unwrap_or((false, None, None))
        };

        // Set up HuggingFace cache in our directory
        let cache = Cache::new(self.cache_dir.clone());
        let api = ApiBuilder::from_cache(cache)
            .build()
            .map_err(|e| ModelError::HubError(e.to_string()))?;

        let repo = api.model(model_id.to_string());

        // Handle quantized models differently - they use GGUF format
        if is_quantized {
            return self
                .download_quantized_model(
                    &api,
                    tokenizer_repo,
                    model_id,
                    &repo,
                    gguf_filename,
                    &model_path,
                    progress_callback,
                )
                .await;
        }

        // Standard model download (safetensors/pytorch)
        self.download_standard_model(model_id, &repo, &model_path, progress_callback)
            .await
    }

    /// Download a quantized (GGUF) model.
    #[cfg(feature = "huggingface")]
    async fn download_quantized_model(
        &self,
        api: &hf_hub::api::tokio::Api,
        tokenizer_repo: Option<String>,
        model_id: &str,
        repo: &hf_hub::api::tokio::ApiRepo,
        gguf_filename: Option<String>,
        model_path: &std::path::Path,
        progress_callback: Option<ProgressCallback>,
    ) -> ModelResult<PathBuf> {
        let gguf_file = gguf_filename.ok_or_else(|| {
            ModelError::DownloadFailed("Quantized model missing GGUF filename".to_string())
        })?;

        tracing::info!(model_id = %model_id, gguf_file = %gguf_file, "Downloading quantized GGUF model");

        // Download GGUF file
        let gguf_path = repo
            .get(&gguf_file)
            .await
            .map_err(|e| ModelError::DownloadFailed(format!("GGUF download failed: {}", e)))?;

        if let Some(cb) = &progress_callback {
            cb(0.8);
        }

        /* THE TOKENIZER IS OFTEN IN A DIFFERENT REPO.
         *
         * A GGUF embeds its own tokenizer, so a GGUF-only repo has no reason to
         * ship `tokenizer.json` — Qwen's is literally one file. A loader that
         * needs the JSON therefore has to look in the base repo, and the model
         * entry names it.
         *
         * The file is COPIED in beside the weights rather than left in the other
         * repo's cache directory, so the model directory is self-contained and
         * whatever loads it can find both files in one place. A failure here is
         * fatal rather than a warning: a quantized model with no tokenizer
         * downloads "successfully" and then fails at load time with a missing
         * file, which is a much worse place to learn about it. */
        let tokenizer_source = match &tokenizer_repo {
            Some(other) => api.model(other.clone()).get("tokenizer.json").await,
            None => repo.get("tokenizer.json").await,
        };

        match tokenizer_source {
            Ok(src) => {
                let dest = gguf_path
                    .parent()
                    .unwrap_or(model_path)
                    .join("tokenizer.json");
                if src != dest && !dest.exists() {
                    // Copy, not symlink: hf-hub already stores blobs behind
                    // symlinks, and a link to a link across cache entries breaks
                    // the moment either is garbage-collected.
                    if let Err(e) = std::fs::copy(&src, &dest) {
                        tracing::warn!(model_id = %model_id, error = %e, "Could not place tokenizer beside the weights");
                    }
                }
            }
            Err(e) => {
                return Err(ModelError::DownloadFailed(format!(
                    "Tokenizer download failed for {} (from {}): {}. A quantized model without \
                     tokenizer.json cannot be loaded.",
                    model_id,
                    tokenizer_repo.as_deref().unwrap_or(model_id),
                    e
                )));
            }
        }

        if let Some(cb) = &progress_callback {
            cb(1.0);
        }

        // Update status to ready
        let mut models = self.models.write().await;
        if let Some(model) = models.get_mut(model_id) {
            let size = calculate_dir_size(model_path);
            model.actual_size_bytes = Some(size);
            model.status = DownloadStatus::Ready;
        }

        tracing::info!(model_id = %model_id, "Quantized model download complete");

        // Return the HF Hub cache path where files are stored
        Ok(gguf_path.parent().unwrap_or(model_path).to_path_buf())
    }

    /// Download a standard (safetensors/pytorch) model.
    #[cfg(feature = "huggingface")]
    async fn download_standard_model(
        &self,
        model_id: &str,
        repo: &hf_hub::api::tokio::ApiRepo,
        model_path: &std::path::Path,
        progress_callback: Option<ProgressCallback>,
    ) -> ModelResult<PathBuf> {
        let result = repo
            .get("config.json")
            .await
            .map_err(|e| ModelError::DownloadFailed(e.to_string()));

        match result {
            Ok(path) => {
                // Report progress
                if let Some(cb) = &progress_callback {
                    cb(0.5);
                }

                // Download main model files
                // Try multiple safetensors filenames (LAION uses open_clip_model.safetensors)
                let safetensors_result = repo.get("model.safetensors").await;
                let open_clip_result = repo.get("open_clip_model.safetensors").await;
                let has_safetensors = safetensors_result.is_ok() || open_clip_result.is_ok();

                // Fallback: try pytorch format if safetensors not available
                let pytorch_result = repo.get("pytorch_model.bin").await;
                let has_pytorch = pytorch_result.is_ok();

                if !has_safetensors && !has_pytorch {
                    tracing::warn!(
                        model_id = %model_id,
                        "No model weights found (tried safetensors and pytorch)"
                    );
                } else if !has_safetensors {
                    tracing::warn!(
                        model_id = %model_id,
                        "No safetensors found, pytorch_model.bin available but requires conversion"
                    );
                }

                // Optional files
                let _ = repo.get("tokenizer.json").await;
                let _ = repo.get("vocab.txt").await;

                if let Some(cb) = &progress_callback {
                    cb(1.0);
                }

                // Update status to ready
                let mut models = self.models.write().await;
                if let Some(model) = models.get_mut(model_id) {
                    let size = calculate_dir_size(model_path);
                    model.actual_size_bytes = Some(size);
                    model.status = DownloadStatus::Ready;
                }

                tracing::info!(model_id = %model_id, "Model download complete");

                // Return the HF Hub cache path where files are stored
                Ok(path.parent().unwrap_or(model_path).to_path_buf())
            }
            Err(e) => {
                // Update status to failed
                let mut models = self.models.write().await;
                if let Some(model) = models.get_mut(model_id) {
                    model.status = DownloadStatus::Failed {
                        error: e.to_string(),
                    };
                }
                Err(e)
            }
        }
    }

    /// Download a model (stub for when huggingface feature is disabled).
    #[cfg(not(feature = "huggingface"))]
    pub async fn download_model(
        &self,
        model_id: &str,
        _progress_callback: Option<ProgressCallback>,
    ) -> ModelResult<PathBuf> {
        Err(ModelError::HubError(format!(
            "HuggingFace feature not enabled. Cannot download model: {}",
            model_id
        )))
    }
}
