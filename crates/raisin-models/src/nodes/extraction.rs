// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The durable EXTRACTION ARTIFACT that a binary asset carries on its own node.
//!
//! # What it is for
//!
//! Text extraction is expensive, needs code that is not always present (the
//! media plugin lives on the Studio installation and nowhere else), and its
//! result is what an embedding is made from. Storing that result as node
//! PROPERTIES rather than in a side table is what makes the rest of the system
//! fall out for free:
//!
//! * **Publish carries it.** Studio publish is a SQL copy of
//!   `node_type, archetype, properties, updated_at` onto the `publish` branch.
//!   Anything in `properties` therefore rides along. The publish branch needs
//!   no extractor and no plugin; embedding there becomes a pure embed call over
//!   text that is already present.
//! * **Replication carries it.** An arriving replica already HAS the text, so
//!   it never re-extracts — the apply path runs no jobs.
//! * **Every index already understands it.** A property is what full-text,
//!   the property index and the vector plan all read. No new plumbing.
//!
//! # The most important field is [`EXTRACT_STATUS_PROP`]
//!
//! Before this artifact, a `.docx` uploaded to a server with no media plugin
//! became a node with no text and NO RECORD that anything had been skipped —
//! indistinguishable from an empty document, forever. There was nothing to
//! query, so when a plugin later gained the format there was no way to find the
//! assets it should now be run over.
//!
//! A durable `__extract_status = 'unsupported'`, with the reason in
//! [`EXTRACT_DETAIL_PROP`], turns that into one SQL statement:
//!
//! ```sql
//! SELECT path, file_type FROM 'assets'
//!  WHERE properties->>'__extract_status'::String = 'unsupported'
//! ```
//!
//! Every other property here exists to serve some other job. That one exists so
//! a gap is visible at all.
//!
//! # Why `__`, and how the properties are protected
//!
//! `__` is this codebase's established convention for ENGINE-OWNED properties
//! (see [`crate::nodes::is_engine_owned_property_key`]). The write paths refuse
//! to let an ordinary client move them: a client that loads a node, edits one
//! field and saves the whole map back would otherwise echo a stale artifact
//! over a fresh one — which for extraction means claiming a binary has been
//! read when it has not, and for the fingerprint means re-extracting forever or
//! never again. Only an engine actor
//! ([`crate::nodes::is_engine_write_actor`]) may set them.
//!
//! # Size: inline, capped, and why not a child node or a blob
//!
//! The text lives INLINE, truncated at [`MAX_INLINE_EXTRACT_BYTES`]. The two
//! alternatives were considered and rejected on the same fact — publish copies
//! PROPERTIES:
//!
//! * **A child node** would need to be selected for publish in its own right.
//!   Studio publish copies a chosen set of paths; an asset published without
//!   its extraction child arrives on the publish branch looking exactly like an
//!   asset that was never extracted, which is the failure this whole artifact
//!   exists to make impossible.
//! * **A reference into binary storage** does not travel at all, and would put
//!   an object-store read on the embedding path of a branch that was supposed
//!   to need nothing but an embed call.
//!
//! The cap is what keeps "every read of this node is now heavier" bounded. It
//! is a byte cap on a UTF-8 boundary, and [`EXTRACT_CHARS_PROP`] always records
//! the FULL length, so a truncated artifact announces itself rather than
//! looking like a short document.

use lazy_static::lazy_static;
use regex::Regex;
use std::borrow::Cow;
use std::collections::HashMap;

use crate::nodes::properties::PropertyValue;

/// The extracted text itself. Engine-owned; see the module docs.
pub const EXTRACTED_TEXT_PROP: &str = "__extracted_text";

/// Outcome of the extraction attempt — see [`ExtractStatus`].
pub const EXTRACT_STATUS_PROP: &str = "__extract_status";

/// Which extractor produced the text (`core-pdf`, `plugin-libreoffice`, …).
pub const EXTRACT_SOURCE_PROP: &str = "__extract_source";

/// Generation of the extraction pipeline that wrote this artifact.
pub const EXTRACT_VERSION_PROP: &str = "__extract_version";

