//! Moondream image captioner using Candle.
//!
//! Moondream is a promptable vision-language model that generates natural language
//! descriptions of images based on user prompts. This enables:
//! - Custom prompt-based image analysis
//! - Separate alt-text vs description generation
//! - Question answering about images
//!
//! Supports both standard (fp32) and quantized models:
//! - Standard: `vikhyatk/moondream2` (~3.6 GB)
//! - Quantized: `santiagomed/candle-moondream` (~1.8 GB, faster CPU)
//!
//! # Architecture
//!
//! Moondream uses:
//! - **Vision Encoder**: SigLIP-based vision transformer
//! - **Language Model**: Phi-2 based decoder
//! - **Input Size**: 378x378 pixels
//!
//! # References
//!
//! - [Moondream GitHub](https://github.com/vikhyat/moondream)
//! - [HuggingFace Model](https://huggingface.co/vikhyatk/moondream2)

mod generation;
#[cfg(test)]
mod tests;
mod utils;

use std::path::Path;

use candle_core::{DType, Device, Module, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::moondream;
use candle_transformers::models::quantized_moondream;
use candle_transformers::quantized_var_builder::VarBuilder as QVarBuilder;
use tokenizers::Tokenizer;

use super::image_utils::preprocess_moondream;
use super::{CandleError, CandleResult};

pub use utils::is_moondream_model;
use utils::{
    clean_caption, find_gguf_file, find_model_file, parse_keywords, truncate_for_alt_text,
};

/// Default Moondream model - quantized version for candle compatibility.
/// Note: vikhyatk/moondream2 has incompatible tensor names with candle-transformers.
pub const DEFAULT_MOONDREAM_MODEL: &str = "santiagomed/candle-moondream";

/// Original Moondream 2 model (requires tensor name mapping).
pub const MOONDREAM2_MODEL: &str = "vikhyatk/moondream2";

/// Quantized Moondream model for faster CPU inference (same as default).
pub const QUANTIZED_MOONDREAM_MODEL: &str = "santiagomed/candle-moondream";

/// Moondream image size (378x378).
pub const MOONDREAM_IMAGE_SIZE: usize = 378;

/// Maximum caption length in tokens.
pub const MAX_CAPTION_LENGTH: usize = 128;

/// Default prompt for generating concise alt-text.
pub const ALT_TEXT_PROMPT: &str = "Describe this image briefly in one sentence for accessibility.";

/// Default prompt for generating detailed descriptions.
pub const DESCRIPTION_PROMPT: &str = "Describe this image in detail.";

/// Default prompt for extracting keywords from an image.
/// Note: Keyword quality depends on the model's ability to follow formatting instructions.
/// The quantized model may produce phrases instead of single words.
pub const KEYWORDS_PROMPT: &str =
    "List the main subjects, objects, and colors visible in this image, separated by commas.";

/// Inner model type - either standard or quantized.
enum MoondreamModelInner {
    Standard(moondream::Model),
    Quantized(quantized_moondream::Model),
}

impl std::fmt::Debug for MoondreamModelInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Standard(_) => write!(f, "Standard"),
            Self::Quantized(_) => write!(f, "Quantized"),
        }
    }
}

/// Moondream image captioner.
///
/// Generates natural language captions for images using Moondream's
/// promptable vision-language model. Unlike BLIP, Moondream can accept
/// custom prompts to guide the caption generation.
#[derive(Debug)]
pub struct MoondreamCaptioner {
    model: MoondreamModelInner,
    tokenizer: Tokenizer,
    device: Device,
    model_id: String,
    eos_token_id: u32,
}

impl MoondreamCaptioner {
    /// Create a new Moondream captioner from a model directory.
    ///
    /// Automatically detects whether to load as safetensors or quantized GGUF.
    ///
    /// # Arguments
    /// * `model_path` - Path to the model files directory
    /// * `device` - Device to use for inference
    pub fn new(model_path: &Path, device: Device) -> CandleResult<Self> {
        // Check if this is a quantized model (GGUF)
        let gguf_file = find_gguf_file(model_path);
        if let Some(gguf_path) = gguf_file {
            let tokenizer_path = model_path.join("tokenizer.json");
            return Self::new_quantized(&gguf_path, &tokenizer_path, device);
        }

        // Otherwise load as safetensors
        Self::load_from_path(model_path, device, DEFAULT_MOONDREAM_MODEL.to_string())
    }

    /// Create a Moondream captioner with a specific model ID.
    pub fn with_model_id(
        model_path: &Path,
        device: Device,
        model_id: String,
    ) -> CandleResult<Self> {
        // Check if this is a quantized model (GGUF)
        let gguf_file = find_gguf_file(model_path);
        if let Some(gguf_path) = gguf_file {
            let tokenizer_path = model_path.join("tokenizer.json");
            return Self::new_quantized_with_id(&gguf_path, &tokenizer_path, device, model_id);
        }

        Self::load_from_path(model_path, device, model_id)
    }

