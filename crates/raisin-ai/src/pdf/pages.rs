//! Page-level PDF extraction with an OCR fallback for scanned pages.
//!
//! # Why this exists
//!
//! `pdf_oxide` reads the PDF's *text layer*. A scanned document — a phone
//! camera "scan", a fax, an old contract — has none: each page is one large
//! JPEG and nothing else, so the text layer is legitimately empty. Before this
//! module, that outcome was stored as `__extract_status = 'empty'` with
//! `source = core-pdf`, indistinguishable from a blank page, and the operator's
//! `pdf_strategy` (auto / force_ocr) was accepted and then ignored. Measured on
//! a production tenant: 30 of ~5,100 PDFs processed in a week came back empty,
//! every one of them a scan.
//!
//! # How the fallback works without a rasterizer
//!
//! Core ships no page renderer (no pdfium, no poppler), and must not grow one
//! for this. It does not need to: a scanned page IS an image XObject, and
//! `pdf_oxide` can hand that image back as bytes. So the fallback is
//!
//! 1. read the text layer of every page (`to_markdown`);
//! 2. for a page the strategy says to OCR — always under `force_ocr` /
//!    `ocr_only`, only when the text layer is (near-)empty under `auto` —
//!    take the LARGEST image on the page and keep its bytes as the OCR
//!    candidate (JPEG passes through untouched; raw pixel data is PNG-encoded);
//! 3. run the configured [`OcrProvider`] over the candidates.
//!
//! A page whose text layer is empty and that carries no image stays empty:
//! there is nothing to read.
//!
//! # Two halves, deliberately
//!
//! Step 1–2 are synchronous parsing work ([`extract_pages_from_path`], run
//! under `spawn_blocking` by the callers); step 3 is async because every
//! provider is ([`finish_with_ocr`]). Keeping the split lets the OCR half be
//! tested with a fake provider and lets both public entry points — the
//! storage-key one and the bytes one — share ONE implementation, which is the
//! invariant `storage_processor` already documents.
//!
//! # Failure policy
//!
//! OCR failing on a page is not the same as the document failing. A text PDF
//! with one blank page must not be marked `failed` because Tesseract is absent
//! on this box. So a per-page OCR error is logged and the page keeps whatever
//! native text it had. The extraction as a whole fails only when it would
//! otherwise return NOTHING and at least one page's OCR errored — then the
//! honest status is `failed` (retryable once the dependency is installed), not
//! `empty` (durable, never looked at again).

use super::ocr::{OcrError, OcrOptions, OcrProvider};
use super::router::{ExtractionMethod, PdfProcessedResult, PdfProcessingOptions, PdfStrategy};

/// One page after the synchronous pass.
#[derive(Debug, Clone, Default)]
pub(crate) struct PageExtraction {
    /// Text from the PDF's text layer (markdown). May be empty.
    pub text: String,
    /// Image bytes (JPEG or PNG) to OCR, when the strategy asked for it and
    /// the page carried an image. `None` means "the text layer is the answer".
    pub ocr_candidate: Option<Vec<u8>>,
}

/// The document after the synchronous pass.
#[derive(Debug, Clone, Default)]
pub(crate) struct PdfPages {
    pub pages: Vec<PageExtraction>,
}

/// Characters that count as "this page has text". Markdown scaffolding
/// (`---`, `#`, `|`) and whitespace do not.
pub(crate) fn readable_chars(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphanumeric()).count()
}

/// Whether, under `options`, a page with this text layer should be OCR'd.
pub(crate) fn wants_ocr(options: &PdfProcessingOptions, native_text: &str) -> bool {
    match options.strategy {
        PdfStrategy::NativeOnly => false,
        PdfStrategy::OcrOnly | PdfStrategy::ForceOcr => true,
        // `min_chars_per_page` is 0 under `Default`; a page with ANY readable
        // character then counts as native. `auto()` sets 50, which is the
        // sensible production value — a stamp or a page number on a scan is
        // not a text layer.
        PdfStrategy::Auto => readable_chars(native_text) < options.min_chars_per_page.max(1),
    }
}

