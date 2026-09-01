//! Helper functions for extracting node properties and processing assets.
//!
//! # These are `pub(crate)` on purpose
//!
//! The job event handler used to carry its own `extract_storage_key` /
//! `extract_mime_type`, narrower than these by several spellings — an asset
//! whose mime type sat in `contentType`, or whose storage key sat directly on
//! an `Object` `file`, was invisible to the enqueue side and perfectly visible
//! to the handler side. The enqueue gate and the processor must agree about
//! what an asset IS, so there is one set of accessors and both call it.

use raisin_error::{Error, Result};
use raisin_models::nodes::Node;
use raisin_storage::jobs::{AssetProcessingOptions, PdfExtractionStrategy};

use super::types::PdfProcessingOutput;

/// Extract the content hash of the binary from node properties.
///
/// Delegates to [`raisin_models::nodes::asset_content_hash`] — the accessors
/// moved down to `raisin-models` when a THIRD caller appeared that could not
/// reach a `pub(crate)` item: a delegated plugin task writing its extraction
/// result back through the function layer. See that module for why a second
/// implementation of the fingerprint is a re-extraction loop.
pub(crate) fn extract_content_hash(node: &Node) -> Option<String> {
    raisin_models::nodes::asset_content_hash(&node.properties)
}

/// Identify the BINARY an extraction result was produced from. See
/// [`raisin_models::nodes::asset_fingerprint`].
pub(crate) fn asset_fingerprint(node: &Node) -> String {
    raisin_models::nodes::asset_fingerprint(node)
}

/// Extract the mime type from node properties.
pub(crate) fn extract_mime_type(node: &Node) -> Option<String> {
    raisin_models::nodes::asset_mime_type(&node.properties)
}

/// Extract the storage key from node properties.
///
/// Keeps the `Result` shape its callers were written against: the shared
/// accessor answers `None` for "the file is not ready", and this is the one
/// call site that wants that as an error.
pub(crate) fn extract_storage_key(node: &Node) -> Result<String> {
    raisin_models::nodes::asset_storage_key(&node.properties)
        .ok_or_else(|| Error::Validation(format!("No storage key found for node: {}", node.id)))
}

/// Process a PDF file and extract text
pub(super) async fn process_pdf(
    data: &[u8],
    options: &AssetProcessingOptions,
) -> Result<PdfProcessingOutput> {
    use raisin_ai::pdf::{PdfProcessingOptions, PdfProcessor, PdfStrategy};

    let strategy = match options.pdf_strategy {
        PdfExtractionStrategy::Auto => PdfStrategy::Auto,
        PdfExtractionStrategy::NativeOnly => PdfStrategy::NativeOnly,
        PdfExtractionStrategy::OcrOnly => PdfStrategy::OcrOnly,
        PdfExtractionStrategy::ForceOcr => PdfStrategy::ForceOcr,
    };

    let pdf_options = PdfProcessingOptions {
        strategy,
        ..Default::default()
    };

    let processor = PdfProcessor::new();
    let result = processor
        .process(data, &pdf_options)
        .await
        .map_err(|e| Error::Backend(format!("PDF processing failed: {}", e)))?;

    Ok(PdfProcessingOutput {
        text: result.text,
        page_count: result.page_count,
        used_ocr: matches!(
            result.method_used,
            raisin_ai::pdf::ExtractionMethod::Ocr | raisin_ai::pdf::ExtractionMethod::Hybrid
        ),
    })
}

/// The mime types this job can actually pull text out of.
///
/// # This is the whole vocabulary, in one place
///
/// It used to be two separate string literals — `mime == "application/pdf"` in
/// the enqueue side and `mime_type.as_deref() == Some("application/pdf")` in the
/// processing side. Two spellings of the same rule is how one of them gets
/// widened and the other does not, giving you a job that is enqueued and then
/// does nothing.
///
/// # Widening it
///
/// Adding a mime type here is only half the change: [`process_extractable`]
/// must gain a branch that can actually read it, or the job will report success
/// and store nothing — which is exactly the failure this whole path was fixed
/// to stop doing. `docx` / `pptx` / `html` belong here the day there is an
/// extractor behind them; until then they are deliberately absent rather than
/// present-and-silent.
pub(crate) fn is_extractable_mime(mime_type: &Option<String>) -> bool {
    matches!(mime_type.as_deref(), Some("application/pdf"))
}

/// Extract text from a supported binary, dispatching on mime type.
///
/// The single dispatch point named by [`is_extractable_mime`]. Returns `None`
/// for a mime type the vocabulary does not claim, so the caller can say so
/// rather than silently succeed.
pub(crate) async fn process_extractable(
    mime_type: &Option<String>,
    data: &[u8],
    options: &AssetProcessingOptions,
) -> Option<Result<PdfProcessingOutput>> {
    match mime_type.as_deref() {
        Some("application/pdf") => Some(process_pdf(data, options).await),
        _ => None,
    }
}

/// Check if a mime type is an image
pub(crate) fn is_image_mime(mime_type: &Option<String>) -> bool {
    mime_type
        .as_ref()
        .map(|m| m.starts_with("image/"))
        .unwrap_or(false)
}
