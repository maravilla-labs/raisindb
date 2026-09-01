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
    /// Languages to recognise, as **Tesseract** codes — `eng`, `deu`, `fra`.
    ///
    /// NOT ISO 639-1: these are passed to Tesseract verbatim (joined with `+`),
    /// and it loads one traineddata file per code. `en` names no file and fails
    /// to initialise. This doc used to say ISO 639-1, which meant following it
    /// produced an init error.
    ///
    /// Empty falls back to the provider's own default. Each additional language
    /// costs roughly 10 MB of resident model and ~0.06s per image; all 112
    /// installed at once measured 1.18 GB and 4.5s, which is why callers are
    /// expected to name a handful rather than everything.
    pub languages: Vec<String>,
    /// DPI to assert for the image. `None` — the default — lets Tesseract
    /// estimate it, which is almost always the right answer.
    ///
    /// MEASURED, because the opposite is tempting: Tesseract logs
    /// `Invalid resolution 0 dpi. Using 70 instead` for an image whose metadata
    /// carries none, which reads like something to fix. It is not. That warning
    /// is about the file's METADATA; Tesseract separately estimates the real
    /// resolution from the image ("Estimating resolution as 448" for a 4032x3024
    /// photo) and uses that. Asserting 300 overrides the estimate, tells the
    /// layout analyser the glyphs are a physical size they are not, and it then
    /// discards them: the same photo went from reading a sign's text correctly
    /// to returning NOTHING. Set this only for a source whose true DPI you know.
    pub dpi: Option<u32>,
    /// Whether to preserve layout/formatting.
    pub preserve_layout: bool,
    /// Drop words the engine scores below this (0-100), before assembling text.
    ///
    /// This is the caller's POLICY and the provider only applies it, because
    /// per-word scores exist only while the engine's output is being parsed and
    /// are gone by the time a `String` reaches the caller.
    ///
    /// `None` keeps everything. The value earns its keep on photographs: a
    /// sideways 12 MP frame yielded `|`(27) `am`(27) `Rhein`(95) `+49`(81)
    /// `7621`(93) `79080`(95) — the real sign text and two hallucinated tokens,
    /// separated by a gap no document-level average can express.
    pub min_word_confidence: Option<u8>,
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
    async fn ocr_image(&self, image_data: &[u8], options: &OcrOptions) -> OcrResult<String> {
        Ok(self.ocr_image_detailed(image_data, options).await?.text)
    }

    /// OCR one image and report **how sure the engine was**.
    ///
    /// The confidence is what lets a caller tell "this page says nothing" from
    /// "this page was unreadable", which the text alone cannot: both arrive as
    /// a short string, and one of them is worth indexing.
    ///
    /// `confidence` is `None` when the provider does not measure one — NOT zero.
    /// Zero would mean "measured, and terrible", so a caller filtering on
    /// `< 70` would sweep up every result from a provider that simply does not
    /// report. Implementations without a score leave it absent and say so here.
    ///
    /// This is the method implementations should override; [`Self::ocr_image`]
    /// is a projection of it, so there is one recognition path and not two to
    /// drift apart.
    async fn ocr_image_detailed(
        &self,
        image_data: &[u8],
        options: &OcrOptions,
    ) -> OcrResult<OcrPageResult>;

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

/// Column index of `conf` in Tesseract's TSV output.
const TSV_CONF_COL: usize = 10;
/// Column index of `text` — the last column, so it is also the arity check.
const TSV_TEXT_COL: usize = 11;
/// TSV `level` value that denotes a word (the rows carrying text and a score).
const TSV_LEVEL_WORD: &str = "5";