/// Identifies the BINARY the artifact was produced from. The loop-breaker:
/// extraction writes a node property, and writing a node property emits
/// `node:updated`, which is the event that enqueues extraction. The enqueue
/// gate skips a node whose stamp already matches, so one upload extracts once
/// and REPLACING the file re-extracts.
pub const EXTRACT_FINGERPRINT_PROP: &str = "__extract_fingerprint";

/// Human-readable reason, for the statuses that have one: the mimetype nothing
/// could read, the plugin task that was not available, the error text.
pub const EXTRACT_DETAIL_PROP: &str = "__extract_detail";

/// FULL character length of the extracted text before any truncation. Larger
/// than the stored text means the artifact is truncated.
pub const EXTRACT_CHARS_PROP: &str = "__extract_chars";

/// How sure the extractor was, 0-100, for extractors that measure it.
///
/// Only OCR does today. It is what turns a threshold from a guess into a query —
/// *"show me everything we kept below 70"* is one SQL statement, and tuning the
/// filter without it means redeploying to learn anything.
///
/// **Absent means "not measured", never "measured as zero".** A PDF read as
/// native text has no score to give, so it stores none; a `< 70` sweep must not
/// collect every PDF on the way to finding the doubtful scans.
pub const EXTRACT_CONFIDENCE_PROP: &str = "__extract_confidence";

/// Generation of the extraction pipeline.
///
/// Bump this when a change makes text this pipeline already produced WRONG — a
/// better extractor, a fixed decoder. Every asset then re-extracts once, because
/// the enqueue gate compares the stored version as well as the fingerprint.
pub const EXTRACTION_ARTIFACT_VERSION: i64 = 1;

/// Largest text stored inline on the node, in bytes.
///
/// 256 KiB is roughly a 100-page PDF of prose. Beyond that the artifact is
/// truncated on a UTF-8 boundary and [`EXTRACT_CHARS_PROP`] records the real
/// length. The number is a judgement about how heavy a node read may become,
/// not a limit of the extractor.
pub const MAX_INLINE_EXTRACT_BYTES: usize = 256 * 1024;

/// What happened when this node's binary was offered to the extractors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractStatus {
    /// Text was produced and is in [`EXTRACTED_TEXT_PROP`].
    Ok,
    /// An extractor ran and the binary genuinely holds no text (an image-only
    /// PDF with OCR off, an empty document). Distinct from `Unsupported`: there
    /// is nothing here for a better extractor to find.
    Empty,
    /// NOTHING ON THIS SERVER CAN READ THESE BYTES. The durable record of a
    /// skip — the one status a backfill queries for when a plugin later gains
    /// the format. See the module docs.
    Unsupported,
    /// An extractor claimed the binary and then failed. Retryable; a backfill
    /// looking for newly-supported formats must NOT confuse it with
    /// `Unsupported`.
    Failed,
    /// TEXT IS EXPECTED, AND IT DOES NOT COME FROM THIS PROCESS.
    ///
    /// The routing table asked for a task that a loaded plugin services —
    /// `doc_to_markdown` via `media.doc.toMarkdown`, say — and the plugin lives
    /// above the job layer, so the asset job cannot perform it and must not
    /// pretend the format is unreadable.
    ///
    /// Distinct from `Unsupported` in the direction that matters: `Unsupported`
    /// means nothing here can EVER read these bytes, and a backfill sweeps it up
    /// when a plugin gains the format. `Delegated` means the plugin is already
    /// here and the work has been handed to it, so sweeping it into a
    /// re-extract would re-run a conversion that is either running or already
    /// done. It is also the query that finds a handover that never landed:
    ///
    /// ```sql
    /// SELECT path, __extract_detail FROM 'assets'
    ///  WHERE properties->>'__extract_status'::String = 'delegated'
    /// ```
    ///
    /// An asset stuck here is a Studio-side failure (the media job died, the
    /// trigger never fired), which is a different investigation from a missing
    /// plugin — and before this variant the two were the same row.
    Delegated,
}

