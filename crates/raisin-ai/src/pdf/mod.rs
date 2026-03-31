//! PDF processing module for text extraction.
//!
//! This module provides PDF processing using pdf_oxide, a pure Rust library
//! that extracts text as markdown (optimal for LLMs).
//!
//! # Features
//!
//! - **Markdown output**: Extracted text is formatted as markdown
//! - **Built-in OCR**: Automatic OCR for scanned PDFs
//! - **Pure Rust**: No C dependencies (pdfium not required)
//! - **Storage-aware**: Works with filesystem and S3 storage
//!
//! # Usage
//!
//! ```rust,ignore
//! use raisin_ai::pdf::{process_pdf_from_storage, StoragePdfOptions};
//!
//! let result = process_pdf_from_storage(
//!     &storage,
//!     "uploads/doc.pdf",
//!     StoragePdfOptions::default(),
//! ).await?;
//!
//! println!("Pages: {}\nMarkdown:\n{}", result.page_count, result.text);
//! ```
//!
//! # Feature Flags
//!
//! - `pdf` - Enable basic PDF text extraction (native via pdf-extract)
//! - `pdf-markdown` - Enable PDF to markdown conversion (via pdf_oxide)

// Legacy native extraction (kept for backward compatibility)
#[cfg(feature = "pdf")]
pub mod native;

// OCR provider trait and types (kept for rules system)
pub mod ocr;

// Router types for backward compatibility (PdfStrategy, ExtractionMethod, etc.)
mod router;

// Storage-aware processing (primary API)
pub mod storage_processor;

// Re-export the main API
pub use storage_processor::{
    process_pdf_from_storage, StoragePdfError, StoragePdfOptions, StoragePdfResult,
};

// Re-export router types for backward compatibility (used by rules.rs)
pub use router::{
    ExtractionMethod, PdfProcessedResult, PdfProcessingError, PdfProcessingOptions, PdfProcessor,
    PdfStrategy,
};

// Re-export OCR types (used by rules.rs and processing)
pub use ocr::{get_default_ocr_provider, OcrError, OcrOptions, OcrProvider};

// Legacy native extraction exports (for backward compatibility)
#[cfg(feature = "pdf")]
pub use native::{
    extract_page_text, extract_text, get_page_count, has_native_text, PageText, PdfExtractError,
    PdfResult, PdfTextResult,
};
