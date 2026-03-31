//! Processing defaults for asset handling (image captioning, embeddings).

use serde::{Deserialize, Serialize};

/// Default Moondream model for image captioning.
/// Moondream is the preferred default because it supports promptable generation
/// for different output types (alt-text vs description).
pub const DEFAULT_CAPTION_MODEL: &str = "vikhyatk/moondream2";

/// Default CLIP model for image embeddings.
pub const DEFAULT_IMAGE_EMBEDDING_MODEL: &str = "openai/clip-vit-base-patch32";

/// Default settings for asset processing (image captioning, embeddings).
///
/// These settings apply when no rule-level override is specified.
/// The priority order is: Rule setting > Tenant default > System default.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProcessingDefaults {
    /// Default caption model ID.
    /// If None, uses the system default (Salesforce/blip-image-captioning-large).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub caption_model: Option<String>,

    /// Default embedding model ID for images.
    /// If None, uses the system default (openai/clip-vit-base-patch32).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub embedding_model: Option<String>,

    /// Whether to generate image captions by default.
    /// If None, defaults to true for image assets.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generate_image_caption: Option<bool>,

    /// Whether to generate image embeddings by default.
    /// If None, defaults to true for image assets.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generate_image_embedding: Option<bool>,

    /// Whether to extract text from PDFs by default.
    /// If None, defaults to true for PDF assets.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extract_pdf_text: Option<bool>,
}

impl ProcessingDefaults {
    /// Get the effective caption model, falling back to system default.
    pub fn effective_caption_model(&self) -> &str {
        self.caption_model
            .as_deref()
            .unwrap_or(DEFAULT_CAPTION_MODEL)
    }

    /// Get the effective embedding model, falling back to system default.
    pub fn effective_embedding_model(&self) -> &str {
        self.embedding_model
            .as_deref()
            .unwrap_or(DEFAULT_IMAGE_EMBEDDING_MODEL)
    }
}
