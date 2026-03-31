//! BLIP image captioner using Candle.
//!
//! BLIP (Bootstrapping Language-Image Pre-training) generates natural language
//! descriptions of images, enabling:
//! - Automatic image captioning for accessibility
//! - SEO-friendly alt text generation
//! - Searchable content indexing
//!
//! Supports both standard (fp32) and quantized (Q4K) models:
//! - Standard: `Salesforce/blip-image-captioning-large` (1.88 GB)
//! - Quantized: `lmz/candle-blip` (271 MB, ~7x faster on CPU)

use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Tensor};
use candle_nn::{Module, VarBuilder};
use candle_transformers::models::{blip, quantized_blip};
use tokenizers::Tokenizer;

use super::image_utils::preprocess_blip;
use super::{CandleError, CandleResult};

/// Default BLIP model - Salesforce's large captioning model.
/// This model has native safetensors support (required by candle).
pub const DEFAULT_BLIP_MODEL: &str = "Salesforce/blip-image-captioning-large";

/// Quantized BLIP model for fast CPU inference.
pub const QUANTIZED_BLIP_MODEL: &str = "lmz/candle-blip";

/// BLIP image size (384x384 for base model).
pub const BLIP_IMAGE_SIZE: usize = 384;

/// Maximum caption length in tokens.
pub const MAX_CAPTION_LENGTH: usize = 50;

/// Wrapper enum for standard and quantized BLIP models.
enum BlipModel {
    Standard(blip::BlipForConditionalGeneration),
    Quantized(quantized_blip::BlipForConditionalGeneration),
}

impl BlipModel {
    fn vision_forward(&self, xs: &Tensor) -> candle_core::Result<Tensor> {
        match self {
            BlipModel::Standard(m) => m.vision_model().forward(xs),
            BlipModel::Quantized(m) => m.vision_model().forward(xs),
        }
    }

    fn text_decoder_forward(
        &mut self,
        xs: &Tensor,
        img_xs: &Tensor,
    ) -> candle_core::Result<Tensor> {
        match self {
            BlipModel::Standard(m) => m.text_decoder().forward(xs, img_xs),
            BlipModel::Quantized(m) => m.text_decoder().forward(xs, img_xs),
        }
    }

    fn reset_kv_cache(&mut self) {
        match self {
            BlipModel::Standard(m) => m.reset_kv_cache(),
            BlipModel::Quantized(m) => m.reset_kv_cache(),
        }
    }
}

/// BLIP image captioner.
///
/// Generates natural language captions for images using Salesforce's BLIP model.
/// Supports both standard (fp32) and quantized (Q4K) variants.
pub struct BlipCaptioner {
    model: BlipModel,
    tokenizer: Tokenizer,
    device: Device,
    model_id: String,
}

impl BlipCaptioner {
    /// Create a new BLIP captioner with standard fp32 model.
    ///
    /// # Arguments
    /// * `model_path` - Path to the model files (directory with model.safetensors)
    /// * `device` - Device to use for inference
    pub fn new(model_path: &PathBuf, device: Device) -> CandleResult<Self> {
        Self::load_from_path(model_path, device, DEFAULT_BLIP_MODEL.to_string())
    }

    /// Create a BLIP captioner with a specific model ID.
    pub fn with_model_id(
        model_path: &PathBuf,
        device: Device,
        model_id: String,
    ) -> CandleResult<Self> {
        Self::load_from_path(model_path, device, model_id)
    }

    /// Create a BLIP captioner with quantized Q4K model for faster CPU inference.
    ///
    /// # Arguments
    /// * `gguf_path` - Path to the GGUF model file (e.g., blip-image-captioning-large-q4k.gguf)
    /// * `tokenizer_path` - Path to tokenizer.json
    /// * `device` - Device to use for inference
    ///
    /// # Performance
    /// Quantized models are ~7x faster on CPU and use ~7x less memory.
    pub fn new_quantized(
        gguf_path: &Path,
        tokenizer_path: &Path,
        device: Device,
    ) -> CandleResult<Self> {
        let config = blip::Config::image_captioning_large();

        // Load tokenizer
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| CandleError::Tokenization(format!("Failed to load tokenizer: {}", e)))?;

        // Load quantized model from GGUF
        let vb = quantized_blip::VarBuilder::from_gguf(gguf_path, &device)
            .map_err(|e| CandleError::ModelLoad(format!("Failed to load GGUF weights: {}", e)))?;

