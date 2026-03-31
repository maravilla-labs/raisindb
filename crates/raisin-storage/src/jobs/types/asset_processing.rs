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
    /// Extract text from PDF files
    #[serde(default = "default_true")]
    pub extract_pdf_text: bool,
    /// Generate image embeddings using CLIP
    #[serde(default)]
    pub generate_image_embedding: bool,
    /// Generate image captions using BLIP
    #[serde(default)]
    pub generate_image_caption: bool,
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
    /// Model ID for image captioning
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_model: Option<String>,
    /// Model ID for image embeddings
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    /// Custom prompt for alt-text generation (Moondream only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_text_prompt: Option<String>,
    /// Custom prompt for description generation (Moondream only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_prompt: Option<String>,
    /// Generate image keywords (Moondream only)
    #[serde(default)]
    pub generate_keywords: bool,
    /// Custom prompt for keyword extraction (Moondream only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords_prompt: Option<String>,
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
