//! PDF processing types for backward compatibility.
//!
//! This module provides type definitions for PDF processing that are used
//! by the rules system. The actual PDF processing has moved to pdf_oxide
//! via storage_processor.rs.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::ocr::{OcrError, OcrOptions};

/// Errors that can occur during PDF processing.
#[derive(Debug, Error)]
pub enum PdfProcessingError {
    /// Extraction failed.
    #[error("PDF extraction failed: {0}")]
    Extraction(String),

    /// OCR processing failed.
    #[error("OCR failed: {0}")]
    Ocr(#[from] OcrError),

    /// Page rendering failed (for OCR).
    #[error("Page rendering failed: {0}")]
    Rendering(String),

    /// The PDF feature is not enabled.
    #[error("PDF processing not available: {0}")]
    NotAvailable(String),
}

/// Result type for PDF processing operations.
pub type ProcessingResult<T> = Result<T, PdfProcessingError>;

/// Strategy for processing PDF files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PdfStrategy {
    /// Automatically detect: try native extraction first, fall back to OCR.
    #[default]
    Auto,
    /// Only use native text extraction (fail if PDF is scanned).
    NativeOnly,
    /// Only use OCR (even if PDF has native text).
    OcrOnly,
    /// Force OCR regardless of native text availability.
    /// Unlike OcrOnly, this is an explicit user override.
    ForceOcr,
}

impl PdfStrategy {
    /// Returns true if this strategy should try native extraction.
    pub fn should_try_native(&self) -> bool {
        matches!(self, PdfStrategy::Auto | PdfStrategy::NativeOnly)
    }

    /// Returns true if this strategy allows OCR fallback.
    pub fn allows_ocr(&self) -> bool {
        matches!(
            self,
            PdfStrategy::Auto | PdfStrategy::OcrOnly | PdfStrategy::ForceOcr
        )
    }
}

/// Options for PDF processing.
#[derive(Debug, Clone, Default)]
pub struct PdfProcessingOptions {
    /// Processing strategy.
    pub strategy: PdfStrategy,
    /// OCR options (if OCR is used).
    pub ocr_options: OcrOptions,
    /// Minimum characters per page to consider native extraction successful.
    pub min_chars_per_page: usize,
}

impl PdfProcessingOptions {
    /// Create options for automatic processing with defaults.
    pub fn auto() -> Self {
        Self {
            strategy: PdfStrategy::Auto,
            min_chars_per_page: 50,
            ..Default::default()
        }
    }

    /// Create options for native-only extraction.
    pub fn native_only() -> Self {
        Self {
            strategy: PdfStrategy::NativeOnly,
            ..Default::default()
        }
    }

    /// Create options for OCR-only processing.
    pub fn ocr_only() -> Self {
        Self {
            strategy: PdfStrategy::OcrOnly,
            ..Default::default()
        }
    }
}

/// Result of PDF processing with metadata about which method was used.
#[derive(Debug, Clone)]
pub struct PdfProcessedResult {
    /// The extracted text.
    pub text: String,
    /// Which method was used for extraction.
    pub method_used: ExtractionMethod,
    /// Number of pages processed.
    pub page_count: usize,
    /// Whether OCR was needed (for scanned pages).
    pub ocr_pages: Vec<usize>,
    /// Mean OCR confidence (0-100) over `ocr_pages`, when the provider
    /// measures one. `None` when no page was OCR'd or the provider does not
    /// score — never zero for "not measured".
    pub ocr_confidence: Option<f32>,
}

/// The method used for text extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionMethod {
    /// Native text extraction (PDF has embedded text).
    Native,
    /// OCR-based extraction (PDF was scanned/image-based).
    Ocr,
    /// Hybrid: some pages native, some OCR.
    Hybrid,
}

/// Bytes-shaped entry point to PDF text extraction.
///
/// This used to be a stub that returned `NotAvailable("Use
/// process_pdf_from_storage() instead")` unconditionally — while being the only
/// thing the AssetProcessing job could call, since that job holds bytes and not
/// a `BinaryStorage` handle. The result was an "extract PDF text" option that
/// could never succeed.
///
/// It now delegates to the same `extract_markdown_from_path` body that
/// `process_pdf_from_storage` uses, staging the bytes in a temp file exactly as
/// the S3 branch of `get_as_path` does. There is ONE extractor; these are two
/// ways of handing it a path.
pub struct PdfProcessor;

impl PdfProcessor {
    /// Create a new PDF processor.
    pub fn new() -> Self {
        Self
    }

    /// Process a PDF file held in memory, returning markdown text.
    ///
    /// Reads every page's text layer, and — as `options.strategy` says — OCRs
    /// the pages that have none through their embedded scan image. See the
    /// `pages` module for the mechanism and the failure policy. This is the
    /// same implementation `process_pdf_from_storage` runs; do not add a
    /// second extractor here — that is how the fork came back.
    pub async fn process(
        &self,
        pdf_data: &[u8],
        options: &PdfProcessingOptions,
    ) -> ProcessingResult<PdfProcessedResult> {
        // Blocking file + parse work: keep it off the async worker thread.
        let data = pdf_data.to_vec();
        let opts = options.clone();
        let pages = tokio::task::spawn_blocking(move || {
            super::storage_processor::extract_pages_from_bytes(&data, &opts)
        })
        .await
        .map_err(|e| PdfProcessingError::Extraction(format!("PDF task panicked: {e}")))?
        .map_err(|e| PdfProcessingError::Extraction(e.to_string()))?;

        let provider = super::ocr::get_default_ocr_provider();
        Ok(super::pages::finish_with_ocr(pages, &*provider, &options.ocr_options).await?)
    }
}

impl Default for PdfProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strategy_should_try_native() {
        assert!(PdfStrategy::Auto.should_try_native());
        assert!(PdfStrategy::NativeOnly.should_try_native());
        assert!(!PdfStrategy::OcrOnly.should_try_native());
        assert!(!PdfStrategy::ForceOcr.should_try_native());
    }

    #[test]
    fn test_strategy_allows_ocr() {
        assert!(PdfStrategy::Auto.allows_ocr());
        assert!(!PdfStrategy::NativeOnly.allows_ocr());
        assert!(PdfStrategy::OcrOnly.allows_ocr());
        assert!(PdfStrategy::ForceOcr.allows_ocr());
    }

    #[test]
    fn test_options_defaults() {
        let opts = PdfProcessingOptions::auto();
        assert_eq!(opts.strategy, PdfStrategy::Auto);
        assert_eq!(opts.min_chars_per_page, 50);
    }
}
