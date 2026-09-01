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
    /// Whether the extracted text was actually written back onto the node.
    ///
    /// The distinction matters: `extracted_text.is_some()` only says the
    /// extractor produced words, and for a long time that was ALL that
    /// happened — the words went into this JSON and nowhere else. Only a `true`
    /// here means the text reached a node property, and therefore the fulltext
    /// and embedding indexes.
    #[serde(default)]
    pub extracted_text_stored: bool,
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

/// Output from a native text extraction — a PDF, or OCR over an image.
///
/// `source` is decided HERE, by the branch that did the work, rather than
/// inferred at the call site from `used_ocr`. The two are not the same
/// question: an image is always OCR, a PDF is OCR only sometimes, and a call
/// site reconstructing the label from a boolean got `core-pdf-ocr` for a PNG.
/// One producer, one label.
pub(super) struct TextExtractionOutput {
    pub text: String,
    pub page_count: usize,
    pub used_ocr: bool,
    /// Recorded verbatim as `__extract_source`.
    pub source: &'static str,
}