impl ExtractStatus {
    /// The stored string. These values are queried from SQL and from an
    /// operator's console, so they are stable vocabulary, not a Debug rendering.
    pub fn as_str(self) -> &'static str {
        match self {
            ExtractStatus::Ok => "ok",
            ExtractStatus::Empty => "empty",
            ExtractStatus::Unsupported => "unsupported",
            ExtractStatus::Failed => "failed",
            ExtractStatus::Delegated => "delegated",
        }
    }

    /// Parse a stored value. Unknown text reads as `None` rather than as any
    /// status: a value this build does not understand was written by a newer
    /// one, and guessing would be worse than admitting it.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(ExtractStatus::Ok),
            "empty" => Some(ExtractStatus::Empty),
            "unsupported" => Some(ExtractStatus::Unsupported),
            "failed" => Some(ExtractStatus::Failed),
            "delegated" => Some(ExtractStatus::Delegated),
            _ => None,
        }
    }
}

/// One extraction outcome, ready to be stamped onto a node.
///
/// Built by the extraction step and applied with [`Self::apply`]. It exists as
/// a struct rather than as seven `properties.insert` calls at the call site
/// because a partially-written artifact is worse than none: a `__extract_status`
/// without a `__extract_fingerprint` re-extracts forever, and a fingerprint
/// without a status is the invisible skip all over again.
#[derive(Debug, Clone)]
pub struct ExtractionArtifact {
    /// Extraction outcome.
    pub status: ExtractStatus,
    /// Extracted text. Ignored unless `status` is [`ExtractStatus::Ok`].
    pub text: String,
    /// Which extractor produced it — `core-pdf`, `core-pdf-ocr`,
    /// `plugin-libreoffice`, `none`. Free-form on purpose: an out-of-tree
    /// plugin names itself, and a closed enum here would have to be widened in
    /// this crate for every plugin that ever ships.
    pub source: String,
    /// Reason, for the statuses that have one.
    pub detail: Option<String>,
    /// The binary this artifact was made from.
    pub fingerprint: String,
    /// Extractor confidence 0-100, for extractors that measure one.
    /// `None` is "not measured" and is written as an ABSENT property, not zero —
    /// see [`EXTRACT_CONFIDENCE_PROP`].
    pub confidence: Option<f32>,
}

impl ExtractionArtifact {
    /// An artifact recording that nothing here can read these bytes.
    pub fn unsupported(fingerprint: String, detail: impl Into<String>) -> Self {
        Self {
            status: ExtractStatus::Unsupported,
            text: String::new(),
            source: "none".to_string(),
            detail: Some(detail.into()),
            fingerprint,
            confidence: None,
        }
    }

