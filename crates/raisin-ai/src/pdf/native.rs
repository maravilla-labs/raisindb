//! Native PDF text extraction using pdf-extract.
//!
//! This module provides text extraction from PDFs that contain native text
//! (as opposed to scanned/image-based PDFs that require OCR).

use thiserror::Error;

/// Errors that can occur during PDF text extraction.
#[derive(Debug, Error)]
pub enum PdfExtractError {
    /// The PDF file is invalid or corrupted.
    #[error("Invalid PDF: {0}")]
    InvalidPdf(String),

    /// Failed to extract text from the PDF.
    #[error("Text extraction failed: {0}")]
    ExtractionFailed(String),

    /// The PDF is encrypted and the password is incorrect or not provided.
    #[error("PDF is encrypted: {0}")]
    Encrypted(String),

    /// IO error while reading the PDF.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Result type for PDF extraction operations.
pub type PdfResult<T> = Result<T, PdfExtractError>;

/// Result of extracting text from a PDF.
#[derive(Debug, Clone)]
pub struct PdfTextResult {
    /// Full extracted text (all pages combined).
    pub full_text: String,
    /// Text per page.
    pub pages: Vec<PageText>,
    /// Total page count.
    pub page_count: usize,
    /// Whether the PDF appears to be mostly images (low text content).
    pub is_likely_scanned: bool,
}

/// Text content from a single page.
#[derive(Debug, Clone)]
pub struct PageText {
    /// Page index (0-based).
    pub index: usize,
    /// Extracted text from this page.
    pub text: String,
    /// Character count.
    pub char_count: usize,
}

/// Minimum characters per page to consider it as having "native" text.
#[cfg(feature = "pdf")]
const MIN_CHARS_PER_PAGE: usize = 50;

/// Extract text from a PDF using pdf-extract.
///
/// This function extracts text from PDFs that contain native text content.
/// For scanned PDFs, use the OCR fallback via [`PdfProcessor`].
///
/// # Arguments
/// * `pdf_data` - Raw PDF file bytes
///
/// # Returns
/// A [`PdfTextResult`] containing the extracted text and metadata.
#[cfg(feature = "pdf")]
pub fn extract_text(pdf_data: &[u8]) -> PdfResult<PdfTextResult> {
    use pdf_extract::extract_text_from_mem_by_pages;

    // Extract text from each page
    let pages_text = extract_text_from_mem_by_pages(pdf_data)
        .map_err(|e| PdfExtractError::ExtractionFailed(e.to_string()))?;

    let page_count = pages_text.len();
    let mut pages = Vec::with_capacity(page_count);
    let mut full_text = String::new();
    let mut total_chars = 0usize;

    for (index, text) in pages_text.into_iter().enumerate() {
        let char_count = text.chars().filter(|c| !c.is_whitespace()).count();
        total_chars += char_count;

        if !full_text.is_empty() {
            full_text.push_str("\n\n--- Page ");
            full_text.push_str(&(index + 1).to_string());
            full_text.push_str(" ---\n\n");
        }
        full_text.push_str(&text);

        pages.push(PageText {
            index,
            text,
            char_count,
        });
    }

    // Heuristic: if average chars per page < threshold, likely scanned
    let avg_chars_per_page = if page_count > 0 {
        total_chars / page_count
    } else {
        0
    };
    let is_likely_scanned = avg_chars_per_page < MIN_CHARS_PER_PAGE;

    Ok(PdfTextResult {
        full_text,
        pages,
        page_count,
        is_likely_scanned,
    })
}

/// Extract text from a specific page of a PDF.
///
/// # Arguments
/// * `pdf_data` - Raw PDF file bytes
/// * `page_index` - 0-based page index
///
/// # Returns
/// The extracted text from the specified page.
#[cfg(feature = "pdf")]
pub fn extract_page_text(pdf_data: &[u8], page_index: usize) -> PdfResult<String> {
    use pdf_extract::extract_text_from_mem_by_pages;

    let pages_text = extract_text_from_mem_by_pages(pdf_data)
        .map_err(|e| PdfExtractError::ExtractionFailed(e.to_string()))?;

    pages_text
        .into_iter()
        .nth(page_index)
        .ok_or_else(|| PdfExtractError::InvalidPdf(format!("Page {} not found", page_index)))
}

/// Check if a PDF contains mostly native text or appears to be scanned.
///
/// This is a quick heuristic check that examines the first page.
///
/// # Arguments
/// * `pdf_data` - Raw PDF file bytes
///
/// # Returns
/// `true` if the PDF appears to have native text, `false` if likely scanned.
#[cfg(feature = "pdf")]
pub fn has_native_text(pdf_data: &[u8]) -> PdfResult<bool> {
    let result = extract_text(pdf_data)?;
    Ok(!result.is_likely_scanned)
}

/// Get the page count of a PDF.
#[cfg(feature = "pdf")]
pub fn get_page_count(pdf_data: &[u8]) -> PdfResult<usize> {
    use pdf_extract::extract_text_from_mem_by_pages;

    let pages = extract_text_from_mem_by_pages(pdf_data)
        .map_err(|e| PdfExtractError::ExtractionFailed(e.to_string()))?;

    Ok(pages.len())
}

// Stub implementations when feature is not enabled
#[cfg(not(feature = "pdf"))]
pub fn extract_text(_pdf_data: &[u8]) -> PdfResult<PdfTextResult> {
    Err(PdfExtractError::ExtractionFailed(
        "PDF feature not enabled".to_string(),
    ))
}

#[cfg(not(feature = "pdf"))]
pub fn extract_page_text(_pdf_data: &[u8], _page_index: usize) -> PdfResult<String> {
    Err(PdfExtractError::ExtractionFailed(
        "PDF feature not enabled".to_string(),
    ))
}

#[cfg(not(feature = "pdf"))]
pub fn has_native_text(_pdf_data: &[u8]) -> PdfResult<bool> {
    Err(PdfExtractError::ExtractionFailed(
        "PDF feature not enabled".to_string(),
    ))
}

#[cfg(not(feature = "pdf"))]
pub fn get_page_count(_pdf_data: &[u8]) -> PdfResult<usize> {
    Err(PdfExtractError::ExtractionFailed(
        "PDF feature not enabled".to_string(),
    ))
}

#[cfg(all(test, feature = "pdf"))]
mod tests {
    use super::*;

