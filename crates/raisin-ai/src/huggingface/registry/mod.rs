//! HuggingFace model registry for managing local models.

mod default_models;
mod download;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::types::*;
use default_models::default_models;

/// Registry for managing HuggingFace models.
///
/// The registry tracks available models, their download status,
/// and manages the local cache directory.
pub struct ModelRegistry {
    /// Path to the model cache directory
    cache_dir: PathBuf,

    /// Available models (model_id -> info)
    models: Arc<RwLock<HashMap<String, ModelInfo>>>,
}

impl ModelRegistry {
    /// Create a new model registry with default cache directory.
    ///
    /// Uses `~/.cache/raisindb/models` as the default cache location.
    pub fn new() -> ModelResult<Self> {
        #[cfg(feature = "huggingface")]
        let cache_dir = {
            let base = dirs::cache_dir().ok_or(ModelError::CacheDirectoryNotFound)?;
            base.join("raisindb").join("models")
        };

        #[cfg(not(feature = "huggingface"))]
        let cache_dir = PathBuf::from(".cache/raisindb/models");

        Self::with_cache_dir(cache_dir)
    }

    /// Create a new model registry with a custom cache directory.
    pub fn with_cache_dir(cache_dir: PathBuf) -> ModelResult<Self> {
        // Create cache directory if it doesn't exist
        if !cache_dir.exists() {
            std::fs::create_dir_all(&cache_dir)?;
        }

        let mut models = HashMap::new();
        for model in default_models() {
            models.insert(model.model_id.clone(), model);
        }

        let registry = Self {
            cache_dir,
            models: Arc::new(RwLock::new(models)),
        };

        Ok(registry)
    }

    /// Create a new model registry and refresh download status.
    ///
    /// This is an async version that also checks which models are already downloaded.
    pub async fn with_cache_dir_async(cache_dir: PathBuf) -> ModelResult<Self> {
        let registry = Self::with_cache_dir(cache_dir)?;
        registry.refresh_download_status().await;
        Ok(registry)
    }

    /// Get the cache directory path.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// List all available models.
    pub async fn list_models(&self) -> Vec<ModelInfo> {
        let models = self.models.read().await;
        models.values().cloned().collect()
    }

    /// Get info for a specific model.
    pub async fn get_model(&self, model_id: &str) -> Option<ModelInfo> {
        let models = self.models.read().await;
        models.get(model_id).cloned()
    }

    /// Check if a model is downloaded and ready to use.
    pub async fn is_model_ready(&self, model_id: &str) -> bool {
        let models = self.models.read().await;
        models
            .get(model_id)
            .map(|m| m.is_downloaded())
            .unwrap_or(false)
    }

    /// Get the local path for a downloaded model.
    pub fn model_path(&self, model_id: &str) -> PathBuf {
        // Convert model_id to safe directory name
        // e.g., "openai/clip-vit-base-patch32" -> "openai--clip-vit-base-patch32"
        let safe_name = model_id.replace('/', "--");
        self.cache_dir.join(safe_name)
    }

    /// Refresh the download status for all models.
    ///
    /// Checks the cache directory to see which models are downloaded.
    pub async fn refresh_download_status(&self) {
        let mut models = self.models.write().await;

        for model in models.values_mut() {
            let model_path = self.model_path(&model.model_id);
            if model_path.exists() && model_path.is_dir() {
                // Check if there are files in the directory
                if let Ok(entries) = std::fs::read_dir(&model_path) {
                    let has_files = entries.flatten().count() > 0;
                    if has_files {
                        // Calculate actual size
                        let size = calculate_dir_size(&model_path);
                        model.actual_size_bytes = Some(size);
                        model.status = DownloadStatus::Ready;
                        continue;
                    }
                }
            }

            // Model is not downloaded
            if !model.status.is_downloading() {
                model.status = DownloadStatus::NotDownloaded;
            }
        }
    }

    /// Delete a downloaded model.
    pub async fn delete_model(&self, model_id: &str) -> ModelResult<()> {
        let model_path = self.model_path(model_id);

        if model_path.exists() {
            std::fs::remove_dir_all(&model_path)?;
        }

        // Update status
        let mut models = self.models.write().await;
        if let Some(model) = models.get_mut(model_id) {
            model.actual_size_bytes = None;
            model.status = DownloadStatus::NotDownloaded;
        }

        tracing::info!(model_id = %model_id, "Deleted model from cache");

        Ok(())
    }

    /// Get total disk usage of all downloaded models.
    pub async fn total_disk_usage(&self) -> u64 {
        let models = self.models.read().await;
        models.values().filter_map(|m| m.actual_size_bytes).sum()
    }

    /// Get total disk usage as a human-readable string.
    pub async fn disk_usage_display(&self) -> String {
        let bytes = self.total_disk_usage().await;
        if bytes >= 1_000_000_000 {
            format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
        } else if bytes >= 1_000_000 {
            format!("{:.1} MB", bytes as f64 / 1_000_000.0)
        } else if bytes >= 1_000 {
            format!("{:.1} KB", bytes as f64 / 1_000.0)
        } else {
            format!("{} B", bytes)
        }
    }

    /// Add a custom model to the registry.
    pub async fn register_model(&self, model: ModelInfo) {
        let mut models = self.models.write().await;
        models.insert(model.model_id.clone(), model);
    }
}

/// Calculate total size of a directory.
pub(super) fn calculate_dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }

    let mut size = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                size += calculate_dir_size(&path);
            } else if let Ok(metadata) = entry.metadata() {
                size += metadata.len();
            }
        }
    }
    size
}