/// Turn Tesseract's TSV into text, dropping words scored below `min_confidence`.
///
/// Deliberately free-standing and NOT behind the `ocr` feature: this is the part
/// with the interesting behaviour, and gating it would mean it could only be
/// tested on a machine with libtesseract installed.
///
/// # What the confidence means
///
/// The reported score is the mean over the words that were KEPT, because that is
/// what describes the text we are about to store. When the filter rejects
/// everything, the mean over all detected words is reported instead — so an
/// operator sees "we measured 31 and kept nothing" rather than a bare absence.
/// `None` means no words were detected at all.
fn parse_tsv(tsv: &str, min_confidence: Option<u8>) -> OcrPageResult {
    let floor = min_confidence.map(f32::from).unwrap_or(f32::MIN);

    // (block, par, line) -> the words kept on that line, in order.
    let mut lines: Vec<(String, Vec<String>)> = Vec::new();
    let mut kept: Vec<f32> = Vec::new();
    let mut all: Vec<f32> = Vec::new();

    for row in tsv.lines().skip(1) {
        let cols: Vec<&str> = row.split('\t').collect();
        if cols.len() <= TSV_TEXT_COL || cols[0] != TSV_LEVEL_WORD {
            continue;
        }
        let text = cols[TSV_TEXT_COL].trim();
        if text.is_empty() {
            continue;
        }
        let Ok(conf) = cols[TSV_CONF_COL].parse::<f32>() else {
            continue;
        };
        all.push(conf);
        if conf < floor {
            continue;
        }
        kept.push(conf);

        // block/par/line together identify the line this word sits on, so the
        // stored text keeps the document's line breaks instead of becoming one
        // run-on paragraph.
        let line_key = format!("{}:{}:{}", cols[2], cols[3], cols[4]);
        match lines.last_mut() {
            Some((key, words)) if *key == line_key => words.push(text.to_string()),
            _ => lines.push((line_key, vec![text.to_string()])),
        }
    }

    let text = lines
        .into_iter()
        .map(|(_, words)| words.join(" "))
        .collect::<Vec<_>>()
        .join("\n");

    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let confidence = if !kept.is_empty() {
        Some(mean(&kept))
    } else if !all.is_empty() {
        Some(mean(&all))
    } else {
        None
    };

    OcrPageResult {
        text: text.trim().to_string(),
        confidence,
        page_index: 0,
    }
}

/// A no-op OCR provider that always returns an error.
///
/// Used as a placeholder when no OCR provider is configured.
pub struct NoOpOcrProvider;