    // Base64-encoded minimal valid PDF with text content.
    // These PDFs are properly structured with correct xref offsets.
    // Generated using Python with precise byte position tracking.
    fn minimal_pdf_with_text() -> Vec<u8> {
        // Single page PDF with enough text (>50 chars) to pass native text detection
        // Text: "The quick brown fox jumps over the lazy dog multiple times for testing"
        const PDF_BASE64: &str = "JVBERi0xLjQKMSAwIG9iago8PC9UeXBlL0NhdGFsb2cvUGFnZXMgMiAwIFI+PgplbmRvYmoKMiAwIG9iago8PC9UeXBlL1BhZ2VzL0tpZHNbMyAwIFJdL0NvdW50IDE+PgplbmRvYmoKMyAwIG9iago8PC9UeXBlL1BhZ2UvUGFyZW50IDIgMCBSL01lZGlhQm94WzAgMCA2MTIgNzkyXS9Db250ZW50cyA0IDAgUi9SZXNvdXJjZXM8PC9Gb250PDwvRjEgNSAwIFI+Pj4+Pj4KZW5kb2JqCjQgMCBvYmoKPDwvTGVuZ3RoIDEwMz4+CnN0cmVhbQpCVAovRjEgMTIgVGYKMTAwIDcwMCBUZAooVGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZyBtdWx0aXBsZSB0aW1lcyBmb3IgdGVzdGluZykgVGoKRVQKZW5kc3RyZWFtCmVuZG9iago1IDAgb2JqCjw8L1R5cGUvRm9udC9TdWJ0eXBlL1R5cGUxL0Jhc2VGb250L0hlbHZldGljYT4+CmVuZG9iagp4cmVmCjAgNgowMDAwMDAwMDAwIDY1NTM1IGYgCjAwMDAwMDAwMDkgMDAwMDAgbiAKMDAwMDAwMDA1NCAwMDAwMCBuIAowMDAwMDAwMTA1IDAwMDAwIG4gCjAwMDAwMDAyMTcgMDAwMDAgbiAKMDAwMDAwMDM2OCAwMDAwMCBuIAp0cmFpbGVyCjw8L1NpemUgNi9Sb290IDEgMCBSPj4Kc3RhcnR4cmVmCjQzMQolJUVPRgo=";
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(PDF_BASE64)
            .expect("Invalid base64 PDF")
    }