    /// An artifact recording that an extractor claimed the binary and failed.
    pub fn failed(
        fingerprint: String,
        source: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            status: ExtractStatus::Failed,
            text: String::new(),
            source: source.into(),
            detail: Some(detail.into()),
            fingerprint,
            confidence: None,
        }
    }

    /// An artifact recording that the text will be produced ABOVE this layer.
    ///
    /// `awaiting` names the tasks that were handed off, so the stored detail
    /// reads `awaiting doc_to_markdown (media.doc.toMarkdown)` rather than a
    /// bare status somebody has to correlate with a log line.
    ///
    /// It stamps the fingerprint like every other artifact, which is what stops
    /// the handover from re-enqueueing itself: the enqueue gate skips a node
    /// whose stamp matches, so the asset job runs ONCE and the delegated task
    /// is dispatched once. Replacing the file changes the fingerprint and hands
    /// off again.
    pub fn delegated(fingerprint: String, awaiting: &[String]) -> Self {
        Self {
            status: ExtractStatus::Delegated,
            text: String::new(),
            source: "delegated".to_string(),
            detail: Some(if awaiting.is_empty() {
                "awaiting a task performed above the job layer".to_string()
            } else {
                format!("awaiting {}", awaiting.join(", "))
            }),
            fingerprint,
            confidence: None,
        }
    }

    /// An artifact carrying extracted text — or recording that the binary was
    /// read and held none.
    ///
    /// The `Ok`/`Empty` decision is made HERE rather than at the call site, so
    /// an extractor that returns `Some("")` cannot accidentally publish an
    /// empty document as a successful extraction.
    pub fn extracted(fingerprint: String, source: impl Into<String>, text: String) -> Self {
        let status = if text.trim().is_empty() {
            ExtractStatus::Empty
        } else {
            ExtractStatus::Ok
        };
        Self {
            status,
            text,
            source: source.into(),
            detail: None,
            fingerprint,
            confidence: None,
        }
    }

    /// Record how sure the extractor was, and why that mattered.
    ///
    /// Both travel together on purpose. A bare 31 says nothing to whoever finds
    /// it later; `detail` is where the artifact explains what the number caused
    /// — "OCR kept no word above 50 (best 31)" — so the record is legible
    /// without the reader knowing the pipeline.
    pub fn with_confidence(mut self, confidence: Option<f32>, detail: Option<String>) -> Self {
        self.confidence = confidence;
        if let Some(d) = detail {
            self.detail = Some(d);
        }
        self
    }

    /// Write the artifact onto a property map, replacing any previous one.
    ///
    /// `store_text` is the operator's `store_extracted_text` setting. When it
    /// is off the TEXT is dropped and everything else is still written: the
    /// status record is bookkeeping about the binary, not content, and losing
    /// it is what made a skip invisible in the first place.
    pub fn apply(&self, properties: &mut HashMap<String, PropertyValue>, store_text: bool) {
        // Strip embedded binaries BEFORE measuring or storing. An extractor that
        // renders a PDF to markdown inlines every figure as a `data:` URI, and
        // the payload is the overwhelming majority of the result: one 40-page
        // deck arrived as 208,492 characters of which the prose was a fraction.
        //
        // That is not merely wasteful. It eats the inline budget, so real text
        // is truncated away in favour of image bytes; it destroys chunking,
        // because a base64 blob is a single unsplittable "word" and the
        // recursive splitter backs off around it and emits fragments like `---`
        // and `![` as chunks of their own; and what does get embedded is a
        // vector of base64 noise, which matches nothing in particular.
        let text = strip_data_uris(&self.text);
        let full_chars = text.chars().count() as i64;

        properties.insert(
            EXTRACT_STATUS_PROP.to_string(),
            PropertyValue::String(self.status.as_str().to_string()),
        );
        properties.insert(
            EXTRACT_SOURCE_PROP.to_string(),
            PropertyValue::String(self.source.clone()),
        );
        properties.insert(
            EXTRACT_VERSION_PROP.to_string(),
            PropertyValue::Integer(EXTRACTION_ARTIFACT_VERSION),
        );
        properties.insert(
            EXTRACT_FINGERPRINT_PROP.to_string(),
            PropertyValue::String(self.fingerprint.clone()),
        );
        properties.insert(
            EXTRACT_CHARS_PROP.to_string(),
            PropertyValue::Integer(full_chars),
        );

        // Absent when not measured, and REMOVED rather than left behind — a
        // stale 95 from a previous OCR sitting beside a fresh native-text
        // extraction would be read as this run's score.
        match self.confidence {
            Some(c) => {
                properties.insert(
                    EXTRACT_CONFIDENCE_PROP.to_string(),
                    PropertyValue::Float(c as f64),
                );
            }
            None => {
                properties.remove(EXTRACT_CONFIDENCE_PROP);
            }
        }

        match &self.detail {
            Some(d) => {
                properties.insert(
                    EXTRACT_DETAIL_PROP.to_string(),
                    PropertyValue::String(d.clone()),
                );
            }
            // Remove rather than leave: a stale "no extractor handles
            // application/msword" next to a fresh `ok` reads as a contradiction
            // and someone will believe the wrong half.
            None => {
                properties.remove(EXTRACT_DETAIL_PROP);
            }
        }

        if store_text && self.status == ExtractStatus::Ok {
            properties.insert(
                EXTRACTED_TEXT_PROP.to_string(),
                PropertyValue::String(truncate_on_char_boundary(&text, MAX_INLINE_EXTRACT_BYTES)),
            );
        } else {
            properties.remove(EXTRACTED_TEXT_PROP);
        }
    }
}