#[async_trait]
impl OcrProvider for NoOpOcrProvider {
    async fn ocr_image_detailed(
        &self,
        _image_data: &[u8],
        _options: &OcrOptions,
    ) -> OcrResult<OcrPageResult> {
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
    async fn ocr_image_detailed(
        &self,
        image_data: &[u8],
        options: &OcrOptions,
    ) -> OcrResult<OcrPageResult> {
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

        let dpi = options.dpi;
        let min_conf = options.min_word_confidence;

        // Run OCR in a blocking task since tesseract is synchronous
        let result = tokio::task::spawn_blocking(move || {
            // Create Tesseract instance
            let mut tess = tesseract::Tesseract::new(None, Some(&language)).map_err(|e| {
                OcrError::ModelError(format!(
                    "Failed to initialize Tesseract for languages '{}': {}. \
                     Codes are Tesseract's own (eng, deu), not ISO 639-1 (en, de), \
                     and each needs its traineddata installed.",
                    language, e
                ))
            })?;

            // Only when the CALLER knows the resolution. Left unset, Tesseract
            // estimates it from the image, and its estimate beats any constant
            // we could pick — see `OcrOptions::dpi` for the measurement.
            if let Some(dpi) = dpi {
                tess = tess
                    .set_variable("user_defined_dpi", &dpi.to_string())
                    .map_err(|e| OcrError::ModelError(format!("Failed to set DPI: {}", e)))?;
            }

            // Set image
            tess = tess
                .set_image(&image_path)
                .map_err(|e| OcrError::ImageProcessing(format!("Failed to set image: {}", e)))?;

            // ORIENTATION DETECTION, and the reason a photograph reads at all.
            //
            // Tesseract does not honour EXIF orientation — it reads raw pixels —
            // and the default PSM does no orientation detection either. A phone
            // photo stored sideways is therefore fed to the layout analyser
            // rotated, which finds text-like structure in edges and texture and
            // invents glyphs from it. Measured on one 4032x3024 frame: the
            // default produced 28 words of noise at mean confidence 31, while
            // this mode produced the sign's actual text at 73.
            //
            // Needs `osd.traineddata`; when it is missing Tesseract falls back
            // to plain auto segmentation rather than failing, so this is safe to
            // set unconditionally.
            tess.set_page_seg_mode(tesseract::PageSegMode::PsmAutoOsd);

            let mut tess = tess
                .recognize()
                .map_err(|e| OcrError::ModelError(format!("Failed to recognize: {}", e)))?;

            // TSV rather than plain text: it is the only output carrying a
            // PER-WORD confidence, and the gap between real and hallucinated
            // words lives there, not in the page average.
            let tsv = tess
                .get_tsv_text(0)
                .map_err(|e| OcrError::ModelError(format!("Failed to get TSV: {}", e)))?;

            Ok::<OcrPageResult, OcrError>(parse_tsv(&tsv, min_conf))
        })
        .await
        .map_err(|e| OcrError::ModelError(format!("OCR task panicked: {}", e)))??;

        Ok(result)
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

    /// The header Tesseract emits, then rows.
    fn tsv(rows: &[&str]) -> String {
        let mut s = String::from(
            "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\t\
             left\ttop\twidth\theight\tconf\ttext\n",
        );
        for r in rows {
            s.push_str(r);
            s.push('\n');
        }
        s
    }

    /// A word row on line `line`, scored `conf`.
    fn word(line: u32, n: u32, conf: &str, text: &str) -> String {
        format!("5\t1\t1\t1\t{line}\t{n}\t0\t0\t10\t10\t{conf}\t{text}")
    }

    /// The real output from the photograph that prompted this work: a German
    /// sign read at high confidence, plus two tokens hallucinated from texture.
    fn sign_rows() -> Vec<String> {
        vec![
            word(1, 1, "26.846680", "|"),
            word(1, 2, "26.846680", "am"),
            word(1, 3, "94.987869", "Rhein"),
            word(2, 1, "80.984947", "+49"),
            word(2, 2, "92.998787", "7621"),
            word(2, 3, "95.255470", "79080"),
        ]
    }

    fn parse(rows: &[String], min: Option<u8>) -> OcrPageResult {
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        parse_tsv(&tsv(&refs), min)
    }

    #[test]
    fn the_filter_keeps_the_sign_and_drops_the_hallucinations() {
        let got = parse(&sign_rows(), Some(50));
        assert_eq!(
            got.text, "Rhein\n+49 7621 79080",
            "the two 27-confidence tokens are texture read as glyphs; \
             everything else is the sign"
        );
        // Mean over KEPT words, not all six — it describes the text stored.
        let c = got.confidence.expect("words were detected");
        assert!((c - 91.06).abs() < 0.5, "expected ~91.06, got {c}");
    }

    #[test]
    fn without_a_threshold_everything_survives() {
        let got = parse(&sign_rows(), None);
        assert_eq!(got.text, "| am Rhein\n+49 7621 79080");
    }

    /// Line structure comes from block/par/line, not from word order — a
    /// document that lost its line breaks would be one run-on paragraph.
    #[test]
    fn line_breaks_follow_the_layout() {
        let rows = vec![
            word(1, 1, "95", "INVOICE"),
            word(2, 1, "95", "Total"),
            word(2, 2, "95", "due"),
        ];
        assert_eq!(parse(&rows, Some(50)).text, "INVOICE\nTotal due");
    }

    /// The case the whole filter exists for: nothing survives, and the caller
    /// still learns what was measured rather than getting a bare absence.
    #[test]
    fn rejecting_everything_still_reports_what_was_measured() {
        let rows = vec![word(1, 1, "31.0", "aa"), word(1, 2, "31.0", "ileeee")];
        let got = parse(&rows, Some(50));
        assert_eq!(got.text, "");
        assert_eq!(
            got.confidence,
            Some(31.0),
            "an operator must see the 31 that caused the rejection"
        );
    }

    #[test]
    fn an_image_with_no_words_has_no_confidence() {
        // Absent, NOT zero: zero would mean "measured and terrible", and a
        // `< 70` sweep must not collect results that were never scored.
        assert_eq!(parse(&[], Some(50)).confidence, None);
    }

    /// Non-word rows (page/block/para/line levels) carry `conf = -1` and no
    /// text; counting them would drag every average down.
    #[test]
    fn structural_rows_are_not_words() {
        let rows = vec![
            "1\t1\t0\t0\t0\t0\t0\t0\t10\t10\t-1\t".to_string(),
            "4\t1\t1\t1\t1\t0\t0\t0\t10\t10\t-1\t".to_string(),
            word(1, 1, "95", "Rhein"),
        ];
        let got = parse(&rows, Some(50));
        assert_eq!(got.text, "Rhein");
        assert_eq!(got.confidence, Some(95.0));
    }

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
