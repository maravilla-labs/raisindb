//! Types and callback definitions for asset processing

use raisin_error::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Callback for retrieving binary data from storage
pub type BinaryRetrievalCallback = Arc<
    dyn Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>>> + Send>>
        + Send
        + Sync,
>;

/// Result of asset processing
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetProcessingResult {
    /// The processed node ID
    pub node_id: String,
    /// Text extracted from PDF (if applicable)
    pub extracted_text: Option<String>,
    /// Page count for PDF (if applicable)
    pub pdf_page_count: Option<usize>,
    /// Whether OCR was used for PDF extraction
    pub used_ocr: bool,
    /// Generated image caption (if applicable)
    pub caption: Option<String>,
    /// Generated alt-text (if applicable)
    pub alt_text: Option<String>,
    /// Generated keywords (if applicable)
    pub keywords: Option<Vec<String>>,
    /// Whether image embedding was generated
    pub image_embedding_generated: bool,
    /// Dimension of the generated image embedding
    pub image_embedding_dim: Option<usize>,
    /// The generated image embedding vector (CLIP)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_embedding: Option<Vec<f32>>,
}

/// Output from PDF processing
pub(super) struct PdfProcessingOutput {
    pub text: String,
    pub page_count: usize,
    pub used_ocr: bool,
}
