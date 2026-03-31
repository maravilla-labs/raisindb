//! OCR Provider trait for PDF text extraction fallback.
//!
//! This module defines the trait for OCR providers that can extract text
//! from images (e.g., scanned PDF pages). Implementations can use different
//! OCR backends like Tesseract, cloud services (Google Vision, Azure), or
//! local models via Candle.

use async_trait::async_trait;
use thiserror::Error;

/// Errors that can occur during OCR processing.
#[derive(Debug, Error)]
pub enum OcrError {
    /// The image format is not supported.
    #[error("Unsupported image format: {0}")]
    UnsupportedFormat(String),

    /// Failed to process the image.
    #[error("Image processing failed: {0}")]
    ImageProcessing(String),

    /// OCR model error.
    #[error("OCR model error: {0}")]
    ModelError(String),

    /// The OCR provider is not available or not configured.
    #[error("OCR provider not available: {0}")]
    NotAvailable(String),

    /// Network or API error (for cloud OCR).
    #[error("OCR API error: {0}")]
    ApiError(String),
}

/// Result type for OCR operations.
pub type OcrResult<T> = Result<T, OcrError>;

/// Represents the result of OCR on a single page or image.
#[derive(Debug, Clone)]
pub struct OcrPageResult {
    /// The extracted text from the page.
    pub text: String,
    /// Confidence score (0.0 to 1.0), if available.
    pub confidence: Option<f32>,
    /// Page index (for multi-page documents).
    pub page_index: usize,
}

/// Options for OCR processing.
#[derive(Debug, Clone, Default)]
pub struct OcrOptions {
    /// Language hints for OCR (ISO 639-1 codes, e.g., "en", "de", "es").
    pub languages: Vec<String>,
    /// DPI to use for rendering (for PDF pages).
    pub dpi: Option<u32>,
    /// Whether to preserve layout/formatting.
    pub preserve_layout: bool,
}

/// Trait for OCR providers.
///
/// Implementations can use different OCR backends:
/// - **Tesseract**: Local OCR via tesseract-rs
/// - **Cloud**: Google Vision, Azure Cognitive Services, AWS Textract
/// - **Candle**: Local models like TrOCR via Candle
#[async_trait]
pub trait OcrProvider: Send + Sync {
    /// Perform OCR on raw image bytes.
    ///
    /// # Arguments
    /// * `image_data` - Raw image bytes (PNG, JPEG, etc.)
    /// * `options` - OCR options
    ///
    /// # Returns
    /// The extracted text from the image.
    async fn ocr_image(&self, image_data: &[u8], options: &OcrOptions) -> OcrResult<String>;

    /// Perform OCR on multiple images (e.g., all pages of a PDF).
    ///
    /// Default implementation calls `ocr_image` for each image sequentially.
    /// Implementations can override for batch processing optimizations.
    async fn ocr_images(
        &self,
        images: &[&[u8]],
        options: &OcrOptions,
    ) -> OcrResult<Vec<OcrPageResult>> {
        let mut results = Vec::with_capacity(images.len());
        for (index, image_data) in images.iter().enumerate() {
            let text = self.ocr_image(image_data, options).await?;
            results.push(OcrPageResult {
                text,
                confidence: None,
                page_index: index,
            });
        }
        Ok(results)
    }

    /// Check if the OCR provider is available and ready.
    async fn is_available(&self) -> bool;

    /// Get the provider name for logging/display.
    fn name(&self) -> &str;
}

/// A no-op OCR provider that always returns an error.
///
/// Used as a placeholder when no OCR provider is configured.
pub struct NoOpOcrProvider;

#[async_trait]
impl OcrProvider for NoOpOcrProvider {
    async fn ocr_image(&self, _image_data: &[u8], _options: &OcrOptions) -> OcrResult<String> {
        Err(OcrError::NotAvailable(
            "No OCR provider configured".to_string(),
        ))
    }

    async fn is_available(&self) -> bool {
        false
    }

    fn name(&self) -> &str {
        "none"
    }
}

/// Tesseract OCR provider for extracting text from images.
///
/// This provider uses the system-installed Tesseract OCR engine.
/// Requires Tesseract to be installed and available in PATH.
#[cfg(feature = "ocr")]
pub struct TesseractOcrProvider {
    /// Language(s) to use for OCR (e.g., "eng", "deu", "eng+deu")
    language: String,
}

#[cfg(feature = "ocr")]
impl TesseractOcrProvider {
    /// Create a new TesseractOcrProvider with the specified language.
    ///
    /// # Arguments
    /// * `language` - Tesseract language code (e.g., "eng" for English)
    pub fn new(language: &str) -> Self {
        Self {
            language: language.to_string(),
        }
    }

    /// Create a provider with English as the default language.
    pub fn english() -> Self {
        Self::new("eng")
    }