/// Synchronous half: read every page's text layer and pick OCR candidates.
///
/// Blocking parse work — callers run it under `spawn_blocking`.
#[cfg(feature = "pdf-markdown")]
pub(crate) fn extract_pages_from_path(
    file_path: &std::path::Path,
    options: &PdfProcessingOptions,
) -> Result<PdfPages, super::storage_processor::StoragePdfError> {
    use super::storage_processor::StoragePdfError;
    use pdf_oxide::converters::ConversionOptions;
    use pdf_oxide::extractors::images::ImageData;
    use pdf_oxide::PdfDocument;

    let mut doc =
        PdfDocument::open(file_path).map_err(|e| StoragePdfError::Processing(e.to_string()))?;

    let page_count = doc
        .page_count()
        .map_err(|e| StoragePdfError::Processing(e.to_string()))?;

    let conversion_options = ConversionOptions::default();
    let mut pages = Vec::with_capacity(page_count);
    let mut page_tree = scan::PageTree::default();

    for page_idx in 0..page_count {
        let text = if options.strategy.should_try_native() {
            doc.to_markdown(page_idx, &conversion_options)
                .map_err(|e| StoragePdfError::Processing(format!("Page {}: {}", page_idx, e)))?
        } else {
            String::new()
        };

        let mut page = PageExtraction {
            text,
            ocr_candidate: None,
        };

        if wants_ocr(options, &page.text) {
            // A failure to find the page's image is not a failure to extract:
            // the text layer already answered, and a malformed resource
            // dictionary should not turn a readable PDF into `failed`.
            match scan::largest_page_image(&mut doc, &mut page_tree, page_idx) {
                Ok(Some(img)) => {
                    let bytes = match img.data() {
                        ImageData::Jpeg(jpeg) => Ok(jpeg.clone()),
                        ImageData::Raw { .. } => img.to_png_bytes(),
                    };
                    match bytes {
                        Ok(b) => page.ocr_candidate = Some(b),
                        Err(e) => tracing::warn!(
                            page = page_idx,
                            error = %e,
                            "PDF page image could not be encoded for OCR; keeping text layer"
                        ),
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    page = page_idx,
                    error = %e,
                    "PDF page images could not be read for OCR; keeping text layer"
                ),
            }
        }

        pages.push(page);
    }

    Ok(PdfPages { pages })
}

/// Finding the scan image on a page.
///
/// `pdf_oxide::PdfDocument::extract_images` exists, but it cannot be used for
/// this: it decodes each image XObject's `/ColorSpace` **in place**, and a
/// colour space given as an indirect reference (`/ColorSpace 6 0 R`, which is
/// how iOS Quartz — every iPhone "scan" — and several scanners write it) makes
/// the decoder return `Invalid color space object: Reference(..)`, which
/// `extract_images` then swallows as "no images on this page". Measured on the
/// production file that opened this: four full-page JPEGs, reported as zero.
///
/// So this walks the page tree itself (with `/Resources` inheritance, PDF 32000
/// §7.7.3.4), resolves the colour-space reference, and only THEN hands the
/// XObject to pdf_oxide's decoder. It also descends one level into Form
/// XObjects, where some scanner drivers wrap the page image.
#[cfg(feature = "pdf-markdown")]
mod scan {
    use pdf_oxide::extractors::images::{extract_image_from_xobject, PdfImage};
    use pdf_oxide::object::{Object, ObjectRef};
    use pdf_oxide::{Error, PdfDocument, Result};
    use std::collections::HashMap;

    type Dict = HashMap<String, Object>;

    /// Leaf pages in document order, each with the `/Resources` that applies
    /// to it (its own, or the nearest ancestor's). Built lazily on first use.
    #[derive(Default)]
    pub(super) struct PageTree {
        pages: Option<Vec<Option<Object>>>,
    }

    fn parse_err(reason: &str) -> Error {
        Error::ParseError {
            offset: 0,
            reason: reason.to_string(),
        }
    }

    /// Load `obj` if it is a reference, else clone it.
    fn deref(doc: &mut PdfDocument, obj: &Object) -> Result<Object> {
        match obj.as_reference() {
            Some(r) => doc.load_object(r),
            None => Ok(obj.clone()),
        }
    }

    fn walk(
        doc: &mut PdfDocument,
        node_ref: &Object,
        inherited_resources: Option<Object>,
        out: &mut Vec<Option<Object>>,
        depth: usize,
    ) -> Result<()> {
        if depth > 64 {
            return Err(parse_err("page tree too deep"));
        }
        let node = deref(doc, node_ref)?;
        let dict = node
            .as_dict()
            .ok_or_else(|| parse_err("page tree node is not a dictionary"))?;
        let resources = dict.get("Resources").cloned().or(inherited_resources);
        let is_pages = dict.get("Type").and_then(Object::as_name) == Some("Pages")
            || dict.contains_key("Kids");
        if is_pages {
            let kids = dict
                .get("Kids")
                .map(|k| deref(doc, k))
                .transpose()?
                .and_then(|k| k.as_array().cloned())
                .unwrap_or_default();
            for kid in &kids {
                walk(doc, kid, resources.clone(), out, depth + 1)?;
            }
        } else {
            out.push(resources);
        }
        Ok(())
    }

    impl PageTree {
        fn resources_for(
            &mut self,
            doc: &mut PdfDocument,
            page_index: usize,
        ) -> Result<Option<Object>> {
            if self.pages.is_none() {
                let catalog = doc.catalog()?;
                let root = catalog
                    .as_dict()
                    .and_then(|c| c.get("Pages").cloned())
                    .ok_or_else(|| parse_err("catalog has no /Pages"))?;
                let mut out = Vec::new();
                walk(doc, &root, None, &mut out, 0)?;
                self.pages = Some(out);
            }
            Ok(self
                .pages
                .as_ref()
                .and_then(|p| p.get(page_index))
                .cloned()
                .flatten())
        }
    }

    /// Collect the image XObjects reachable from a resources dictionary,
    /// descending `form_depth` levels into Form XObjects.
    fn collect_images(
        doc: &mut PdfDocument,
        resources: &Object,
        form_depth: usize,
        out: &mut Vec<(Object, Option<ObjectRef>)>,
    ) -> Result<()> {
        let resources = deref(doc, resources)?;
        let Some(xobjects) = resources.as_dict().and_then(|r| r.get("XObject")) else {
            return Ok(());
        };
        let xobjects = deref(doc, xobjects)?;
        let Some(entries) = xobjects.as_dict() else {
            return Ok(());
        };
        let entries: Vec<Object> = entries.values().cloned().collect();
        for entry in entries {
            let obj_ref = entry.as_reference();
            let xobj = deref(doc, &entry)?;
            let Some(dict) = xobj.as_dict() else { continue };
            match dict.get("Subtype").and_then(Object::as_name) {
                Some("Image") => out.push((xobj, obj_ref)),
                Some("Form") if form_depth > 0 => {
                    if let Some(res) = dict.get("Resources").cloned() {
                        collect_images(doc, &res, form_depth - 1, out)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Return the image XObject with its `/ColorSpace` resolved to a direct
    /// object, which is the shape pdf_oxide's decoder can read.
    fn with_direct_color_space(doc: &mut PdfDocument, xobj: Object) -> Result<Object> {
        let Object::Stream { mut dict, data } = xobj else {
            return Ok(xobj);
        };
        if let Some(cs) = dict.get("ColorSpace").cloned() {
            if cs.as_reference().is_some() {
                let resolved = deref(doc, &cs)?;
                dict.insert("ColorSpace".to_string(), resolved);
            }
        }
        Ok(Object::Stream { dict, data })
    }

    fn area(dict: &Dict) -> u64 {
        let w = dict
            .get("Width")
            .and_then(Object::as_integer)
            .unwrap_or(0)
            .max(0) as u64;
        let h = dict
            .get("Height")
            .and_then(Object::as_integer)
            .unwrap_or(0)
            .max(0) as u64;
        w * h
    }

    /// The largest image on `page_index`, decoded — or `None` when the page
    /// has no image, or none that pdf_oxide can decode (each such failure is
    /// logged, not swallowed).
    pub(super) fn largest_page_image(
        doc: &mut PdfDocument,
        tree: &mut PageTree,
        page_index: usize,
    ) -> Result<Option<PdfImage>> {
        let Some(resources) = tree.resources_for(doc, page_index)? else {
            return Ok(None);
        };
        let mut found = Vec::new();
        collect_images(doc, &resources, 1, &mut found)?;

        // Largest first, so the page scan wins over a logo in the corner and
        // a decode failure on the scan is what gets reported.
        found.sort_by_key(|(x, _)| std::cmp::Reverse(x.as_dict().map(area).unwrap_or(0)));

        for (xobj, obj_ref) in found {
            let xobj = with_direct_color_space(doc, xobj)?;
            match extract_image_from_xobject(Some(&*doc), &xobj, obj_ref) {
                Ok(img) => return Ok(Some(img)),
                Err(e) => tracing::warn!(
                    page = page_index,
                    error = %e,
                    "PDF page image could not be decoded for OCR; trying the next one"
                ),
            }
        }
        Ok(None)
    }
}

/// Async half: OCR every candidate page and assemble the document.
///
/// See the module docs for the failure policy. `provider` is injected so the
/// policy is testable without Tesseract on the build machine.
pub(crate) async fn finish_with_ocr(
    pages: PdfPages,
    provider: &dyn OcrProvider,
    ocr_options: &OcrOptions,
) -> Result<PdfProcessedResult, OcrError> {
    let page_count = pages.pages.len();
    let mut texts: Vec<String> = Vec::with_capacity(page_count);
    let mut ocr_pages: Vec<usize> = Vec::new();
    let mut confidences: Vec<f32> = Vec::new();
    let mut first_error: Option<OcrError> = None;

    for (idx, page) in pages.pages.into_iter().enumerate() {
        let PageExtraction {
            text: native,
            ocr_candidate,
        } = page;

        let Some(image) = ocr_candidate else {
            texts.push(native);
            continue;
        };

        match provider.ocr_image_detailed(&image, ocr_options).await {
            // Keep whichever reading has more to say. Under `auto` the native
            // layer is (near-)empty by construction; under `force_ocr` a real
            // text layer still beats a worse OCR of the same page.
            Ok(result) if readable_chars(&result.text) > readable_chars(&native) => {
                ocr_pages.push(idx);
                if let Some(c) = result.confidence {
                    confidences.push(c);
                }
                texts.push(result.text);
            }
            Ok(_) => texts.push(native),
            Err(e) => {
                tracing::warn!(
                    page = idx,
                    provider = provider.name(),
                    error = %e,
                    "PDF page OCR failed; keeping text layer"
                );
                if first_error.is_none() {
                    first_error = Some(e);
                }
                texts.push(native);
            }
        }
    }

    let has_text = texts.iter().any(|t| readable_chars(t) > 0);
    if !has_text {
        if let Some(e) = first_error {
            // Nothing to show AND the reader that could have produced
            // something broke: surface it, so the asset is retried rather
            // than filed as a blank document.
            return Err(e);
        }
    }

    let text = texts.join("\n\n---\n\n");
    let method_used = if ocr_pages.is_empty() {
        ExtractionMethod::Native
    } else if ocr_pages.len() == page_count {
        ExtractionMethod::Ocr
    } else {
        ExtractionMethod::Hybrid
    };
    let ocr_confidence = if confidences.is_empty() {
        None
    } else {
        Some(confidences.iter().sum::<f32>() / confidences.len() as f32)
    };

    Ok(PdfProcessedResult {
        text,
        method_used,
        page_count,
        ocr_pages,
        ocr_confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::ocr::{OcrPageResult, OcrResult};
    use async_trait::async_trait;

    /// A provider that answers a fixed script, so the assembly policy is under
    /// test and Tesseract is not.
    struct Scripted(Vec<Result<(&'static str, Option<f32>), &'static str>>);

    #[async_trait]
    impl OcrProvider for Scripted {
        async fn ocr_image_detailed(
            &self,
            image_data: &[u8],
            _options: &OcrOptions,
        ) -> OcrResult<OcrPageResult> {
            // The candidate bytes carry the script index.
            let i = image_data[0] as usize;
            match &self.0[i] {
                Ok((text, conf)) => Ok(OcrPageResult {
                    text: text.to_string(),
                    confidence: *conf,
                    page_index: i,
                }),
                Err(msg) => Err(OcrError::ModelError(msg.to_string())),
            }
        }
        async fn is_available(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "scripted"
        }
    }

    fn page(text: &str, candidate: Option<u8>) -> PageExtraction {
        PageExtraction {
            text: text.to_string(),
            ocr_candidate: candidate.map(|i| vec![i]),
        }
    }

    #[test]
    fn auto_ocrs_only_pages_without_a_text_layer() {
        let opts = PdfProcessingOptions::auto();
        assert!(wants_ocr(&opts, ""));
        assert!(
            wants_ocr(&opts, "---\n\n# \n"),
            "markdown scaffolding is not text"
        );
        assert!(wants_ocr(&opts, "Page 3"), "a stamp is below the floor");
        assert!(!wants_ocr(&opts, &"word ".repeat(20)));
    }

    #[test]
    fn strategies_are_honoured() {
        let text = &"word ".repeat(20);
        let mut opts = PdfProcessingOptions::auto();
        opts.strategy = PdfStrategy::NativeOnly;
        assert!(!wants_ocr(&opts, ""));
        opts.strategy = PdfStrategy::ForceOcr;
        assert!(wants_ocr(&opts, text));
        opts.strategy = PdfStrategy::OcrOnly;
        assert!(wants_ocr(&opts, text));
    }

    #[test]
    fn default_options_treat_any_character_as_native() {
        // `Default` has min_chars_per_page = 0; the floor is clamped to 1 so
        // Auto still means "OCR the pages that have nothing".
        let opts = PdfProcessingOptions::default();
        assert!(wants_ocr(&opts, ""));
        assert!(!wants_ocr(&opts, "x"));
    }

    #[tokio::test]
    async fn a_scanned_document_is_ocr_on_every_page() {
        let pages = PdfPages {
            pages: vec![page("", Some(0)), page("", Some(1))],
        };
        let provider = Scripted(vec![
            Ok(("Seite eins", Some(90.0))),
            Ok(("Seite zwei", Some(80.0))),
        ]);
        let out = finish_with_ocr(pages, &provider, &OcrOptions::default())
            .await
            .unwrap();
        assert_eq!(out.method_used, ExtractionMethod::Ocr);
        assert_eq!(out.ocr_pages, vec![0, 1]);
        assert_eq!(out.text, "Seite eins\n\n---\n\nSeite zwei");
        assert_eq!(out.ocr_confidence, Some(85.0));
        assert_eq!(out.page_count, 2);
    }

    #[tokio::test]
    async fn a_mixed_document_is_hybrid_and_keeps_native_pages_verbatim() {
        let pages = PdfPages {
            pages: vec![page("Typed cover letter", None), page("", Some(0))],
        };
        let provider = Scripted(vec![Ok(("scanned attachment", None))]);
        let out = finish_with_ocr(pages, &provider, &OcrOptions::default())
            .await
            .unwrap();
        assert_eq!(out.method_used, ExtractionMethod::Hybrid);
        assert_eq!(out.ocr_pages, vec![1]);
        assert_eq!(out.text, "Typed cover letter\n\n---\n\nscanned attachment");
        assert_eq!(out.ocr_confidence, None);
    }

    #[tokio::test]
    async fn force_ocr_keeps_the_text_layer_when_it_reads_better() {
        let pages = PdfPages {
            pages: vec![page("A real text layer with many words in it", Some(0))],
        };
        let provider = Scripted(vec![Ok(("A r", Some(20.0)))]);
        let out = finish_with_ocr(pages, &provider, &OcrOptions::default())
            .await
            .unwrap();
        assert_eq!(out.method_used, ExtractionMethod::Native);
        assert!(out.ocr_pages.is_empty());
        assert_eq!(out.text, "A real text layer with many words in it");
    }

    #[tokio::test]
    async fn an_ocr_error_on_one_page_does_not_fail_a_document_that_has_text() {
        let pages = PdfPages {
            pages: vec![page("Typed page", None), page("", Some(0))],
        };
        let provider = Scripted(vec![Err("tesseract missing")]);
        let out = finish_with_ocr(pages, &provider, &OcrOptions::default())
            .await
            .unwrap();
        assert_eq!(out.method_used, ExtractionMethod::Native);
        assert_eq!(out.text, "Typed page\n\n---\n\n");
    }

    #[tokio::test]
    async fn an_ocr_error_that_leaves_nothing_is_a_failure_not_an_empty() {
        let pages = PdfPages {
            pages: vec![page("", Some(0))],
        };
        let provider = Scripted(vec![Err("tesseract missing")]);
        let err = finish_with_ocr(pages, &provider, &OcrOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, OcrError::ModelError(_)));
    }

    #[tokio::test]
    async fn a_genuinely_blank_scan_is_empty_not_failed() {
        let pages = PdfPages {
            pages: vec![page("", Some(0))],
        };
        let provider = Scripted(vec![Ok(("", None))]);
        let out = finish_with_ocr(pages, &provider, &OcrOptions::default())
            .await
            .unwrap();
        assert_eq!(out.method_used, ExtractionMethod::Native);
        assert_eq!(out.text, "");
    }
}

/// Manual end-to-end probe against a real PDF with the real OCR provider.
///
/// `RAISIN_PDF_PROBE=/path/to/scan.pdf cargo test -p raisin-ai --features
/// pdf-markdown,ocr --lib pdf::pages::probe -- --ignored --nocapture`
#[cfg(all(test, feature = "pdf-markdown"))]
mod probe {
    #[tokio::test]
    #[ignore = "needs RAISIN_PDF_PROBE and a local tesseract"]
    async fn probe_real_pdf() {
        let Ok(path) = std::env::var("RAISIN_PDF_PROBE") else {
            return;
        };
        let data = std::fs::read(&path).unwrap();
        let out = crate::pdf::PdfProcessor::new()
            .process(&data, &crate::pdf::PdfProcessingOptions::auto())
            .await
            .unwrap();
        println!(
            "method={:?} pages={} ocr_pages={:?} confidence={:?} chars={}\n{}",
            out.method_used,
            out.page_count,
            out.ocr_pages,
            out.ocr_confidence,
            out.text.len(),
            out.text.chars().take(600).collect::<String>()
        );
        assert!(!out.text.trim().is_empty());
    }
}

/// Regression: a scanned page whose image carries an INDIRECT colour space —
/// the shape iOS Quartz writes — must yield an OCR candidate. This is the
/// exact case pdf_oxide's own `extract_images` reports as "no images".
#[cfg(all(test, feature = "pdf-markdown"))]
mod scan_tests {
    use super::*;

    /// Build a one-page PDF: page → resources → one DCTDecode image whose
    /// `/ColorSpace` is a reference to an `[/ICCBased <stream>]` array, like a
    /// phone scan. Offsets are computed, so the xref is valid.
    fn quartz_style_scan_pdf(jpeg: &[u8], text_layer: Option<&str>) -> Vec<u8> {
        let content = match text_layer {
            Some(t) => {
                format!("BT /F1 12 Tf 20 700 Td ({t}) Tj ET\nq 500 0 0 700 0 0 cm /Im1 Do Q")
            }
            None => "q 500 0 0 700 0 0 cm /Im1 Do Q".to_string(),
        };
        let icc = b"\x00\x00\x00\x00fake-icc-profile";
        let objs: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources 4 0 R >>".to_vec(),
            // The page INHERITS /Resources from its parent — exercised on purpose.
            b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /MediaBox [0 0 500 700] >>".to_vec(),
            b"<< /XObject << /Im1 6 0 R >> /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>".to_vec(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content).into_bytes(),
            {
                let mut v = format!(
                    "<< /Type /XObject /Subtype /Image /Width 8 /Height 8 /BitsPerComponent 8 /ColorSpace 7 0 R /Filter /DCTDecode /Length {} >>\nstream\n",
                    jpeg.len()
                )
                .into_bytes();
                v.extend_from_slice(jpeg);
                v.extend_from_slice(b"\nendstream");
                v
            },
            b"[ /ICCBased 8 0 R ]".to_vec(),
            {
                let mut v = format!("<< /N 3 /Alternate /DeviceRGB /Length {} >>\nstream\n", icc.len()).into_bytes();
                v.extend_from_slice(icc);
                v.extend_from_slice(b"\nendstream");
                v
            },
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes(),
        );
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
                objs.len() + 1,
                xref
            )
            .as_bytes(),
        );
        out
    }

    fn tiny_jpeg() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([255, 255, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .unwrap();
        buf.into_inner()
    }

    fn write_temp(bytes: &[u8]) -> tempfile::NamedTempFile {
        let f = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
        std::fs::write(f.path(), bytes).unwrap();
        f
    }

    #[test]
    fn a_phone_scan_with_an_indirect_colour_space_yields_its_jpeg_for_ocr() {
        let jpeg = tiny_jpeg();
        let pdf = write_temp(&quartz_style_scan_pdf(&jpeg, None));
        let pages = extract_pages_from_path(pdf.path(), &PdfProcessingOptions::auto()).unwrap();
        assert_eq!(pages.pages.len(), 1);
        let candidate = pages.pages[0]
            .ocr_candidate
            .as_ref()
            .expect("the page image is the OCR candidate");
        assert_eq!(candidate, &jpeg, "JPEG passes through byte-for-byte");
    }

    #[test]
    fn native_only_never_looks_at_the_image() {
        let pdf = write_temp(&quartz_style_scan_pdf(&tiny_jpeg(), None));
        let mut opts = PdfProcessingOptions::auto();
        opts.strategy = PdfStrategy::NativeOnly;
        let pages = extract_pages_from_path(pdf.path(), &opts).unwrap();
        assert!(pages.pages[0].ocr_candidate.is_none());
    }

    #[test]
    fn force_ocr_takes_the_image_even_when_the_page_has_text() {
        let pdf = write_temp(&quartz_style_scan_pdf(
            &tiny_jpeg(),
            Some("A typed line that is long enough to count as a real text layer here"),
        ));
        let mut opts = PdfProcessingOptions::auto();
        opts.strategy = PdfStrategy::ForceOcr;
        let pages = extract_pages_from_path(pdf.path(), &opts).unwrap();
        assert!(pages.pages[0].ocr_candidate.is_some());
    }
}
