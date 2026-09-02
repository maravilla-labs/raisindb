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
    select_device, BlipCaptioner, ClipEmbedder, MoondreamCaptioner, QwenGenerator,
    CLIP_EMBEDDING_DIM,
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

/* THE TEXT MODEL IS CACHED PER PROCESS, NOT PER PROVIDER.
 *
 * `LocalCandleProvider` is constructed FRESH on every completion — the `local:`
 * route builds one per call because it needs no tenant config to look up — so a
 * cache held on `self` is empty every time and 1.1 GB of weights is read from
 * disk for each request. Measured: that alone exceeded the client's 30s request
 * timeout, i.e. the model was unusable rather than merely slow.
 *
 * A `static` keeps the weights resident for the life of the server, which is
 * what makes the second prompt fast. One instance behind a mutex is also the
 * honest shape: generating is `&mut self` because the model owns a KV cache, so
 * requests serialize — a second concurrent generation would need a second copy
 * of the weights in memory, which is exactly what a 1.1 GB model cannot afford.
 *
 * The vision models still cache per instance. Same latent problem, but they are
 * reached through paths that reuse a provider, and changing them is not this
 * change. */
#[cfg(feature = "candle")]
static QWEN: std::sync::OnceLock<Mutex<Option<QwenGenerator>>> = std::sync::OnceLock::new();

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

    /// Get or create the Qwen text generator.
    ///
    /// The model path handed in is whatever the registry returned, and that is
    /// NOT one consistent shape: a fresh download returns the HuggingFace
    /// snapshot directory, while an already-present model returns the local
    /// model directory. So the two files are SEARCHED for rather than assumed —
    /// which also means a model dropped in by hand works without matching
    /// hf-hub's layout.
    #[cfg(feature = "candle")]
    pub(crate) fn get_qwen(
        &self,
        model_path: &PathBuf,
    ) -> Result<std::sync::MutexGuard<'static, Option<QwenGenerator>>> {
        let mut guard = QWEN
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|e| ProviderError::Unknown(format!("Failed to lock Qwen mutex: {}", e)))?;

        if guard.is_none() {
            let gguf = find_file(model_path, |name| name.ends_with(".gguf")).ok_or_else(|| {
                ProviderError::ProviderNotAvailable(format!(
                    "No .gguf file under {:?}. The model is not downloaded.",
                    model_path
                ))
            })?;
            let tokenizer =
                find_file(model_path, |name| name == "tokenizer.json").ok_or_else(|| {
                    ProviderError::ProviderNotAvailable(format!(
                        "No tokenizer.json under {:?}. The GGUF alone cannot tokenize.",
                        model_path
                    ))
                })?;

            let device = select_device(true)
                .map_err(|e| ProviderError::ProviderNotAvailable(format!("Device error: {}", e)))?;

            let generator = QwenGenerator::new(&gguf, &tokenizer, device).map_err(|e| {
                ProviderError::ProviderNotAvailable(format!("Qwen load error: {}", e))
            })?;

            *guard = Some(generator);
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

/// Find one file under `root`, searching a few levels down.
///
/// Shallow on purpose (3 levels): hf-hub nests a repo as
/// `models--org--name/snapshots/<sha>/`, which is exactly three, and a deeper
/// walk on a large models directory would cost more than it can ever find.
#[cfg(feature = "candle")]
fn find_file(root: &std::path::Path, matches: impl Fn(&str) -> bool + Copy) -> Option<PathBuf> {
    fn walk(
        dir: &std::path::Path,
        depth: usize,
        matches: &dyn Fn(&str) -> bool,
    ) -> Option<PathBuf> {
        if depth == 0 {
            return None;
        }
        let entries = std::fs::read_dir(dir).ok()?;
        let mut dirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches(name) {
                    return Some(path);
                }
            }
        }
        // Files before directories, so a match beside the root wins over a
        // deeper one — the snapshot a symlink points at, not a stale sibling.
        for d in dirs {
            if let Some(found) = walk(&d, depth - 1, matches) {
                return Some(found);
            }
        }
        None
    }
    walk(root, 4, &matches)
}
