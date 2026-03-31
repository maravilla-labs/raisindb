//! Processing settings for content handling rules.

use serde::{Deserialize, Serialize};

use crate::config::ChunkingConfig;
use crate::pdf::PdfStrategy;

/// Settings that control how content is processed.
///
/// All fields are optional - if None, the default or tenant-level setting is used.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessingSettings {
    /// Chunking configuration for text embedding.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "deserialize_chunking_compat"
    )]
    pub chunking: Option<ChunkingConfig>,

    /// PDF extraction strategy.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pdf_strategy: Option<PdfStrategy>,
    /// Whether to generate image embeddings (CLIP).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generate_image_embedding: Option<bool>,

    /// Whether to generate image captions (BLIP).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generate_image_caption: Option<bool>,

    /// Model to use for image captioning.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub caption_model: Option<String>,

    /// Custom prompt for alt-text generation (Moondream only).
    /// If not set, uses the default: "Describe this image briefly in one sentence for accessibility."
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub alt_text_prompt: Option<String>,

    /// Custom prompt for description generation (Moondream only).
    /// If not set, uses the default: "Describe this image in detail."
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description_prompt: Option<String>,

    /// Whether to generate image keywords (Moondream only).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generate_keywords: Option<bool>,

    /// Custom prompt for keyword extraction (Moondream only).
    /// If not set, uses the default: "List 5-10 keywords that describe this image, separated by commas."
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub keywords_prompt: Option<String>,

    /// Model to use for embeddings.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub embedding_model: Option<String>,

    /// Whether to trigger embedding generation after text extraction.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trigger_embedding: Option<bool>,

    /// Whether to store extracted text in node properties.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub store_extracted_text: Option<bool>,

    /// Maximum text length to store (for extracted text).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_stored_text_length: Option<usize>,
}

/// Helper enum for backward compatibility with boolean chunking config
#[derive(Deserialize)]
#[serde(untagged)]
enum ChunkingConfigCompat {
    Bool(bool),
    Config(ChunkingConfig),
}

fn deserialize_chunking_compat<'de, D>(deserializer: D) -> Result<Option<ChunkingConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let helper: Option<ChunkingConfigCompat> = Option::deserialize(deserializer)?;
    match helper {
        Some(ChunkingConfigCompat::Bool(true)) => Ok(Some(ChunkingConfig::default())),
        Some(ChunkingConfigCompat::Bool(false)) => Ok(None),
        Some(ChunkingConfigCompat::Config(c)) => Ok(Some(c)),
        None => Ok(None),
    }
}

impl ProcessingSettings {
    /// Create settings for PDF processing.
    pub fn pdf() -> Self {
        Self {
            pdf_strategy: Some(PdfStrategy::Auto),
            trigger_embedding: Some(true),
            store_extracted_text: Some(true),
            ..Default::default()
        }
    }

    /// Create settings for image processing.
    pub fn image() -> Self {
        Self {
            generate_image_embedding: Some(true),
            generate_image_caption: Some(true),
            ..Default::default()
        }
    }

    /// Merge with another settings, preferring values from `other`.
    pub fn merge(&self, other: &ProcessingSettings) -> ProcessingSettings {
        ProcessingSettings {
            chunking: other.chunking.clone().or_else(|| self.chunking.clone()),
            pdf_strategy: other.pdf_strategy.or(self.pdf_strategy),
            generate_image_embedding: other
                .generate_image_embedding
                .or(self.generate_image_embedding),
            generate_image_caption: other.generate_image_caption.or(self.generate_image_caption),
            caption_model: other
                .caption_model
                .clone()
                .or_else(|| self.caption_model.clone()),
            alt_text_prompt: other
                .alt_text_prompt
                .clone()
                .or_else(|| self.alt_text_prompt.clone()),
            description_prompt: other
                .description_prompt
                .clone()
                .or_else(|| self.description_prompt.clone()),
            generate_keywords: other.generate_keywords.or(self.generate_keywords),
            keywords_prompt: other
                .keywords_prompt
                .clone()
                .or_else(|| self.keywords_prompt.clone()),
            embedding_model: other
                .embedding_model
                .clone()
                .or_else(|| self.embedding_model.clone()),
            trigger_embedding: other.trigger_embedding.or(self.trigger_embedding),
            store_extracted_text: other.store_extracted_text.or(self.store_extracted_text),
            max_stored_text_length: other.max_stored_text_length.or(self.max_stored_text_length),
        }
    }
}
