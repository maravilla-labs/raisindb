//! Storage-aware PDF processing using pdf_oxide.
//!
//! This module provides a high-level API for processing PDFs stored in
//! binary storage backends (filesystem or S3). It extracts text as markdown
//! which is optimal for LLM consumption.

use raisin_binary::BinaryStorage;
use serde::{Deserialize, Serialize};

/// Options for processing a PDF from storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoragePdfOptions {
    /// Detect and format headings in markdown output.
    #[serde(default = "default_true")]
    pub detect_headings: bool,

    /// Preserve layout structure in output.
    #[serde(default)]
    pub preserve_layout: bool,
}

fn default_true() -> bool {
    true
}

impl Default for StoragePdfOptions {
    fn default() -> Self {
        Self {
            detect_headings: true,
            preserve_layout: false,
        }
    }
}

/// Result of processing a PDF from storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoragePdfResult {
    /// Extracted text as markdown (optimal for LLMs).
    pub text: String,

    /// Number of pages in the PDF.
    pub page_count: usize,

    /// Whether the PDF appears to be scanned (image-based).
    pub is_scanned: bool,

    /// Whether OCR was used for text extraction.
    pub ocr_used: bool,

    /// Extraction method used.
    pub extraction_method: String,
}

/// Error type for storage PDF processing.
#[derive(Debug, thiserror::Error)]
pub enum StoragePdfError {
    #[error("Storage error: {0}")]
    Storage(String),

    #[error("PDF processing error: {0}")]
    Processing(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Process a PDF from binary storage and extract text as markdown.
///
/// This function is **storage-agnostic**: it works transparently with both
/// filesystem and S3 storage backends.
///
/// # Flow
///
/// 1. **Get file path** - Uses `storage.get_as_path(key)`:
///    - Filesystem: Returns actual file path directly (zero-copy)
///    - S3/R2: Downloads to temp file, returns temp path
/// 2. **Extract markdown** using pdf_oxide (pure Rust, includes OCR)
/// 3. **Cleanup temp file** if downloaded from S3
///
/// # Example
///
/// ```rust,ignore
/// let result = process_pdf_from_storage(
///     &storage,
///     "uploads/tenant/doc.pdf",
///     StoragePdfOptions::default(),
/// ).await?;
///
/// println!("Pages: {}, Markdown:\n{}", result.page_count, result.text);
/// ```
#[cfg(feature = "pdf-markdown")]
pub async fn process_pdf_from_storage<B: BinaryStorage>(
    storage: &B,
    storage_key: &str,
    _options: StoragePdfOptions,
) -> Result<StoragePdfResult, StoragePdfError> {
    // 1. Get file path from storage (filesystem = direct, S3 = temp file)
    let (file_path, is_temp) = storage
        .get_as_path(storage_key)
        .await
        .map_err(|e| StoragePdfError::Storage(e.to_string()))?;

    // Ensure cleanup on all exit paths
    let _cleanup_guard = if is_temp {
        Some(TempFileCleanup(file_path.clone()))
    } else {
        None
    };

    // 2-3. The ONE extraction implementation (see `extract_markdown_from_path`).
    extract_markdown_from_path(&file_path)
}

/// Extract a whole PDF as markdown from a path — **the single extraction
/// implementation in this crate**.
///
/// Both public entry points funnel through here: [`process_pdf_from_storage`]
/// (storage key; zero-copy on the filesystem backend) and
/// [`crate::pdf::PdfProcessor::process`] (raw bytes, which is what the
/// AssetProcessing job holds). They differ only in how they obtain a path.
/// Keeping one body is deliberate: a second copy is exactly how you end up with
/// a "PDF extraction" that works through one surface and silently returns
/// nothing through the other — which is the state this replaced.
#[cfg(feature = "pdf-markdown")]
pub(crate) fn extract_markdown_from_path(
    file_path: &std::path::Path,
) -> Result<StoragePdfResult, StoragePdfError> {
    use pdf_oxide::converters::ConversionOptions;
    use pdf_oxide::PdfDocument;

    let mut doc =
        PdfDocument::open(file_path).map_err(|e| StoragePdfError::Processing(e.to_string()))?;

    let page_count = doc
        .page_count()
        .map_err(|e| StoragePdfError::Processing(e.to_string()))?;

    // pdf_oxide's to_markdown returns a String directly
    let mut full_markdown = String::new();
    let conversion_options = ConversionOptions::default();

    for page_idx in 0..page_count {
        let page_markdown = doc
            .to_markdown(page_idx, &conversion_options)
            .map_err(|e| StoragePdfError::Processing(format!("Page {}: {}", page_idx, e)))?;

        if !full_markdown.is_empty() {
            full_markdown.push_str("\n\n---\n\n"); // Page separator
        }
        full_markdown.push_str(&page_markdown);
    }

    Ok(StoragePdfResult {
        text: full_markdown,
        page_count,
        is_scanned: false, // pdf_oxide doesn't expose this directly
        ocr_used: false,   // Would need to check doc metadata
        extraction_method: "pdf_oxide".to_string(),
    })
}

/// Extract a PDF held in memory by staging it in the OS temp directory.
///
/// The staging is not a workaround: `get_as_path` does the very same thing for
/// the S3 backend, because pdf_oxide reads from a path. This exists so the
/// bytes-shaped caller reaches [`extract_markdown_from_path`] instead of
/// growing its own extractor.
#[cfg(feature = "pdf-markdown")]
pub(crate) fn extract_markdown_from_bytes(
    data: &[u8],
) -> Result<StoragePdfResult, StoragePdfError> {
    let path = std::env::temp_dir().join(format!("raisin-pdf-{}.pdf", uuid::Uuid::new_v4()));
    std::fs::write(&path, data)?;
    let _cleanup_guard = TempFileCleanup(path.clone());
    extract_markdown_from_path(&path)
}

/// Extract a PDF held in memory — stub when `pdf-markdown` is off.
///
/// The server's default `ai` feature turns `pdf-markdown` on, so this arm is
/// what a `--no-default-features` build gets. It FAILS loudly rather than
/// returning empty text, which would be indistinguishable from "this PDF had
/// no words in it".
#[cfg(not(feature = "pdf-markdown"))]
pub(crate) fn extract_markdown_from_bytes(
    _data: &[u8],
) -> Result<StoragePdfResult, StoragePdfError> {
    Err(StoragePdfError::Processing(
        "PDF text extraction requires the `pdf-markdown` feature (enabled by the server's `ai` feature)"
            .to_string(),
    ))
}

/// RAII guard for temp file cleanup.
#[cfg_attr(not(feature = "pdf-markdown"), allow(dead_code))]
struct TempFileCleanup(std::path::PathBuf);

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        // Best-effort cleanup - ignore errors
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Stub when pdf-markdown feature is not enabled.
#[cfg(not(feature = "pdf-markdown"))]
pub async fn process_pdf_from_storage<B: BinaryStorage>(
    _storage: &B,
    _storage_key: &str,
    _options: StoragePdfOptions,
) -> Result<StoragePdfResult, StoragePdfError> {
    Err(StoragePdfError::Processing(
        "PDF processing requires pdf-markdown feature".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_options_defaults() {
        let opts = StoragePdfOptions::default();
        assert!(opts.detect_headings);
        assert!(!opts.preserve_layout);
    }

    #[test]
    fn test_options_serde() {
        let json = r#"{"detectHeadings": false, "preserveLayout": true}"#;
        let opts: StoragePdfOptions = serde_json::from_str(json).unwrap();
        assert!(!opts.detect_headings);
        assert!(opts.preserve_layout);
    }
}