/// Replace the payload of every `data:` URI with nothing, keeping surrounding
/// text intact.
///
/// Matches `data:<mime>;base64,<payload>` and drops the whole run, so markdown
/// like `![Image 1 from page 1](data:image/jpeg;base64,/9j/4AAQ...)` becomes
/// `![Image 1 from page 1]()` — the alt text survives, because "Image 1 from
/// page 1" is the only part of a figure that carries meaning for retrieval.
///
/// The payload class is deliberately standard base64 only (`A-Za-z0-9+/=`) and
/// excludes whitespace: a real data URI never contains any, and admitting it
/// would let the match run past the URI and swallow the prose after it.
pub fn strip_data_uris(text: &str) -> Cow<'_, str> {
    lazy_static! {
        static ref DATA_URI: Regex =
            Regex::new(r"data:[^;,)\s]*;base64,[A-Za-z0-9+/=]+").expect("valid data-URI regex");
    }
    DATA_URI.replace_all(text, "")
}

/// Every property key the extraction artifact owns.
///
/// The shield reads this, so a key added to the artifact and forgotten here is
/// a key a client can forge — which for `__extract_status` means claiming a
/// binary was read when it was not, and for `__extract_fingerprint` means
/// switching extraction off for that asset permanently.
pub const EXTRACTION_ARTIFACT_KEYS: &[&str] = &[
    EXTRACTED_TEXT_PROP,
    EXTRACT_STATUS_PROP,
    EXTRACT_SOURCE_PROP,
    EXTRACT_VERSION_PROP,
    EXTRACT_FINGERPRINT_PROP,
    EXTRACT_DETAIL_PROP,
    EXTRACT_CHARS_PROP,
    EXTRACT_CONFIDENCE_PROP,
];

/// Is `key` part of the extraction artifact?
pub fn is_extraction_artifact_key(key: &str) -> bool {
    EXTRACTION_ARTIFACT_KEYS.contains(&key)
}

/// The stored extraction status of a node, if it has an artifact.
pub fn extract_status(properties: &HashMap<String, PropertyValue>) -> Option<ExtractStatus> {
    match properties.get(EXTRACT_STATUS_PROP) {
        Some(PropertyValue::String(s)) => ExtractStatus::parse(s),
        _ => None,
    }
}

