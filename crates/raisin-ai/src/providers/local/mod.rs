//! Local Candle provider for in-process AI inference.
//!
//! This provider uses the Candle ML framework to run AI models locally,
//! without requiring external API calls. Supported models:
//!
//! - **Moondream**: Promptable vision-language model for image captioning
//! - **BLIP**: Fast image captioning (fallback)
//! - **CLIP**: Image embeddings for semantic search

mod model;
#[cfg(test)]
mod tests;
mod trait_impl;

pub use model::LocalModel;

use std::path::PathBuf;
#[cfg(feature = "candle")]
use std::sync::Mutex;

use tokio::sync::RwLock;

use crate::huggingface::ModelRegistry;
use crate::provider::{ProviderError, Result};
use crate::types::Message;

#[cfg(feature = "candle")]
use crate::candle::{
    select_device, BlipCaptioner, ClipEmbedder, MoondreamCaptioner, CLIP_EMBEDDING_DIM,
};

/// Local Candle provider for in-process AI inference.
///
/// This provider runs AI models locally using the Candle framework,
/// without requiring external API calls. Models are automatically
/// downloaded from HuggingFace if not present locally.
pub struct LocalCandleProvider {
    /// Base directory for model files
    models_dir: PathBuf,

    /// Model registry for auto-downloading models
    registry: RwLock<Option<ModelRegistry>>,

    /// Cached Moondream captioner (requires mutable access)
    #[cfg(feature = "candle")]
    moondream: Mutex<Option<MoondreamCaptioner>>,

    /// Cached BLIP captioner (requires mutable access)
    #[cfg(feature = "candle")]
    blip: Mutex<Option<BlipCaptioner>>,

    /// Cached CLIP embedder
    #[cfg(feature = "candle")]
    clip: Mutex<Option<ClipEmbedder>>,
}

impl LocalCandleProvider {
    /// Create a new local Candle provider.
    pub fn new(models_dir: impl Into<PathBuf>) -> Self {
        Self {
            models_dir: models_dir.into(),
            registry: RwLock::new(None),
            #[cfg(feature = "candle")]
            moondream: Mutex::new(None),
            #[cfg(feature = "candle")]
            blip: Mutex::new(None),
            #[cfg(feature = "candle")]
            clip: Mutex::new(None),
        }
    }

    /// Ensure the model registry is initialized.
    pub(crate) async fn ensure_registry(&self) -> Result<()> {
        let mut registry = self.registry.write().await;
        if registry.is_none() {
            let new_registry = ModelRegistry::new().map_err(|e| {
                ProviderError::ProviderNotAvailable(format!(
                    "Failed to create model registry: {}",
                    e
                ))
            })?;
            new_registry.refresh_download_status().await;
            *registry = Some(new_registry);
        }
        Ok(())
    }

    /// Ensure a model is downloaded and return its path.
    pub(crate) async fn ensure_model_downloaded(&self, local_model: LocalModel) -> Result<PathBuf> {
        #[cfg(feature = "candle")]
        {
            let hf_model_id = local_model.hf_model_id();
            let model_subdir = local_model.name();
            let model_path = self.models_dir.join(model_subdir);

            if model_path.exists() {
                tracing::debug!(
                    model = %model_subdir,
                    path = %model_path.display(),
                    "Model already available locally"
                );
                return Ok(model_path);
            }

            self.ensure_registry().await?;

            let registry_guard = self.registry.read().await;
            let registry = registry_guard.as_ref().ok_or_else(|| {
                ProviderError::ProviderNotAvailable("Model registry not initialized".to_string())
            })?;

            if registry.is_model_ready(hf_model_id).await {
                return Ok(registry.model_path(hf_model_id));
            }

            tracing::info!(
                model_id = %hf_model_id,
                target_path = %model_path.display(),
                "Downloading local AI model on-demand (this may take a few minutes)..."
            );

            let downloaded_path =
                registry
                    .download_model(hf_model_id, None)
                    .await
                    .map_err(|e| {
                        ProviderError::ProviderNotAvailable(format!(
                        "Failed to download model '{}': {}. Try downloading manually via Admin Console.",
                        hf_model_id, e
                    ))
                    })?;

            tracing::info!(
                model_id = %hf_model_id,
                path = %downloaded_path.display(),
                "Model downloaded successfully"
            );

            Ok(downloaded_path)
        }

        #[cfg(not(feature = "candle"))]
        {
            let _ = local_model;
            Err(ProviderError::ProviderNotAvailable(
                "Candle feature not enabled".to_string(),
            ))
        }
    }

