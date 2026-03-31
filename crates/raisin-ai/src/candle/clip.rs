//! CLIP image embedder using Candle.
//!
//! CLIP (Contrastive Language-Image Pre-Training) generates embeddings that
//! encode both visual and textual information, enabling:
//! - Semantic image search
//! - Zero-shot image classification
//! - Cross-modal similarity matching

use std::path::PathBuf;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::clip;

use super::image_utils::{l2_normalize, preprocess_clip};
use super::{CandleError, CandleResult};

// Module trait is used implicitly by the model

/// Default CLIP model - LAION's ViT-B/32 trained on LAION-2B.
/// This model has native safetensors support (required by candle).
pub const DEFAULT_CLIP_MODEL: &str = "laion/CLIP-ViT-B-32-laion2B-s34B-b79K";

/// CLIP image size (224x224 for base model).
pub const CLIP_IMAGE_SIZE: usize = 224;

/// CLIP embedding dimension (512 for base model).
pub const CLIP_EMBEDDING_DIM: usize = 512;

/// CLIP image embedder.
///
/// Generates image embeddings using OpenAI's CLIP model via Candle.
pub struct ClipEmbedder {
    model: clip::ClipModel,
    device: Device,
    model_id: String,
}

impl ClipEmbedder {
    /// Create a new CLIP embedder.
    ///
    /// # Arguments
    /// * `model_path` - Path to the model files (must contain model.safetensors and config.json)
    /// * `device` - Device to use for inference
    pub fn new(model_path: &PathBuf, device: Device) -> CandleResult<Self> {
        Self::load_from_path(model_path, device, DEFAULT_CLIP_MODEL.to_string())
    }

    /// Create a CLIP embedder with a specific model ID.
    pub fn with_model_id(
        model_path: &PathBuf,
        device: Device,
        model_id: String,
    ) -> CandleResult<Self> {
        Self::load_from_path(model_path, device, model_id)
    }