    /// Create a quantized Moondream captioner from GGUF model.
    ///
    /// # Arguments
    /// * `gguf_path` - Path to the quantized GGUF model file
    /// * `tokenizer_path` - Path to the tokenizer.json file
    /// * `device` - Device to use for inference
    pub fn new_quantized(
        gguf_path: &Path,
        tokenizer_path: &Path,
        device: Device,
    ) -> CandleResult<Self> {
        Self::new_quantized_with_id(
            gguf_path,
            tokenizer_path,
            device,
            QUANTIZED_MOONDREAM_MODEL.to_string(),
        )
    }

    /// Create a quantized Moondream captioner with a custom model ID.
    pub fn new_quantized_with_id(
        gguf_path: &Path,
        tokenizer_path: &Path,
        device: Device,
        model_id: String,
    ) -> CandleResult<Self> {
        // Use Moondream v2 config
        let config = moondream::Config::v2();

        // Load tokenizer
        let tokenizer = if tokenizer_path.exists() {
            Tokenizer::from_file(tokenizer_path).map_err(|e| {
                CandleError::Tokenization(format!("Failed to load tokenizer: {}", e))
            })?
        } else {
            return Err(CandleError::ModelNotDownloaded(format!(
                "Tokenizer not found at {:?}",
                tokenizer_path
            )));
        };

        // Get EOS token ID
        let eos_token_id = tokenizer.token_to_id("<|endoftext|>").unwrap_or(50256);

        // Load quantized model from GGUF using quantized_var_builder
        let vb = QVarBuilder::from_gguf(gguf_path, &device)
            .map_err(|e| CandleError::ModelLoad(format!("Failed to load GGUF: {}", e)))?;

        let model = quantized_moondream::Model::new(&config, vb).map_err(|e| {
            CandleError::ModelLoad(format!("Failed to create quantized model: {}", e))
        })?;

        tracing::info!(
            model_id = %model_id,
            gguf_path = ?gguf_path,
            device = ?device,
            "Quantized Moondream model loaded successfully"
        );

        Ok(Self {
            model: MoondreamModelInner::Quantized(model),
            tokenizer,
            device,
            model_id,
            eos_token_id,
        })
    }

