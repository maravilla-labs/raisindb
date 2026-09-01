// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Asset processing options and PDF extraction strategy

use serde::{Deserialize, Serialize};

/// Options for asset processing jobs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AssetProcessingOptions {
    /// The task slugs this job was asked to run, already filtered to the ones
    /// this process can actually perform.
    ///
    /// Resolved at enqueue time by `raisin_ai::plan_tasks` from the matched
    /// processing rule, so it — not the booleans below — is the authoritative
    /// record of what the routing table decided. The booleans are kept as the
    /// dispatch shape the handler already reads and as the wire-compatible
    /// spelling for jobs persisted before this field existed; they are a
    /// PROJECTION of this list and must never be set independently of it.
    ///
    /// Empty on a job enqueued by an older binary, which is why the handler
    /// still dispatches on the booleans rather than on this.
    #[serde(default)]
    pub tasks: Vec<String>,

    /// Extract text from PDF files.
    ///
    /// "The routing table asked for text AND this process can read these
    /// bytes." See [`Self::extract_text_requested`] for the other half.
    #[serde(default = "default_true")]
    pub extract_pdf_text: bool,

    /// The routing table asked for text from this binary, WHETHER OR NOT
    /// anything here can read it.
    ///
    /// # Why this is separate from `extract_pdf_text`
    ///
    /// It used to be one flag, computed as "wanted AND supported". That single
    /// conjunction is what made a skip invisible: a `.docx` uploaded with no
    /// media plugin loaded produced `extract_pdf_text = false`, which the
    /// handler read as "nobody asked", so it wrote nothing at all — a node with
    /// no text and no record that anything had been skipped, indistinguishable
    /// from an empty document forever, and impossible to find later when a
    /// plugin gains the format.
    ///
    /// Splitting the two lets the handler write the durable
    /// `__extract_status = 'unsupported'` artifact for exactly the assets that
    /// were asked for and could not be read. `wanted` is the question the
    /// routing table answers; `supported` is the question this binary answers;
    /// conflating them loses the difference between "not asked" and "asked and
    /// skipped".
    ///
    /// Defaults to false so a job persisted by an older binary keeps its old
    /// behaviour (no artifact) rather than mass-labelling a backlog
    /// `unsupported` on the strength of a field it never set.
    #[serde(default)]
    pub extract_text_requested: bool,

    /// The routing table asked for text and a task that a LOADED PLUGIN
    /// services was handed the job.
    ///
    /// The third answer that `extract_text_requested` alone could not give.
    /// That flag means "asked for, from this binary"; a delegated task is asked
    /// for and answerable, just not here — a `.docx` on a Studio server with the
    /// media plugin loaded. Recording it as unsupported would claim the format
    /// is unreadable while the converter that reads it is running, and would
    /// put the asset in the backfill sweep that looks for formats a plugin has
    /// newly gained.
    ///
    /// Defaults to false, so a job persisted by an older binary keeps the
    /// two-state behaviour it was enqueued under.
    #[serde(default)]
    pub text_delegated: bool,

    /// Task slugs the routing table asked for that CANNOT run in this process —
    /// typically a media-plugin transform on a server with no such plugin.
    ///
    /// Carried into the job rather than only logged at enqueue time, so the
    /// artifact can name the reason an asset was skipped (`doc_to_markdown
    /// needs media.doc.toMarkdown`) instead of recording a bare "unsupported"
    /// that an operator has to correlate with a log line from hours earlier.
    #[serde(default)]
    pub blocked_tasks: Vec<String>,

    /// Task slugs that WILL run, but not in this process — each rendered
    /// `slug (plugin.method)`.
    ///
    /// Carried so the durable artifact can name what it is waiting for. An
    /// asset parked on `__extract_status = 'delegated'` is then a specific
    /// question ("the media job for this never came back") rather than the same
    /// row as an asset nothing can read.
    #[serde(default)]
    pub delegated_tasks: Vec<String>,
    /// Generate image embeddings using CLIP
    #[serde(default)]
    pub generate_image_embedding: bool,
    // NO captioning fields. `generate_image_caption`, `caption_model`,
    // `alt_text_prompt`, `description_prompt`, `generate_keywords` and
    // `keywords_prompt` used to be serialized into every asset job and read by
    // nothing — the handler answered all six with one warn line. They are gone
    // from the rule settings, this payload, the handler and the console
    // together, because removing any one of those alone leaves a state that is
    // invisible from both ends: no UI, but still on the wire.
    //
    // Captioning is a product-layer feature (a trigger function picking its own
    // model and prompt). Core's half is that `raisin:Asset`'s `description` /
    // `alt_text` / `keywords` are Vector-indexed, so whatever such a trigger
    // writes is semantically searchable through the ordinary write path.
    //
    // Serde ignores unknown fields, so a job persisted by an older binary still
    // deserializes; the dead keys are dropped.
    /// PDF extraction strategy (auto, native, ocr)
    #[serde(default)]
    pub pdf_strategy: PdfExtractionStrategy,
    /// Store extracted text in node properties
    #[serde(default = "default_true")]
    pub store_extracted_text: bool,
    /// Trigger embedding generation after text extraction
    #[serde(default)]
    pub trigger_embedding: bool,
    /// Content hash of the file (for deduplication)
    /// When set, prevents re-processing the same binary content
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Model ID for image embeddings
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Strategy for PDF text extraction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PdfExtractionStrategy {
    /// Automatically detect: try native first, fall back to OCR
    #[default]
    Auto,
    /// Only use native text extraction
    NativeOnly,
    /// Only use OCR
    OcrOnly,
    /// Force OCR even if PDF has native text
    ForceOcr,
}