        let model =
            quantized_blip::BlipForConditionalGeneration::new(&config, vb).map_err(|e| {
                CandleError::ModelLoad(format!("Failed to create quantized model: {}", e))
            })?;

        tracing::info!(
            model_id = %QUANTIZED_BLIP_MODEL,
            device = ?device,
            "Quantized BLIP model loaded successfully"
        );

        Ok(Self {
            model: BlipModel::Quantized(model),
            tokenizer,
            device,
            model_id: QUANTIZED_BLIP_MODEL.to_string(),
        })
    }

    /// Load standard model from a directory path.
    fn load_from_path(path: &PathBuf, device: Device, model_id: String) -> CandleResult<Self> {
        // Use the appropriate pre-defined config based on model_id
        let config = if model_id.contains("large") {
            blip::Config::image_captioning_large()
        } else {
            // Default to base model
            // Note: candle-transformers may not have a base() method, use large as fallback
            blip::Config::image_captioning_large()
        };

        // Load tokenizer
        let tokenizer_path = path.join("tokenizer.json");
        let tokenizer = if tokenizer_path.exists() {
            Tokenizer::from_file(&tokenizer_path).map_err(|e| {
                CandleError::Tokenization(format!("Failed to load tokenizer: {}", e))
            })?
        } else {
            // Try to use a default BERT tokenizer
            return Err(CandleError::ModelNotDownloaded(
                "Tokenizer not found. Please download the full model including tokenizer.json"
                    .to_string(),
            ));
        };

        // Load model weights - prefer safetensors, but check for pytorch as fallback
        let safetensors_path = path.join("model.safetensors");
        let pytorch_path = path.join("pytorch_model.bin");

        let model_path_file = if safetensors_path.exists() {
            safetensors_path
        } else if pytorch_path.exists() {
            // pytorch_model.bin exists but we can't load it without pickle support
            return Err(CandleError::ModelNotDownloaded(format!(
                "Model found at {:?} but it's in pytorch format. \
                Candle requires safetensors format. \
                Please convert the model using: \
                `python -c \"from transformers import BlipForConditionalGeneration; \
                m = BlipForConditionalGeneration.from_pretrained('{}'); \
                m.save_pretrained('.', safe_serialization=True)\"`",
                pytorch_path, model_id
            )));
        } else {
            return Err(CandleError::ModelNotDownloaded(format!(
                "Model not found. Expected model.safetensors or pytorch_model.bin at {:?}",
                path
            )));
        };

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[model_path_file], DType::F32, &device)
                .map_err(|e| CandleError::ModelLoad(format!("Failed to load weights: {}", e)))?
        };

        let model = blip::BlipForConditionalGeneration::new(&config, vb)
            .map_err(|e| CandleError::ModelLoad(format!("Failed to create model: {}", e)))?;

        tracing::info!(
            model_id = %model_id,
            device = ?device,
            "BLIP model loaded successfully"
        );

        Ok(Self {
            model: BlipModel::Standard(model),
            tokenizer,
            device,
            model_id,
        })
    }

    /// Generate a caption for an image.
    ///
    /// # Arguments
    /// * `image_bytes` - Raw image bytes (JPEG, PNG, etc.)
    ///
    /// # Returns
    /// A natural language description of the image.
    pub fn caption_image(&mut self, image_bytes: &[u8]) -> CandleResult<String> {
        self.caption_image_with_options(image_bytes, MAX_CAPTION_LENGTH, None)
    }

    /// Generate a caption with custom options.
    ///
    /// # Arguments
    /// * `image_bytes` - Raw image bytes
    /// * `max_length` - Maximum caption length in tokens
    /// * `prompt` - Optional prompt to guide captioning (e.g., "A photo of")
    pub fn caption_image_with_options(
        &mut self,
        image_bytes: &[u8],
        max_length: usize,
        prompt: Option<&str>,
    ) -> CandleResult<String> {
        let total_start = std::time::Instant::now();

        // Preprocess image (already includes batch dimension from preprocess_blip)
        let preprocess_start = std::time::Instant::now();
        let image_tensor = preprocess_blip(image_bytes, BLIP_IMAGE_SIZE, &self.device)?;
        let preprocess_time = preprocess_start.elapsed();

        // Get image embeddings from vision encoder
        let vision_start = std::time::Instant::now();
        let image_embeds = self
            .model
            .vision_forward(&image_tensor)
            .map_err(|e| CandleError::Inference(format!("Vision encoding failed: {}", e)))?;
        let vision_time = vision_start.elapsed();

        // Reset KV cache before generation
        self.model.reset_kv_cache();

        // Initialize decoder input
        let bos_token_id = 30522u32; // [DEC] token for BLIP
        let eos_token_id = 102u32; // [SEP] token

        // Start with BOS token or prompt
        let mut token_ids: Vec<u32> = if let Some(prompt_text) = prompt {
            let encoding = self
                .tokenizer
                .encode(prompt_text, true)
                .map_err(|e| CandleError::Tokenization(format!("Encoding failed: {}", e)))?;
            encoding.get_ids().to_vec()
        } else {
            vec![bos_token_id]
        };

        // Generate tokens autoregressively with incremental decoding
        // Following the official candle BLIP example pattern
        let decode_start = std::time::Instant::now();
        for index in 0..max_length {
            // After first iteration, only pass the last token (KV cache handles history)
            let context_size = if index > 0 { 1 } else { token_ids.len() };
            let start_pos = token_ids.len().saturating_sub(context_size);

            let input_ids = Tensor::new(&token_ids[start_pos..], &self.device)
                .map_err(|e| CandleError::Inference(format!("Input tensor failed: {}", e)))?
                .unsqueeze(0)
                .map_err(|e| CandleError::Inference(format!("Unsqueeze failed: {}", e)))?;

            // Get next token logits
            let logits = self
                .model
                .text_decoder_forward(&input_ids, &image_embeds)
                .map_err(|e| CandleError::Inference(format!("Decoder forward failed: {}", e)))?;

            // Get logits for last position
            let seq_len = logits
                .dim(1)
                .map_err(|e| CandleError::Inference(format!("Dim failed: {}", e)))?;
            let next_token_logits = logits
                .squeeze(0)
                .map_err(|e| CandleError::Inference(format!("Squeeze failed: {}", e)))?
                .get(seq_len - 1)
                .map_err(|e| CandleError::Inference(format!("Get last failed: {}", e)))?;

            // Greedy decoding: take argmax
            let next_token = next_token_logits
                .argmax(0)
                .map_err(|e| CandleError::Inference(format!("Argmax failed: {}", e)))?
                .to_scalar::<u32>()
                .map_err(|e| CandleError::Inference(format!("Scalar conversion failed: {}", e)))?;

            // Check for EOS
            if next_token == eos_token_id {
                break;
            }

            token_ids.push(next_token);
        }
        let decode_time = decode_start.elapsed();
        let tokens_generated = token_ids.len();

        // Decode tokens to text
        let caption = self
            .tokenizer
            .decode(&token_ids, true)
            .map_err(|e| CandleError::Tokenization(format!("Decoding failed: {}", e)))?;

        let total_time = total_start.elapsed();
        let ms_per_token = if tokens_generated > 0 {
            decode_time.as_millis() as f64 / tokens_generated as f64
        } else {
            0.0
        };

        tracing::info!(
            preprocess_ms = %preprocess_time.as_millis(),
            vision_ms = %vision_time.as_millis(),
            decode_ms = %decode_time.as_millis(),
            total_ms = %total_time.as_millis(),
            tokens = %tokens_generated,
            ms_per_token = %format!("{:.1}", ms_per_token),
            model = %self.model_id,
            "BLIP caption generated"
        );

        Ok(clean_caption(&caption))
    }

    /// Generate concise alt-text suitable for accessibility.
    ///
    /// Produces shorter, more descriptive captions optimized for screen readers.
    pub fn generate_alt_text(&mut self, image_bytes: &[u8]) -> CandleResult<String> {
        let caption = self.caption_image(image_bytes)?;

        // Post-process for alt-text
        Ok(generate_alt_text_from_caption(&caption))
    }

    /// Get the model ID.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// Clean up generated caption text.