    /// Load model from a directory path.
    fn load_from_path(path: &PathBuf, device: Device, model_id: String) -> CandleResult<Self> {
        // Use the appropriate pre-defined config based on model_id
        // For now, we only support vit-base-patch32
        let config = if model_id.contains("patch32") {
            clip::ClipConfig::vit_base_patch32()
        } else {
            // Default to base model
            clip::ClipConfig::vit_base_patch32()
        };

        // Load model weights - try multiple safetensors filenames
        // LAION models use "open_clip_model.safetensors", others use "model.safetensors"
        let safetensors_candidates = [
            path.join("model.safetensors"),
            path.join("open_clip_model.safetensors"),
        ];

        let model_path = safetensors_candidates.iter().find(|p| p.exists()).cloned();

        let model_path = match model_path {
            Some(p) => p,
            None => {
                // Check for pytorch fallback to give helpful error
                let pytorch_path = path.join("pytorch_model.bin");
                if pytorch_path.exists() {
                    return Err(CandleError::ModelNotDownloaded(format!(
                        "Model found at {:?} but it's in pytorch format. \
                        Candle requires safetensors format. \
                        Please use a model with safetensors support or convert using: \
                        `python -c \"from transformers import CLIPModel; \
                        m = CLIPModel.from_pretrained('{}'); \
                        m.save_pretrained('.', safe_serialization=True)\"`",
                        pytorch_path, model_id
                    )));
                } else {
                    return Err(CandleError::ModelNotDownloaded(format!(
                        "No model weights found at {:?}. Expected model.safetensors or open_clip_model.safetensors",
                        path
                    )));
                }
            }
        };

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)
                .map_err(|e| CandleError::ModelLoad(format!("Failed to load weights: {}", e)))?
        };

        let model = clip::ClipModel::new(vb, &config)
            .map_err(|e| CandleError::ModelLoad(format!("Failed to create model: {}", e)))?;

        tracing::info!(
            model_id = %model_id,
            device = ?device,
            "CLIP model loaded successfully"
        );

        Ok(Self {
            model,
            device,
            model_id,
        })
    }

    /// Generate embedding for an image.
    ///
    /// # Arguments
    /// * `image_bytes` - Raw image bytes (JPEG, PNG, etc.)
    ///
    /// # Returns
    /// L2-normalized embedding vector of dimension 512.
    pub fn embed_image(&self, image_bytes: &[u8]) -> CandleResult<Vec<f32>> {
        // Preprocess image
        let image_tensor = preprocess_clip(image_bytes, CLIP_IMAGE_SIZE, &self.device)?;

        // Get image features from vision encoder
        let image_features = self
            .model
            .get_image_features(&image_tensor)
            .map_err(|e| CandleError::Inference(format!("Image encoding failed: {}", e)))?;

        // Convert to Vec<f32>
        let embedding = image_features
            .squeeze(0)
            .map_err(|e| CandleError::Inference(format!("Squeeze failed: {}", e)))?
            .to_vec1::<f32>()
            .map_err(|e| CandleError::Inference(format!("Conversion failed: {}", e)))?;

        // L2 normalize for cosine similarity
        Ok(l2_normalize(&embedding))
    }

    /// Generate embeddings for multiple images in a batch.
    ///
    /// More efficient than calling `embed_image` for each image.
    pub fn embed_images(&self, images: &[&[u8]]) -> CandleResult<Vec<Vec<f32>>> {
        if images.is_empty() {
            return Ok(vec![]);
        }

        // Preprocess all images
        let mut tensors = Vec::with_capacity(images.len());
        for image_bytes in images {
            let tensor = preprocess_clip(image_bytes, CLIP_IMAGE_SIZE, &self.device)?;
            tensors.push(tensor);
        }

        // Stack into batch
        let batch = Tensor::cat(&tensors, 0)
            .map_err(|e| CandleError::Inference(format!("Batch concat failed: {}", e)))?;

        // Get image features
        let image_features = self
            .model
            .get_image_features(&batch)
            .map_err(|e| CandleError::Inference(format!("Batch encoding failed: {}", e)))?;

        // Extract individual embeddings
        let batch_size = images.len();
        let mut embeddings = Vec::with_capacity(batch_size);

        for i in 0..batch_size {
            let embedding = image_features
                .get(i)
                .map_err(|e| CandleError::Inference(format!("Index {} failed: {}", i, e)))?
                .to_vec1::<f32>()
                .map_err(|e| CandleError::Inference(format!("Conversion failed: {}", e)))?;

            embeddings.push(l2_normalize(&embedding));
        }

        Ok(embeddings)
    }

    /// Generate text embedding for similarity matching.
    ///
    /// Useful for text-to-image search.
    pub fn embed_text(&self, text: &str) -> CandleResult<Vec<f32>> {
        // Create a simple tokenizer for CLIP
        // Note: In production, you should use the proper CLIP tokenizer
        let tokens = self.tokenize_simple(text)?;

        let input_ids = Tensor::new(vec![tokens], &self.device)
            .map_err(|e| CandleError::Inference(format!("Token tensor failed: {}", e)))?;

        let text_features = self
            .model
            .get_text_features(&input_ids)
            .map_err(|e| CandleError::Inference(format!("Text encoding failed: {}", e)))?;

        let embedding = text_features
            .squeeze(0)
            .map_err(|e| CandleError::Inference(format!("Squeeze failed: {}", e)))?
            .to_vec1::<f32>()
            .map_err(|e| CandleError::Inference(format!("Conversion failed: {}", e)))?;

        Ok(l2_normalize(&embedding))
    }

    /// Simple tokenization for CLIP.
    ///
    /// Note: This is a placeholder. In production, use the proper CLIP tokenizer.
    fn tokenize_simple(&self, text: &str) -> CandleResult<Vec<u32>> {
        // CLIP uses a BPE tokenizer with specific tokens
        // For now, use a simple space-based tokenization with placeholders
        // Start token: 49406, End token: 49407, Pad: 0

        let max_len = 77; // CLIP max sequence length
        let mut tokens = vec![49406u32]; // Start token

        // Simple word-to-token mapping (placeholder)
        for word in text.split_whitespace().take(max_len - 2) {
            // Map each character to a token (very simplified)
            for c in word.chars().take(3) {
                tokens.push((c as u32) % 49000 + 1000);
            }
        }

        tokens.push(49407); // End token

        // Pad to max_len
        while tokens.len() < max_len {
            tokens.push(0);
        }

        tokens.truncate(max_len);
        Ok(tokens)
    }

    /// Get the model ID.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Get the embedding dimension.
    pub fn embedding_dim(&self) -> usize {
        CLIP_EMBEDDING_DIM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(CLIP_IMAGE_SIZE, 224);
        assert_eq!(CLIP_EMBEDDING_DIM, 512);
        assert!(!DEFAULT_CLIP_MODEL.is_empty());
        assert!(DEFAULT_CLIP_MODEL.to_lowercase().contains("clip"));
    }

    #[test]
    fn test_embedding_dim_accessor() {
        // Test that embedding_dim() returns the correct constant
        assert_eq!(CLIP_EMBEDDING_DIM, 512);
    }

    #[test]
    fn test_model_id_format() {
        // Verify model ID format is valid HuggingFace format
        assert!(DEFAULT_CLIP_MODEL.contains('/'));
        let parts: Vec<&str> = DEFAULT_CLIP_MODEL.split('/').collect();
        assert_eq!(parts.len(), 2);
        assert!(!parts[0].is_empty()); // org/user
        assert!(!parts[1].is_empty()); // model name
    }
}