    fn minimal_multi_page_pdf() -> Vec<u8> {
        // Two-page PDF with "Page One" and "Page Two" text
        const PDF_BASE64: &str = "JVBERi0xLjQKMSAwIG9iago8PC9UeXBlL0NhdGFsb2cvUGFnZXMgMiAwIFI+PgplbmRvYmoKMiAwIG9iago8PC9UeXBlL1BhZ2VzL0tpZHNbMyAwIFIgNiAwIFJdL0NvdW50IDI+PgplbmRvYmoKMyAwIG9iago8PC9UeXBlL1BhZ2UvUGFyZW50IDIgMCBSL01lZGlhQm94WzAgMCA2MTIgNzkyXS9Db250ZW50cyA0IDAgUi9SZXNvdXJjZXM8PC9Gb250PDwvRjEgNSAwIFI+Pj4+Pj4KZW5kb2JqCjQgMCBvYmoKPDwvTGVuZ3RoIDQxPj4Kc3RyZWFtCkJUCi9GMSAxMiBUZgoxMDAgNzAwIFRkCihQYWdlIE9uZSkgVGoKRVQKZW5kc3RyZWFtCmVuZG9iago1IDAgb2JqCjw8L1R5cGUvRm9udC9TdWJ0eXBlL1R5cGUxL0Jhc2VGb250L0hlbHZldGljYT4+CmVuZG9iago2IDAgb2JqCjw8L1R5cGUvUGFnZS9QYXJlbnQgMiAwIFIvTWVkaWFCb3hbMCAwIDYxMiA3OTJdL0NvbnRlbnRzIDcgMCBSL1Jlc291cmNlczw8L0ZvbnQ8PC9GMSA1IDAgUj4+Pj4+PgplbmRvYmoKNyAwIG9iago8PC9MZW5ndGggNDE+PgpzdHJlYW0KQlQKL0YxIDEyIFRmCjEwMCA3MDAgVGQKKFBhZ2UgVHdvKSBUagpFVAplbmRzdHJlYW0KZW5kb2JqCnhyZWYKMCA4CjAwMDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAwOSAwMDAwMCBuIAowMDAwMDAwMDU0IDAwMDAwIG4gCjAwMDAwMDAxMTEgMDAwMDAgbiAKMDAwMDAwMDIyMyAwMDAwMCBuIAowMDAwMDAwMzExIDAwMDAwIG4gCjAwMDAwMDAzNzQgMDAwMDAgbiAKMDAwMDAwMDQ4NiAwMDAwMCBuIAp0cmFpbGVyCjw8L1NpemUgOC9Sb290IDEgMCBSPj4Kc3RhcnR4cmVmCjU3NAolJUVPRgo=";
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(PDF_BASE64)
            .expect("Invalid base64 PDF")
    }

    fn minimal_scanned_pdf() -> Vec<u8> {
        // PDF with just "x" - minimal text to trigger scanned detection heuristic
        const PDF_BASE64: &str = "JVBERi0xLjQKMSAwIG9iago8PC9UeXBlL0NhdGFsb2cvUGFnZXMgMiAwIFI+PgplbmRvYmoKMiAwIG9iago8PC9UeXBlL1BhZ2VzL0tpZHNbMyAwIFJdL0NvdW50IDE+PgplbmRvYmoKMyAwIG9iago8PC9UeXBlL1BhZ2UvUGFyZW50IDIgMCBSL01lZGlhQm94WzAgMCA2MTIgNzkyXS9Db250ZW50cyA0IDAgUi9SZXNvdXJjZXM8PC9Gb250PDwvRjEgNSAwIFI+Pj4+Pj4KZW5kb2JqCjQgMCBvYmoKPDwvTGVuZ3RoIDM0Pj4Kc3RyZWFtCkJUCi9GMSAxMiBUZgoxMDAgNzAwIFRkCih4KSBUagpFVAplbmRzdHJlYW0KZW5kb2JqCjUgMCBvYmoKPDwvVHlwZS9Gb250L1N1YnR5cGUvVHlwZTEvQmFzZUZvbnQvSGVsdmV0aWNhPj4KZW5kb2JqCnhyZWYKMCA2CjAwMDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAwOSAwMDAwMCBuIAowMDAwMDAwMDU0IDAwMDAwIG4gCjAwMDAwMDAxMDUgMDAwMDAgbiAKMDAwMDAwMDIxNyAwMDAwMCBuIAowMDAwMDAwMjk4IDAwMDAwIG4gCnRyYWlsZXIKPDwvU2l6ZSA2L1Jvb3QgMSAwIFI+PgpzdGFydHhyZWYKMzYxCiUlRU9GCg==";
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(PDF_BASE64)
            .expect("Invalid base64 PDF")
    }