fn clean_caption(caption: &str) -> String {
    let cleaned = caption
        .trim()
        // Remove common artifacts
        .trim_start_matches("[CLS]")
        .trim_start_matches("[DEC]")
        .trim_end_matches("[SEP]")
        .trim_end_matches("[PAD]")
        .trim();

    // Capitalize first letter
    let mut chars = cleaned.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Generate concise alt-text from a verbose caption.
///
/// BLIP captions like "a photograph of a white dog playing with a red ball"
/// become "White dog playing with red ball".
pub fn generate_alt_text_from_caption(caption: &str) -> String {
    let result = caption
        .to_lowercase()
        // Remove common filler phrases
        .trim_start_matches("a photograph of ")
        .trim_start_matches("an image of ")
        .trim_start_matches("a photo of ")
        .trim_start_matches("a picture of ")
        .trim_start_matches("a close up of ")
        .trim_start_matches("a ")
        .trim_start_matches("an ")
        .trim()
        .to_string();

    // Capitalize first letter
    let mut chars = result.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let capitalized = first.to_uppercase().collect::<String>() + chars.as_str();
            // Truncate to ~125 chars for WCAG recommendation
            if capitalized.len() > 125 {
                let truncated: String = capitalized.chars().take(122).collect();
                format!("{}...", truncated.trim_end())
            } else {
                capitalized
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_caption() {
        assert_eq!(clean_caption("  a dog  "), "A dog");
        assert_eq!(clean_caption("[CLS] a cat"), "A cat");
        assert_eq!(clean_caption("bird [SEP]"), "Bird");
    }

    #[test]
    fn test_clean_caption_multiple_tokens() {
        assert_eq!(clean_caption("[CLS] hello [SEP]"), "Hello");
        assert_eq!(clean_caption("[DEC] test [PAD]"), "Test");
    }

    #[test]
    fn test_clean_caption_empty() {
        assert_eq!(clean_caption(""), "");
        assert_eq!(clean_caption("   "), "");
    }

    #[test]
    fn test_clean_caption_preserves_internal_case() {
        // Only first letter should be capitalized
        assert_eq!(clean_caption("iPhone display"), "IPhone display");
    }

    #[test]
    fn test_generate_alt_text() {
        assert_eq!(
            generate_alt_text_from_caption("a photograph of a white dog"),
            "White dog"
        );
        assert_eq!(
            generate_alt_text_from_caption("an image of a beautiful sunset"),
            "Beautiful sunset"
        );
        assert_eq!(
            generate_alt_text_from_caption("A cat sleeping"),
            "Cat sleeping"
        );
    }

    #[test]
    fn test_alt_text_removes_various_prefixes() {
        assert_eq!(
            generate_alt_text_from_caption("a photo of mountains"),
            "Mountains"
        );
        assert_eq!(generate_alt_text_from_caption("a picture of a car"), "Car");
        assert_eq!(
            generate_alt_text_from_caption("a close up of a flower"),
            "Flower"
        );
        assert_eq!(generate_alt_text_from_caption("an elephant"), "Elephant");
    }

    #[test]
    fn test_alt_text_handles_edge_cases() {
        // Already clean text
        assert_eq!(generate_alt_text_from_caption("Dog"), "Dog");

        // Empty input
        assert_eq!(generate_alt_text_from_caption(""), "");

        // Just whitespace
        assert_eq!(generate_alt_text_from_caption("   "), "");

        // Just prefix
        assert_eq!(generate_alt_text_from_caption("a photograph of "), "");
    }

    #[test]
    fn test_alt_text_truncation() {
        let long_caption = "a".repeat(200);
        let alt = generate_alt_text_from_caption(&long_caption);
        assert!(alt.len() <= 128); // 125 + "..."
        assert!(alt.ends_with("..."));
    }

    #[test]
    fn test_alt_text_exactly_at_limit() {
        // Create a caption that's exactly 125 chars after processing
        let caption = "x".repeat(125);
        let alt = generate_alt_text_from_caption(&caption);
        assert_eq!(alt.len(), 125);
        assert!(!alt.ends_with("..."));
    }

    #[test]
    fn test_alt_text_unicode() {
        // Ensure Unicode is handled correctly
        assert_eq!(
            generate_alt_text_from_caption("a photo of 日本語"),
            "日本語"
        );
    }

    #[test]
    fn test_constants() {
        assert_eq!(BLIP_IMAGE_SIZE, 384);
        assert_eq!(MAX_CAPTION_LENGTH, 50);
        assert!(!DEFAULT_BLIP_MODEL.is_empty());
    }
}
