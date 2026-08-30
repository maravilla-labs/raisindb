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
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{AssetProcessingOptions, PdfExtractionStrategy};

use super::types::PdfProcessingOutput;

/// Read a `PropertyValue` as a string, or `None`.
fn as_str(pv: &PropertyValue) -> Option<String> {
    match pv {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Extract the content hash of the binary from node properties.
///
/// Looked for in the `file` Resource/Object metadata, then as a top-level
/// `content_hash` property (which `raisin:Asset` declares and the package
/// installer and the on-demand attachment fetch both set).
pub(crate) fn extract_content_hash(node: &Node) -> Option<String> {
    if let Some(PropertyValue::Resource(res)) = node.properties.get("file") {
        if let Some(h) = res
            .metadata
            .as_ref()
            .and_then(|m| m.get("content_hash"))
            .and_then(as_str)
        {
            return Some(h);
        }
    }
    if let Some(PropertyValue::Object(obj)) = node.properties.get("file") {
        if let Some(PropertyValue::Object(meta)) = obj.get("metadata") {
            if let Some(h) = meta.get("content_hash").and_then(as_str) {
                return Some(h);
            }
        }
        if let Some(h) = obj.get("content_hash").and_then(as_str) {
            return Some(h);
        }
    }
    node.properties.get("content_hash").and_then(as_str)
}

/// Identify the BINARY an extraction result was produced from.
///
/// # Why this exists
///
/// Extraction terminates in a node property, and writing a node property emits
/// `node:updated` — which is the very event that enqueues asset processing. So
/// the write-back would re-trigger the job, which would write again, forever.
/// The job stamps this fingerprint alongside the text, and the enqueue gate
/// (`should_process_asset`) skips a node whose stamp already matches. One
/// upload therefore extracts exactly once, and REPLACING the file (new content
/// hash, new storage key, or new size) changes the fingerprint and re-extracts.
///
/// A revision counter or `updated_by` would not do: replication delivers node
/// revisions written by another node's system actor, and a mount sync stamps
/// its own. Only "which bytes was this text made from" answers the question
/// that is actually being asked.
///
/// The `v1:` prefix is the extractor generation. Bump it to force every asset
/// to be re-extracted after a change that makes old output wrong.
pub(crate) fn asset_fingerprint(node: &Node) -> String {
    let hash = extract_content_hash(node).unwrap_or_else(|| "-".to_string());
    let key = extract_storage_key(node).unwrap_or_else(|_| "-".to_string());
    let size = match node.properties.get("file_size") {
        Some(PropertyValue::Integer(n)) => n.to_string(),
        Some(PropertyValue::Float(f)) => f.to_string(),
        _ => "-".to_string(),
    };
    format!("v1:{hash}|{key}|{size}")
}

/// Extract the mime type from node properties
pub(crate) fn extract_mime_type(node: &Node) -> Option<String> {
    // Try file property first (raisin:Asset pattern with Resource type)
    if let Some(PropertyValue::Resource(resource)) = node.properties.get("file") {
        if let Some(ref mime) = resource.mime_type {
            return Some(mime.clone());
        }
        if let Some(ref metadata) = resource.metadata {
            if let Some(PropertyValue::String(mime)) = metadata.get("mime_type") {
                return Some(mime.clone());
            }
            if let Some(PropertyValue::String(mime)) = metadata.get("mimeType") {
                return Some(mime.clone());
            }
        }
    }

    // Try file property as Object (legacy format)
    if let Some(PropertyValue::Object(obj)) = node.properties.get("file") {
        if let Some(PropertyValue::String(mime)) = obj.get("mime_type") {
            return Some(mime.clone());
        }
        if let Some(PropertyValue::String(mime)) = obj.get("mimeType") {
            return Some(mime.clone());
        }
    }

    // Try contentType property
    if let Some(PropertyValue::String(ct)) = node.properties.get("contentType") {
        return Some(ct.clone());
    }

    // Try mimeType property
    if let Some(PropertyValue::String(mt)) = node.properties.get("mimeType") {
        return Some(mt.clone());
    }

    None
}

/// Extract the storage key from node properties
pub(crate) fn extract_storage_key(node: &Node) -> Result<String> {
    // Try file property with storage_key (standard upload format with Resource type)
    if let Some(PropertyValue::Resource(resource)) = node.properties.get("file") {
        if let Some(ref metadata) = resource.metadata {
            if let Some(PropertyValue::String(key)) = metadata.get("storage_key") {
                return Ok(key.clone());
            }
            if let Some(PropertyValue::String(key)) = metadata.get("storageKey") {
                return Ok(key.clone());
            }
        }
    }

    // Try file property as Object (legacy format)
    if let Some(PropertyValue::Object(obj)) = node.properties.get("file") {
        if let Some(PropertyValue::String(key)) = obj.get("storage_key") {
            return Ok(key.clone());
        }
        if let Some(PropertyValue::String(key)) = obj.get("storageKey") {
            return Ok(key.clone());
        }
        // Check nested metadata
        if let Some(PropertyValue::Object(metadata)) = obj.get("metadata") {
            if let Some(PropertyValue::String(key)) = metadata.get("storage_key") {
                return Ok(key.clone());
            }
        }
    }

    // Try resource property (package format)
    if let Some(PropertyValue::Resource(resource)) = node.properties.get("resource") {
        if let Some(ref metadata) = resource.metadata {
            if let Some(PropertyValue::String(key)) = metadata.get("storage_key") {
                return Ok(key.clone());
            }
        }
    }

    Err(Error::Validation(format!(
        "No storage key found for node: {}",
        node.id
    )))
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