    /// Get or create the Moondream captioner.
    #[cfg(feature = "candle")]
    pub(crate) fn get_moondream(
        &self,
        model_path: &PathBuf,
    ) -> Result<std::sync::MutexGuard<'_, Option<MoondreamCaptioner>>> {
        let mut guard = self.moondream.lock().map_err(|e| {
            ProviderError::Unknown(format!("Failed to lock Moondream mutex: {}", e))
        })?;

        if guard.is_none() {
            if !model_path.exists() {
                return Err(ProviderError::ProviderNotAvailable(format!(
                    "Moondream model not found at {:?}. Please download the model first.",
                    model_path
                )));
            }

            let device = select_device(true)
                .map_err(|e| ProviderError::ProviderNotAvailable(format!("Device error: {}", e)))?;

            let captioner = MoondreamCaptioner::new(model_path, device).map_err(|e| {
                ProviderError::ProviderNotAvailable(format!("Moondream load error: {}", e))
            })?;

            *guard = Some(captioner);
        }

        Ok(guard)
    }

    /// Get or create the BLIP captioner.
    #[cfg(feature = "candle")]
    pub(crate) fn get_blip(
        &self,
        model_path: &PathBuf,
    ) -> Result<std::sync::MutexGuard<'_, Option<BlipCaptioner>>> {
        let mut guard = self
            .blip
            .lock()
            .map_err(|e| ProviderError::Unknown(format!("Failed to lock BLIP mutex: {}", e)))?;

        if guard.is_none() {
            if !model_path.exists() {
                return Err(ProviderError::ProviderNotAvailable(format!(
                    "BLIP model not found at {:?}. Please download the model first.",
                    model_path
                )));
            }

            let device = select_device(true)
                .map_err(|e| ProviderError::ProviderNotAvailable(format!("Device error: {}", e)))?;

            let captioner = BlipCaptioner::new(model_path, device).map_err(|e| {
                ProviderError::ProviderNotAvailable(format!("BLIP load error: {}", e))
            })?;

            *guard = Some(captioner);
        }

        Ok(guard)
    }

    /// Get or create the CLIP embedder.
    #[cfg(feature = "candle")]
    pub(crate) fn get_clip(
        &self,
        model_path: &PathBuf,
    ) -> Result<std::sync::MutexGuard<'_, Option<ClipEmbedder>>> {
        let mut guard = self
            .clip
            .lock()
            .map_err(|e| ProviderError::Unknown(format!("Failed to lock CLIP mutex: {}", e)))?;

        if guard.is_none() {
            if !model_path.exists() {
                return Err(ProviderError::ProviderNotAvailable(format!(
                    "CLIP model not found at {:?}. Please download the model first.",
                    model_path
                )));
            }

            let device = select_device(true)
                .map_err(|e| ProviderError::ProviderNotAvailable(format!("Device error: {}", e)))?;

            let embedder = ClipEmbedder::new(model_path, device).map_err(|e| {
                ProviderError::ProviderNotAvailable(format!("CLIP load error: {}", e))
            })?;

            *guard = Some(embedder);
        }

        Ok(guard)
    }

    /// Extract image data from a multimodal message.
    pub(crate) fn extract_image_from_messages(messages: &[Message]) -> Option<(String, String)> {
        for msg in messages.iter().rev() {
            if let Some((data, media_type)) = msg.first_image() {
                return Some((data.to_string(), media_type.to_string()));
            }
        }
        None
    }

    /// Extract the prompt/question from messages.
    pub(crate) fn extract_prompt_from_messages(messages: &[Message]) -> String {
        for msg in messages.iter().rev() {
            if msg.role == crate::types::Role::User {
                return msg.effective_text();
            }
        }
        "Describe this image.".to_string()
    }

    /// Decode base64 image data to bytes.
    pub(crate) fn decode_image(base64_data: &str) -> Result<Vec<u8>> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(base64_data)
            .map_err(|e| {
                ProviderError::DeserializationError(format!("Invalid base64 image: {}", e))
            })
    }
}

/// Get the CLIP embedding dimension.
#[cfg(feature = "candle")]
pub fn clip_embedding_dim() -> usize {
    CLIP_EMBEDDING_DIM
}

/// Get the CLIP embedding dimension.
#[cfg(not(feature = "candle"))]
pub fn clip_embedding_dim() -> usize {
    512 // Standard CLIP ViT-B/32 dimension
}