    /// Check if Tesseract is available via the dependency flags.
    fn check_available() -> bool {
        raisin_deps::DEPENDENCY_FLAGS.is_available("tesseract")
    }

    /// Write image data to a temporary file and return the path.
    fn write_temp_image(image_data: &[u8]) -> OcrResult<tempfile::NamedTempFile> {
        use std::io::Write;

        // Create a temp file with appropriate extension based on image format
        let extension = Self::detect_image_format(image_data)?;

        let mut temp_file = tempfile::Builder::new()
            .suffix(&format!(".{}", extension))
            .tempfile()
            .map_err(|e| OcrError::ImageProcessing(format!("Failed to create temp file: {}", e)))?;

        temp_file
            .write_all(image_data)
            .map_err(|e| OcrError::ImageProcessing(format!("Failed to write temp file: {}", e)))?;

        temp_file
            .flush()
            .map_err(|e| OcrError::ImageProcessing(format!("Failed to flush temp file: {}", e)))?;

        Ok(temp_file)
    }

    /// Detect image format from magic bytes.
    fn detect_image_format(data: &[u8]) -> OcrResult<&'static str> {
        if data.len() < 8 {
            return Err(OcrError::ImageProcessing(
                "Image data too short".to_string(),
            ));
        }

        // PNG: 89 50 4E 47 0D 0A 1A 0A
        if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            return Ok("png");
        }

        // JPEG: FF D8 FF
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Ok("jpg");
        }

        // TIFF: 49 49 2A 00 or 4D 4D 00 2A
        if data.starts_with(&[0x49, 0x49, 0x2A, 0x00])
            || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
        {
            return Ok("tiff");
        }

        // BMP: 42 4D
        if data.starts_with(&[0x42, 0x4D]) {
            return Ok("bmp");
        }

        // WebP: RIFF....WEBP
        if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
            return Ok("webp");
        }

        // GIF: GIF87a or GIF89a
        if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
            return Ok("gif");
        }

        Err(OcrError::UnsupportedFormat(
            "Unknown image format".to_string(),
        ))
    }
}

#[cfg(feature = "ocr")]
#[async_trait]
impl OcrProvider for TesseractOcrProvider {
    async fn ocr_image(&self, image_data: &[u8], options: &OcrOptions) -> OcrResult<String> {
        // Check if Tesseract is available
        if !Self::check_available() {
            return Err(OcrError::NotAvailable(
                "Tesseract is not installed. Enable OCR in Admin Console → Settings → AI Models"
                    .to_string(),
            ));
        }

        // Determine language to use
        let language = if options.languages.is_empty() {
            self.language.clone()
        } else {
            // Tesseract uses + to combine languages
            options.languages.join("+")
        };

        // Write image to temp file (Tesseract requires file path)
        let temp_file = Self::write_temp_image(image_data)?;
        let image_path = temp_file.path().to_string_lossy().to_string();

        // Run OCR in a blocking task since tesseract is synchronous
        let result = tokio::task::spawn_blocking(move || {
            // Create Tesseract instance
            let mut tess = tesseract::Tesseract::new(None, Some(&language)).map_err(|e| {
                OcrError::ModelError(format!("Failed to initialize Tesseract: {}", e))
            })?;

            // Set image
            tess = tess
                .set_image(&image_path)
                .map_err(|e| OcrError::ImageProcessing(format!("Failed to set image: {}", e)))?;

            // Get text
            let text = tess
                .get_text()
                .map_err(|e| OcrError::ModelError(format!("Failed to get text: {}", e)))?;

            Ok::<String, OcrError>(text)
        })
        .await
        .map_err(|e| OcrError::ModelError(format!("OCR task panicked: {}", e)))??;

        Ok(result.trim().to_string())
    }

    async fn is_available(&self) -> bool {
        Self::check_available()
    }

    fn name(&self) -> &str {
        "tesseract"
    }
}

/// Get the default OCR provider based on available dependencies.
///
/// Returns TesseractOcrProvider if Tesseract is available, otherwise NoOpOcrProvider.
#[cfg(feature = "ocr")]
pub fn get_default_ocr_provider() -> Box<dyn OcrProvider> {
    if TesseractOcrProvider::check_available() {
        Box::new(TesseractOcrProvider::english())
    } else {
        Box::new(NoOpOcrProvider)
    }
}

/// Get the default OCR provider (always NoOp when ocr feature is disabled).
#[cfg(not(feature = "ocr"))]
pub fn get_default_ocr_provider() -> Box<dyn OcrProvider> {
    Box::new(NoOpOcrProvider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_noop_provider() {
        let provider = NoOpOcrProvider;
        assert!(!provider.is_available().await);
        assert_eq!(provider.name(), "none");

        let result = provider
            .ocr_image(b"fake image", &OcrOptions::default())
            .await;
        assert!(result.is_err());
    }
}
