//! HuggingFace model management job handlers
//!
//! Handles downloading and deleting HuggingFace models for local AI inference.

use raisin_ai::ModelRegistry;
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Result of a HuggingFace model download operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HuggingFaceModelDownloadResult {
    /// The model ID that was downloaded
    pub model_id: String,
    /// Path where the model was stored
    pub model_path: PathBuf,
    /// Size of the downloaded model in bytes
    pub size_bytes: u64,
}

/// Result of a HuggingFace model delete operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HuggingFaceModelDeleteResult {
    /// The model ID that was deleted
    pub model_id: String,
    /// Whether the deletion was successful
    pub success: bool,
}

/// Combined handler for all HuggingFace model operations
///
/// Uses lazy initialization for the model registry, creating it on first use.
pub struct HuggingFaceModelHandler {
    registry: Arc<RwLock<Option<ModelRegistry>>>,
}

impl HuggingFaceModelHandler {
    /// Create a new combined handler with lazy registry initialization
    pub fn new() -> Self {
        Self {
            registry: Arc::new(RwLock::new(None)),
        }
    }

    /// Ensure the model registry is initialized
    async fn ensure_registry(&self) -> Result<()> {
        let mut registry = self.registry.write().await;
        if registry.is_none() {
            let new_registry = ModelRegistry::new()
                .map_err(|e| Error::Backend(format!("Failed to create model registry: {}", e)))?;
            new_registry.refresh_download_status().await;
            *registry = Some(new_registry);
        }
        Ok(())
    }

    /// Handle a HuggingFace model download job
    pub async fn handle_download(
        &self,
        job: &JobInfo,
        _context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let model_id = match &job.job_type {
            JobType::HuggingFaceModelDownload { model_id } => model_id.clone(),
            _ => {
                return Err(Error::Validation(format!(
                    "Expected HuggingFaceModelDownload job, got: {}",
                    job.job_type
                )))
            }
        };

        tracing::info!(
            job_id = %job.id,
            model_id = %model_id,
            "Starting HuggingFace model download"
        );

        // Ensure registry is initialized
        self.ensure_registry().await?;

        // Get the registry
        let registry_guard = self.registry.read().await;
        let registry = registry_guard
            .as_ref()
            .ok_or_else(|| Error::Backend("Model registry not initialized".to_string()))?;

        // Check if model exists in registry
        if registry.get_model(&model_id).await.is_none() {
            return Err(Error::NotFound(format!(
                "Model not found in registry: {}",
                model_id
            )));
        }

        // Download the model
        let model_path = match registry.download_model(&model_id, None).await {
            Ok(path) => path,
            Err(e) => {
                tracing::error!(
                    job_id = %job.id,
                    model_id = %model_id,
                    error = %e,
                    "Failed to download HuggingFace model"
                );
                return Err(Error::Backend(format!(
                    "Failed to download model {}: {}",
                    model_id, e
                )));
            }
        };

        // Get the model info for size
        let size_bytes = registry
            .get_model(&model_id)
            .await
            .and_then(|m| m.actual_size_bytes)
            .unwrap_or(0);

        tracing::info!(
            job_id = %job.id,
            model_id = %model_id,
            model_path = %model_path.display(),
            size_bytes = size_bytes,
            "HuggingFace model download complete"
        );

        let result = HuggingFaceModelDownloadResult {
            model_id,
            model_path,
            size_bytes,
        };

        Ok(Some(serde_json::to_value(result).unwrap_or_default()))
    }

    /// Handle a HuggingFace model delete job
    pub async fn handle_delete(
        &self,
        job: &JobInfo,
        _context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let model_id = match &job.job_type {
            JobType::HuggingFaceModelDelete { model_id } => model_id.clone(),
            _ => {
                return Err(Error::Validation(format!(
                    "Expected HuggingFaceModelDelete job, got: {}",
                    job.job_type
                )))
            }
        };

        tracing::info!(
            job_id = %job.id,
            model_id = %model_id,
            "Starting HuggingFace model deletion"
        );

        // Ensure registry is initialized
        self.ensure_registry().await?;

        let registry_guard = self.registry.read().await;
        let registry = registry_guard
            .as_ref()
            .ok_or_else(|| Error::Backend("Model registry not initialized".to_string()))?;

        match registry.delete_model(&model_id).await {
            Ok(()) => {
                tracing::info!(
                    job_id = %job.id,
                    model_id = %model_id,
                    "HuggingFace model deleted successfully"
                );

                let result = HuggingFaceModelDeleteResult {
                    model_id,
                    success: true,
                };

                Ok(Some(serde_json::to_value(result).unwrap_or_default()))
            }
            Err(e) => {
                tracing::error!(
                    job_id = %job.id,
                    model_id = %model_id,
                    error = %e,
                    "Failed to delete HuggingFace model"
                );

                Err(Error::Backend(format!(
                    "Failed to delete model {}: {}",
                    model_id, e
                )))
            }
        }
    }
}

impl Default for HuggingFaceModelHandler {
    fn default() -> Self {
        Self::new()
    }
}