    #[test]
    fn test_extract_empty_fails() {
        let result = extract_text(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_invalid_fails() {
        let result = extract_text(b"not a valid pdf");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_text_from_valid_pdf() {
        let pdf_bytes = minimal_pdf_with_text();
        let result = extract_text(&pdf_bytes);
        assert!(result.is_ok(), "Failed to extract text: {:?}", result.err());

        let text_result = result.unwrap();
        assert_eq!(text_result.page_count, 1);
        assert!(!text_result.full_text.is_empty());
        // The PDF contains "The quick brown fox jumps over the lazy dog multiple times for testing"
        assert!(
            text_result.full_text.contains("quick")
                || text_result.full_text.contains("fox")
                || text_result.full_text.contains("dog"),
            "Expected text not found in: {}",
            text_result.full_text
        );
    }

    #[test]
    fn test_extract_page_count_single() {
        let pdf_bytes = minimal_pdf_with_text();
        let count = get_page_count(&pdf_bytes);
        assert!(count.is_ok());
        assert_eq!(count.unwrap(), 1);
    }

    #[test]
    fn test_extract_page_count_multi() {
        let pdf_bytes = minimal_multi_page_pdf();
        let count = get_page_count(&pdf_bytes);
        assert!(count.is_ok());
        assert_eq!(count.unwrap(), 2);
    }

    #[test]
    fn test_has_native_text_true() {
        let pdf_bytes = minimal_pdf_with_text();
        let result = has_native_text(&pdf_bytes);
        assert!(result.is_ok());
        assert!(
            result.unwrap(),
            "PDF with text should be detected as having native text"
        );
    }

    #[test]
    fn test_has_native_text_false_for_scanned() {
        let pdf_bytes = minimal_scanned_pdf();
        let result = has_native_text(&pdf_bytes);
        assert!(result.is_ok());
        // This PDF has very little text, should be detected as likely scanned
        assert!(
            !result.unwrap(),
            "PDF with minimal text should be detected as likely scanned"
        );
    }

    #[test]
    fn test_extract_page_text_first_page() {
        let pdf_bytes = minimal_multi_page_pdf();
        let result = extract_page_text(&pdf_bytes, 0);
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(
            text.contains("Page") || text.contains("One"),
            "First page text not found: {}",
            text
        );
    }

    #[test]
    fn test_extract_page_text_second_page() {
        let pdf_bytes = minimal_multi_page_pdf();
        let result = extract_page_text(&pdf_bytes, 1);
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(
            text.contains("Page") || text.contains("Two"),
            "Second page text not found: {}",
            text
        );
    }

    #[test]
    fn test_extract_page_text_invalid_index() {
        let pdf_bytes = minimal_pdf_with_text();
        let result = extract_page_text(&pdf_bytes, 999);
        assert!(result.is_err());
    }

    #[test]
    fn test_pdf_text_result_pages_populated() {
        let pdf_bytes = minimal_multi_page_pdf();
        let result = extract_text(&pdf_bytes).unwrap();

        assert_eq!(result.pages.len(), 2);
        assert_eq!(result.pages[0].index, 0);
        assert_eq!(result.pages[1].index, 1);
        assert!(result.pages[0].char_count > 0 || result.pages[1].char_count > 0);
    }
}
