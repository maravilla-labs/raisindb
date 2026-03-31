//! Candle-based local AI inference for images.
//!
//! This module provides local AI inference capabilities using the Candle
//! framework from Hugging Face. It supports:
//!
//! - **CLIP**: Image embeddings for semantic search
//! - **Moondream**: Promptable image captioning (default)
//! - **BLIP**: Fast image captioning (fallback)
//!
//! # Feature Flag
//!
//! This module requires the `candle` feature to be enabled.
//!
//! # Usage
//!
//! ```rust,ignore
//! use raisin_ai::candle::{ClipEmbedder, MoondreamCaptioner, Device};
//!
//! // Create embedder with default model
//! let embedder = ClipEmbedder::new(Device::Cpu)?;
//!
//! // Generate embedding for image
//! let image_bytes = std::fs::read("image.jpg")?;
//! let embedding = embedder.embed_image(&image_bytes)?;
//!
//! // Create promptable captioner (Moondream)
//! let captioner = MoondreamCaptioner::new(model_path, Device::Cpu)?;
//! let alt_text = captioner.generate_alt_text(&image_bytes)?;
//! let description = captioner.generate_description(&image_bytes)?;
//! ```

#[cfg(feature = "candle")]
pub mod clip;

#[cfg(feature = "candle")]
pub mod blip;

#[cfg(feature = "candle")]
pub mod moondream;

#[cfg(feature = "candle")]
mod device;

#[cfg(feature = "candle")]
mod image_utils;

use thiserror::Error;

/// Errors that can occur during Candle model inference.
#[derive(Debug, Error)]
pub enum CandleError {
    /// Failed to load the model.
    #[error("Model loading failed: {0}")]
    ModelLoad(String),

    /// Model is not downloaded.
    #[error("Model not downloaded: {0}")]
    ModelNotDownloaded(String),

    /// Failed to process the image.
    #[error("Image processing failed: {0}")]
    ImageProcessing(String),

    /// Inference failed.
    #[error("Inference failed: {0}")]
    Inference(String),

    /// Device not available.
    #[error("Device not available: {0}")]
    DeviceUnavailable(String),

    /// Tokenization failed.
    #[error("Tokenization failed: {0}")]
    Tokenization(String),

    /// Model not yet supported.
    #[error("Model not supported: {0}")]
    ModelNotSupported(String),
}

/// Result type for Candle operations.
pub type CandleResult<T> = Result<T, CandleError>;

#[cfg(feature = "candle")]
pub use clip::{ClipEmbedder, CLIP_EMBEDDING_DIM};

#[cfg(feature = "candle")]
pub use blip::{BlipCaptioner, DEFAULT_BLIP_MODEL, QUANTIZED_BLIP_MODEL};

#[cfg(feature = "candle")]
pub use moondream::{
    is_moondream_model, MoondreamCaptioner, ALT_TEXT_PROMPT, DEFAULT_MOONDREAM_MODEL,
    DESCRIPTION_PROMPT, KEYWORDS_PROMPT, MOONDREAM2_MODEL, MOONDREAM_IMAGE_SIZE,
    QUANTIZED_MOONDREAM_MODEL,
};

#[cfg(feature = "candle")]
pub use device::select_device;

// ============================================================================
// Caption Model Registry
// ============================================================================

/// Information about an available captioning model.
#[derive(Debug, Clone)]
pub struct CaptionModelInfo {
    /// Model ID (e.g., "Salesforce/blip-image-captioning-large")
    pub id: &'static str,
    /// Human-readable name
    pub name: &'static str,
    /// Approximate model size in MB
    pub size_mb: u32,
    /// Whether this model is currently supported
    pub supported: bool,
    /// Brief description
    pub description: &'static str,
}

/// Available captioning models.
///
/// Moondream is the default because it supports prompting for different
/// output types (alt-text vs description). BLIP is available as a fallback.
pub const AVAILABLE_CAPTION_MODELS: &[CaptionModelInfo] = &[
    CaptionModelInfo {
        id: "vikhyatk/moondream2",
        name: "Moondream 2",
        size_mb: 3600,
        supported: true,
        description: "Default - Promptable vision-language model for detailed captions",
    },
    CaptionModelInfo {
        id: "santiagomed/candle-moondream",
        name: "Moondream (Quantized)",
        size_mb: 1800,
        supported: true,
        description: "Faster CPU inference, smaller model",
    },
    CaptionModelInfo {
        id: "Salesforce/blip-image-captioning-large",
        name: "BLIP Large",
        size_mb: 1880,
        supported: true,
        description: "Fallback - Fast single-caption model",
    },
    CaptionModelInfo {
        id: "lmz/candle-blip",
        name: "BLIP Large (Quantized)",
        size_mb: 271,
        supported: true,
        description: "Fallback - Fastest CPU inference",
    },
];

/// Check if a model ID is a BLIP-family model.
pub fn is_blip_model(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();
    lower.contains("blip") || lower.contains("salesforce")
}

/// Get the default captioning model ID.
///
/// Returns Moondream as the default because it supports promptable
/// generation for different output types (alt-text vs description).
pub fn default_caption_model() -> &'static str {
    DEFAULT_MOONDREAM_MODEL
}
