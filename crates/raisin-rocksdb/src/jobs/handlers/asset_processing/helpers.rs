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
use raisin_models::nodes::{Node, MAX_INLINE_EXTRACT_BYTES};
use raisin_storage::jobs::{AssetProcessingOptions, PdfExtractionStrategy};

use super::types::TextExtractionOutput;

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
) -> Result<TextExtractionOutput> {
    use raisin_ai::pdf::{PdfProcessingOptions, PdfProcessor, PdfStrategy};

    let strategy = match options.pdf_strategy {
        PdfExtractionStrategy::Auto => PdfStrategy::Auto,
        PdfExtractionStrategy::NativeOnly => PdfStrategy::NativeOnly,
        PdfExtractionStrategy::OcrOnly => PdfStrategy::OcrOnly,
        PdfExtractionStrategy::ForceOcr => PdfStrategy::ForceOcr,
    };

    // `auto()` (not `Default`) so the native-vs-scan floor is the production
    // value of 50 readable characters per page. The rule's OCR knobs travel
    // with it: they are the same policy the image OCR path applies.
    let mut pdf_options = PdfProcessingOptions::auto();
    pdf_options.strategy = strategy;
    pdf_options.ocr_options.languages = options.ocr_languages.clone();
    pdf_options.ocr_options.min_word_confidence = options.ocr_min_word_confidence;

    let processor = PdfProcessor::new();
    let result = processor
        .process(data, &pdf_options)
        .await
        .map_err(|e| Error::Backend(format!("PDF processing failed: {}", e)))?;

    let used_ocr = matches!(
        result.method_used,
        raisin_ai::pdf::ExtractionMethod::Ocr | raisin_ai::pdf::ExtractionMethod::Hybrid
    );

    // The confidence is the OCR provider's mean over the pages it read; a
    // pure text-layer extraction measures nothing, and absent must not be
    // conflated with zero — see `EXTRACT_CONFIDENCE_PROP`.
    let detail = if used_ocr {
        Some(format!(
            "ocr on {} of {} pages",
            result.ocr_pages.len(),
            result.page_count
        ))
    } else {
        None
    };
    Ok(TextExtractionOutput {
        text: result.text,
        page_count: result.page_count,
        used_ocr,
        source: if used_ocr { "core-pdf-ocr" } else { "core-pdf" },
        confidence: result.ocr_confidence,
        detail,
    })
}

