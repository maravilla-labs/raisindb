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

pub mod asset;
pub mod audit_log;
pub mod extraction;
pub mod integrations;
pub mod types;
pub mod version;

// Bring Node and DeepNode types into this namespace directly from node/core/
#[path = "node/core/mod.rs"]
mod node_core;
pub use node_core::*;
#[path = "node/graph.rs"]
mod graph;
pub use graph::*;

// Re-export types for easier access
pub use types::*;
// Re-export version for easier access
pub use version::*;

// Re-export properties module
pub mod properties;

// Re-export audit_log module
pub use audit_log::*;

// Re-export the asset accessors. ONE implementation of "what is this binary",
// read by the enqueue gate, the extraction job and the delegated writeback —
// see `asset` for why a second one is a re-extraction loop.
pub use asset::{asset_content_hash, asset_fingerprint, asset_mime_type, asset_storage_key};

// Re-export the extraction artifact vocabulary
pub use extraction::{
    extract_fingerprint, extract_status, extract_version, extracted_text,
    is_extraction_artifact_key, ExtractStatus, ExtractionArtifact, EXTRACTED_TEXT_PROP,
    EXTRACTION_ARTIFACT_KEYS, EXTRACTION_ARTIFACT_VERSION, EXTRACT_CHARS_PROP,
    EXTRACT_CONFIDENCE_PROP, EXTRACT_DETAIL_PROP, EXTRACT_FINGERPRINT_PROP, EXTRACT_SOURCE_PROP,
    EXTRACT_STATUS_PROP, EXTRACT_VERSION_PROP, MAX_INLINE_EXTRACT_BYTES,
};