    /// Load model from a directory path (safetensors format).
    fn load_from_path(path: &Path, device: Device, model_id: String) -> CandleResult<Self> {
        // Use Moondream v2 config
        let config = moondream::Config::v2();

        // Load tokenizer
        let tokenizer_path = path.join("tokenizer.json");
        let tokenizer = if tokenizer_path.exists() {
            Tokenizer::from_file(&tokenizer_path).map_err(|e| {
                CandleError::Tokenization(format!("Failed to load tokenizer: {}", e))
            })?
        } else {
            return Err(CandleError::ModelNotDownloaded(
                "Tokenizer not found. Please download the full model including tokenizer.json"
                    .to_string(),
            ));
        };

        // Get EOS token ID
        let eos_token_id = tokenizer.token_to_id("<|endoftext|>").unwrap_or(50256);

        // Determine dtype based on device
        let dtype = if device.is_cuda() {
            DType::F16
        } else {
            DType::F32
        };

        // Load model weights
        let model_file = find_model_file(path)?;

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[model_file], dtype, &device)
                .map_err(|e| CandleError::ModelLoad(format!("Failed to load weights: {}", e)))?
        };

        let model = moondream::Model::new(&config, vb)
            .map_err(|e| CandleError::ModelLoad(format!("Failed to create model: {}", e)))?;

        tracing::info!(
            model_id = %model_id,
            device = ?device,
            dtype = ?dtype,
            "Moondream model loaded successfully"
        );

        Ok(Self {
            model: MoondreamModelInner::Standard(model),
            tokenizer,
            device,
            model_id,
            eos_token_id,
        })
    }

    /// Generate a caption using a specific prompt.
    ///
    /// # Arguments
    /// * `image_bytes` - Raw image bytes (JPEG, PNG, etc.)
    /// * `prompt` - The prompt to guide caption generation
    ///
    /// # Returns
    /// The model's response to the prompt about the image.
    pub fn caption_with_prompt(
        &mut self,
        image_bytes: &[u8],
        prompt: &str,
    ) -> CandleResult<String> {
        self.caption_with_options(image_bytes, prompt, MAX_CAPTION_LENGTH)
    }

    /// Generate a caption with custom options.
    ///
    /// # Arguments
    /// * `image_bytes` - Raw image bytes
    /// * `prompt` - The prompt to guide generation
    /// * `max_length` - Maximum response length in tokens
    pub fn caption_with_options(
        &mut self,
        image_bytes: &[u8],
        prompt: &str,
        max_length: usize,
    ) -> CandleResult<String> {
        let total_start = std::time::Instant::now();

        // Preprocess image
        let preprocess_start = std::time::Instant::now();
        let image_tensor = preprocess_moondream(image_bytes, &self.device)?;
        let preprocess_time = preprocess_start.elapsed();

        // Format prompt for Moondream
        let formatted_prompt = format!("\n\nQuestion: {}\n\nAnswer:", prompt);

        // Encode prompt
        let prompt_tokens = self
            .tokenizer
            .encode(formatted_prompt.as_str(), true)
            .map_err(|e| CandleError::Tokenization(format!("Encoding failed: {}", e)))?;

        let prompt_token_ids: Vec<u32> = prompt_tokens.get_ids().to_vec();

        // Extract values before the match to avoid borrow checker issues
        let device = &self.device;
        let eos_token_id = self.eos_token_id;

        // Create BOS token tensor
        let bos_token = Tensor::new(&[eos_token_id], device)
            .map_err(|e| CandleError::Inference(format!("BOS tensor failed: {}", e)))?
            .unsqueeze(0)
            .map_err(|e| CandleError::Inference(format!("Unsqueeze failed: {}", e)))?;

        // Generate based on model type
        let (generated_tokens, decode_time, vision_time) = match &mut self.model {
            MoondreamModelInner::Standard(model) => {
                // Get image embeddings from vision encoder
                let vision_start = std::time::Instant::now();
                let image_embeds = model.vision_encoder.forward(&image_tensor).map_err(|e| {
                    CandleError::Inference(format!("Vision encoding failed: {}", e))
                })?;
                let vision_time = vision_start.elapsed();

                // Clear KV cache before generation
                model.text_model.clear_kv_cache();

                // Generate tokens
                let decode_start = std::time::Instant::now();
                let tokens = generation::generate_tokens_standard(
                    model,
                    &bos_token,
                    &prompt_token_ids,
                    &image_embeds,
                    max_length,
                    device,
                    eos_token_id,
                )?;
                let decode_time = decode_start.elapsed();

                (tokens, decode_time, vision_time)
            }
            MoondreamModelInner::Quantized(model) => {
                // Get image embeddings from vision encoder
                let vision_start = std::time::Instant::now();
                let image_embeds = model.vision_encoder.forward(&image_tensor).map_err(|e| {
                    CandleError::Inference(format!("Vision encoding failed: {}", e))
                })?;
                let vision_time = vision_start.elapsed();

                // Clear KV cache before generation
                model.text_model.clear_kv_cache();

                // Generate tokens
                let decode_start = std::time::Instant::now();
                let tokens = generation::generate_tokens_quantized(
                    model,
                    &bos_token,
                    &prompt_token_ids,
                    &image_embeds,
                    max_length,
                    device,
                    eos_token_id,
                )?;
                let decode_time = decode_start.elapsed();

                (tokens, decode_time, vision_time)
            }
        };

        // Decode generated tokens
        let output = self
            .tokenizer
            .decode(&generated_tokens, true)
            .map_err(|e| CandleError::Tokenization(format!("Decoding failed: {}", e)))?;

        let total_time = total_start.elapsed();
        tracing::debug!(
            preprocess_ms = preprocess_time.as_millis(),
            vision_ms = vision_time.as_millis(),
            decode_ms = decode_time.as_millis(),
            total_ms = total_time.as_millis(),
            generated_tokens = generated_tokens.len(),
            "Caption generation timing"
        );

        Ok(clean_caption(&output))
    }

    /// Generate a default caption for an image.
    ///
    /// Uses the DESCRIPTION_PROMPT by default.
    pub fn caption_image(&mut self, image_bytes: &[u8]) -> CandleResult<String> {
        self.caption_with_prompt(image_bytes, DESCRIPTION_PROMPT)
    }

    /// Generate concise alt-text suitable for accessibility.
    ///
    /// Uses the ALT_TEXT_PROMPT to generate a brief, one-sentence description.
    pub fn generate_alt_text(&mut self, image_bytes: &[u8]) -> CandleResult<String> {
        let caption = self.caption_with_prompt(image_bytes, ALT_TEXT_PROMPT)?;
        Ok(truncate_for_alt_text(&caption))
    }

    /// Generate detailed description of an image.
    ///
    /// Uses the DESCRIPTION_PROMPT to generate a comprehensive description.
    pub fn generate_description(&mut self, image_bytes: &[u8]) -> CandleResult<String> {
        self.caption_with_prompt(image_bytes, DESCRIPTION_PROMPT)
    }

    /// Generate keywords for an image.
    ///
    /// Uses the KEYWORDS_PROMPT to extract relevant keywords.
    /// Returns a vector of keywords parsed from the comma-separated response.
    pub fn generate_keywords(&mut self, image_bytes: &[u8]) -> CandleResult<Vec<String>> {
        let response = self.caption_with_prompt(image_bytes, KEYWORDS_PROMPT)?;
        Ok(parse_keywords(&response))
    }

    /// Generate keywords using a custom prompt.
    pub fn generate_keywords_with_prompt(
        &mut self,
        image_bytes: &[u8],
        prompt: &str,
    ) -> CandleResult<Vec<String>> {
        let response = self.caption_with_prompt(image_bytes, prompt)?;
        Ok(parse_keywords(&response))
    }

    /// Get the model ID.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }
}