/// OCR an image into text.
///
/// Reaches for the shared provider rather than constructing Tesseract here, so
/// a deployment that swaps in a cloud OCR backend changes one factory and both
/// the PDF fallback and this path follow.
///
/// # An empty result is a result
///
/// Most images hold no text, and that is the expected outcome rather than a
/// fault. It returns `Ok("")`, which the caller records as an ordinary
/// extraction with zero characters — durable, and never retried. Reporting it
/// as an error would put every holiday photo into the retry population; hiding
/// it entirely would leave the asset indistinguishable from one that was never
/// looked at.
///
/// A provider that is genuinely absent (no `ocr` feature, no libtesseract) is a
/// different matter and DOES return an error, so it lands as `failed` and is
/// retried once the binary can actually do the work.
///
/// # The policy lives here, the measurement lives in the provider
///
/// `options` carries the operator's language list and per-word floor. The
/// provider applies the floor while parsing — per-word scores exist only there —
/// but the VALUE is the caller's, resolved from the processing rule at enqueue
/// time. Neither half decides alone.
pub(super) async fn process_image_ocr(
    data: &[u8],
    options: &AssetProcessingOptions,
) -> Result<TextExtractionOutput> {
    use raisin_ai::pdf::ocr::{get_default_ocr_provider, OcrOptions};

    let provider = get_default_ocr_provider();
    let ocr_options = OcrOptions {
        languages: options.ocr_languages.clone(),
        min_word_confidence: options.ocr_min_word_confidence,
        ..Default::default()
    };

    let result = provider
        .ocr_image_detailed(data, &ocr_options)
        .await
        .map_err(|e| Error::Backend(format!("Image OCR failed ({}): {}", provider.name(), e)))?;

    // Tesseract is generous with blank lines and stray whitespace on an image
    // that holds nothing. Normalising here means "did we find text?" is one
    // `is_empty()` for every reader, rather than each one inventing its own
    // idea of blank.
    let text = result.text.trim().to_string();

    // When the floor rejected everything, say so in terms an operator can act
    // on. `__extract_status` will be `empty`, which is truthful but says nothing
    // about WHY — and "this photo has no text" and "this scan was too poor to
    // read" want different responses.
    let detail = match (
        text.is_empty(),
        options.ocr_min_word_confidence,
        result.confidence,
    ) {
        // `mean`, not `best`: when nothing survives the filter the provider
        // reports the average over every word it detected. Calling that the
        // best score would overstate the image — an operator reading "best 31"
        // would conclude no word beat 31, which is not what was measured.
        (true, Some(floor), Some(mean)) if mean < f32::from(floor) => Some(format!(
            "OCR kept no word at or above confidence {floor} (mean {mean:.0})"
        )),
        _ => None,
    };

    Ok(TextExtractionOutput {
        page_count: 1,
        used_ocr: true,
        source: "core-image-ocr",
        confidence: result.confidence,
        detail,
        text,
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
    match mime_type.as_deref() {
        Some("application/pdf") => true,
        // Every image, via OCR. A photo of a whiteboard, a scanned invoice and
        // a screenshot of an error message all carry text that is otherwise
        // unfindable — the filename is the only thing the index ever saw.
        //
        // An image that turns out to hold no text is a NORMAL result, not a
        // failure: it records `ok` with zero characters, which is durable and
        // stops the asset being reconsidered on every sweep.
        // Text that is ALREADY text. Reading a `.txt`, `.md` or `.csv` is a
        // byte decode — same bytes, same output, no model and no page
        // rasterizer — so it belongs in core by the same rule that keeps PDF
        // and OCR here and captioning out.
        //
        // Before this, a text file planned `extract_text` (the routing table
        // has always called `text/*` text-bearing) and then recorded
        // `unsupported`, which reads as "nothing on this server can read these
        // bytes". For a plain text file that is plainly false, and it left the
        // most trivially indexable format in the system findable only by
        // filename.
        Some(m) => m.starts_with("image/") || m.starts_with("text/"),
        None => false,
    }
}

/// Decode a text asset. Lossy on purpose.
///
/// The alternative is failing a whole document over one bad byte, which for an
/// index is strictly worse than a replacement character: a CSV exported from a
/// legacy system with a stray Latin-1 byte is still worth every other row.
/// Truncation is by BYTES with a char-boundary walk, because slicing a UTF-8
/// string mid-codepoint panics.
fn process_text(data: &[u8]) -> TextExtractionOutput {
    let mut text = String::from_utf8_lossy(data).into_owned();
    let mut detail = None;
    if text.len() > MAX_INLINE_EXTRACT_BYTES {
        let mut cut = MAX_INLINE_EXTRACT_BYTES;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        detail = Some(format!("truncated to {MAX_INLINE_EXTRACT_BYTES} bytes"));
    }
    TextExtractionOutput {
        text,
        page_count: 1,
        used_ocr: false,
        source: "text",
        // Not measured rather than perfect: a decode has no confidence to
        // report, and `None` must stay distinguishable from a measured zero.
        confidence: None,
        detail,
    }
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
) -> Option<Result<TextExtractionOutput>> {
    match mime_type.as_deref() {
        Some("application/pdf") => Some(process_pdf(data, options).await),
        Some(m) if m.starts_with("image/") => Some(process_image_ocr(data, options).await),
        Some(m) if m.starts_with("text/") => Some(Ok(process_text(data))),
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

#[cfg(test)]
mod text_extraction_tests {
    use super::*;

    #[test]
    fn a_text_file_is_extractable_and_reads_back_verbatim() {
        assert!(is_extractable_mime(&Some("text/plain".into())));
        assert!(is_extractable_mime(&Some("text/markdown".into())));
        assert!(is_extractable_mime(&Some("text/csv".into())));

        let out = process_text(b"hello\nworld");
        assert_eq!(out.text, "hello\nworld");
        assert_eq!(out.source, "text");
        assert!(!out.used_ocr);
        // A decode measures no confidence; `None` must not read as zero.
        assert!(out.confidence.is_none());
        assert!(out.detail.is_none());
    }

    /// One bad byte must not lose the other rows.
    #[test]
    fn invalid_utf8_degrades_rather_than_failing() {
        let out = process_text(b"ok,\xffbad,rows");
        assert!(out.text.contains("ok,"));
        assert!(out.text.contains("rows"));
    }

    /// Truncation walks back to a char boundary; slicing mid-codepoint panics.
    #[test]
    fn oversized_text_truncates_on_a_char_boundary() {
        let big = "é".repeat(MAX_INLINE_EXTRACT_BYTES);
        let out = process_text(big.as_bytes());
        assert!(out.text.len() <= MAX_INLINE_EXTRACT_BYTES);
        assert!(out.detail.is_some());
        // The real assertion: it is still valid UTF-8 and did not panic.
        assert!(out.text.chars().all(|c| c == 'é'));
    }
}
