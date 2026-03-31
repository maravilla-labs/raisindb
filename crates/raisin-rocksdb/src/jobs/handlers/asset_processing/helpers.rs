//! Helper functions for extracting node properties and processing assets

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{AssetProcessingOptions, PdfExtractionStrategy};

use super::types::PdfProcessingOutput;

/// Extract the mime type from node properties
pub(super) fn extract_mime_type(node: &Node) -> Option<String> {
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
pub(super) fn extract_storage_key(node: &Node) -> Result<String> {
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

/// Check if a mime type is an image
pub(super) fn is_image_mime(mime_type: &Option<String>) -> bool {
    mime_type
        .as_ref()
        .map(|m| m.starts_with("image/"))
        .unwrap_or(false)
}