/// The stored extraction fingerprint, if any.
pub fn extract_fingerprint(properties: &HashMap<String, PropertyValue>) -> Option<&str> {
    match properties.get(EXTRACT_FINGERPRINT_PROP) {
        Some(PropertyValue::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// The stored pipeline version, if any.
pub fn extract_version(properties: &HashMap<String, PropertyValue>) -> Option<i64> {
    match properties.get(EXTRACT_VERSION_PROP) {
        Some(PropertyValue::Integer(n)) => Some(*n),
        _ => None,
    }
}

/// The extracted text of a node whose extraction SUCCEEDED.
///
/// Returns `None` for every other status, so a caller cannot accidentally embed
/// the empty string of an `unsupported` node and record it as a real vector.
pub fn extracted_text(properties: &HashMap<String, PropertyValue>) -> Option<&str> {
    if extract_status(properties) != Some(ExtractStatus::Ok) {
        return None;
    }
    match properties.get(EXTRACTED_TEXT_PROP) {
        Some(PropertyValue::String(s)) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    }
}

/// Truncate to at most `max_bytes`, never splitting a UTF-8 code point.
fn truncate_on_char_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp() -> String {
        "v1:hash|key|10".to_string()
    }

    #[test]
    fn an_unsupported_binary_leaves_a_durable_queryable_record() {
        // THE property of this design: a skip must be findable later.
        let mut props = HashMap::new();
        ExtractionArtifact::unsupported(fp(), "no extractor for application/msword")
            .apply(&mut props, true);

        assert_eq!(extract_status(&props), Some(ExtractStatus::Unsupported));
        assert_eq!(extract_fingerprint(&props), Some(fp().as_str()));
        assert_eq!(extract_version(&props), Some(EXTRACTION_ARTIFACT_VERSION));
        assert!(matches!(
            props.get(EXTRACT_DETAIL_PROP),
            Some(PropertyValue::String(d)) if d.contains("msword")
        ));
        // No text, and no pretence of any.
        assert_eq!(extracted_text(&props), None);
        assert!(!props.contains_key(EXTRACTED_TEXT_PROP));
    }

    #[test]
    fn the_status_record_survives_store_extracted_text_being_off() {
        let mut props = HashMap::new();
        ExtractionArtifact::extracted(fp(), "core-pdf", "hello".into()).apply(&mut props, false);
        assert_eq!(extract_status(&props), Some(ExtractStatus::Ok));
        assert!(!props.contains_key(EXTRACTED_TEXT_PROP));
        // The FULL length is still recorded, so "nothing stored" is not
        // confusable with "nothing there".
        assert_eq!(
            props.get(EXTRACT_CHARS_PROP),
            Some(&PropertyValue::Integer(5))
        );
    }

    #[test]
    fn empty_output_is_empty_not_ok() {
        let mut props = HashMap::new();
        ExtractionArtifact::extracted(fp(), "core-pdf", "   \n ".into()).apply(&mut props, true);
        assert_eq!(extract_status(&props), Some(ExtractStatus::Empty));
        assert_eq!(extracted_text(&props), None);
    }

    #[test]
    fn a_fresh_artifact_clears_the_previous_detail() {
        let mut props = HashMap::new();
        ExtractionArtifact::unsupported(fp(), "no extractor").apply(&mut props, true);
        assert!(props.contains_key(EXTRACT_DETAIL_PROP));

        ExtractionArtifact::extracted(fp(), "plugin-libreoffice", "now readable".into())
            .apply(&mut props, true);
        assert_eq!(extract_status(&props), Some(ExtractStatus::Ok));
        assert!(
            !props.contains_key(EXTRACT_DETAIL_PROP),
            "a stale reason next to a fresh success is a contradiction someone will believe"
        );
    }

    #[test]
    fn truncation_announces_itself_and_never_splits_a_code_point() {
        let text: String = "é".repeat(MAX_INLINE_EXTRACT_BYTES); // 2 bytes each
        let mut props = HashMap::new();
        ExtractionArtifact::extracted(fp(), "core-pdf", text.clone()).apply(&mut props, true);

        let stored = extracted_text(&props).unwrap();
        assert!(stored.len() <= MAX_INLINE_EXTRACT_BYTES);
        assert!(stored.chars().all(|c| c == 'é'), "split a code point");
        assert_eq!(
            props.get(EXTRACT_CHARS_PROP),
            Some(&PropertyValue::Integer(MAX_INLINE_EXTRACT_BYTES as i64)),
            "the recorded length is the REAL one, so truncation is detectable"
        );
    }

    #[test]
    fn every_artifact_property_is_engine_owned() {
        // If one of these ever loses its `__`, the shield stops covering it and
        // a client can forge an extraction result.
        for p in EXTRACTION_ARTIFACT_KEYS {
            assert!(
                crate::nodes::is_engine_owned_property_key(p),
                "{p} is not engine-owned"
            );
            assert!(
                crate::nodes::is_shielded_property_key(p, &HashMap::new()),
                "{p} is writable by an ordinary client"
            );
        }
        // And the list is the WHOLE artifact: anything `apply` writes must be
        // in it, or the shield has a hole shaped exactly like that property.
        let mut props = HashMap::new();
        ExtractionArtifact::unsupported(fp(), "x").apply(&mut props, true);
        ExtractionArtifact::extracted(fp(), "core-pdf", "text".into()).apply(&mut props, true);
        // Every OPTIONAL property too, or a key only some artifacts write can
        // slip past this check — `__extract_confidence` is written by exactly
        // one path and would otherwise never appear here.
        ExtractionArtifact::extracted(fp(), "core-image-ocr", "text".into())
            .with_confidence(Some(91.0), Some("why".into()))
            .apply(&mut props, true);
        for key in props.keys() {
            assert!(
                is_extraction_artifact_key(key),
                "{key} is not in the shield list"
            );
        }
    }

    /// Not measured and measured-as-terrible are different facts, and only one
    /// of them should answer a `__extract_confidence < 70` sweep.
    #[test]
    fn an_unmeasured_confidence_is_absent_not_zero() {
        let mut props = HashMap::new();
        ExtractionArtifact::extracted(fp(), "core-pdf", "native text".into())
            .apply(&mut props, true);
        assert!(
            !props.contains_key(EXTRACT_CONFIDENCE_PROP),
            "a PDF read as native text has no score to report"
        );

        // And a stale score never outlives the run that produced it.
        ExtractionArtifact::extracted(fp(), "core-image-ocr", "t".into())
            .with_confidence(Some(31.0), None)
            .apply(&mut props, true);
        assert_eq!(
            props.get(EXTRACT_CONFIDENCE_PROP),
            Some(&PropertyValue::Float(31.0))
        );
        ExtractionArtifact::extracted(fp(), "core-pdf", "t".into()).apply(&mut props, true);
        assert!(
            !props.contains_key(EXTRACT_CONFIDENCE_PROP),
            "a re-extraction that measures nothing must clear the old score, \
             or the 31 is read as this run's"
        );
    }
}

#[cfg(test)]
mod data_uri_tests {
    use super::*;

    /// The shape that actually arrived from production: a PDF rendered to
    /// markdown with every figure inlined as base64.
    #[test]
    fn a_figure_keeps_its_caption_and_loses_its_payload() {
        let text = "# Wunderframe\n\n\
                    ![Image 1 from page 1](data:image/jpeg;base64,/9j/4AAQSkZJRgABAQEAYABgAAD)\n\n\
                    Revolutionizing event ticketing.";
        let out = strip_data_uris(text);
        assert_eq!(
            out,
            "# Wunderframe\n\n![Image 1 from page 1]()\n\nRevolutionizing event ticketing."
        );
    }

    #[test]
    fn every_occurrence_goes_not_just_the_first() {
        let text = "a(data:image/png;base64,AAAA)b(data:image/gif;base64,BBBB)c";
        assert_eq!(strip_data_uris(text), "a()b()c");
    }

    /// The hazard in the payload character class: admitting whitespace would let
    /// the match run past the URI and eat the document.
    #[test]
    fn prose_after_a_payload_survives() {
        let text = "![x](data:image/png;base64,AAAA) The quick brown fox jumps over it.";
        assert_eq!(
            strip_data_uris(text),
            "![x]() The quick brown fox jumps over it."
        );
    }

    #[test]
    fn text_without_data_uris_is_untouched_and_not_reallocated() {
        let text = "Ordinary prose mentioning base64 and data: in passing.";
        let out = strip_data_uris(text);
        assert_eq!(out, text);
        assert!(matches!(out, Cow::Borrowed(_)), "no needless allocation");
    }

    /// The end-to-end property: what gets STORED, and what `__extract_chars`
    /// reports, are both the stripped text — otherwise the character count
    /// describes bytes nobody can search and the inline budget is spent on
    /// image data instead of prose.
    #[test]
    fn the_stored_artifact_and_its_char_count_exclude_the_payload() {
        let payload = "A".repeat(50_000);
        let artifact = ExtractionArtifact {
            text: format!("Real prose here.\n\n![fig](data:image/png;base64,{payload})"),
            status: ExtractStatus::Ok,
            source: "core-pdf".to_string(),
            fingerprint: "fp".to_string(),
            detail: None,
            confidence: None,
        };

        let mut props = HashMap::new();
        artifact.apply(&mut props, true);

        let stored = match props.get(EXTRACTED_TEXT_PROP) {
            Some(PropertyValue::String(s)) => s.clone(),
            other => panic!("expected stored text, got {other:?}"),
        };
        assert!(
            !stored.contains("AAAA"),
            "the base64 payload must not be stored"
        );
        assert!(
            stored.contains("Real prose here."),
            "the prose must survive"
        );

        let chars = match props.get(EXTRACT_CHARS_PROP) {
            Some(PropertyValue::Integer(n)) => *n,
            other => panic!("expected a char count, got {other:?}"),
        };
        assert_eq!(
            chars,
            stored.chars().count() as i64,
            "__extract_chars must describe the text that was actually stored"
        );
        assert!(
            chars < 1_000,
            "50 KB of base64 must not be counted: {chars}"
        );
    }
}
