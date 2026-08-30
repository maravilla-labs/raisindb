//! Asset processing job handler for PDF text extraction and image processing.
//!
//! **DEPRECATED**: This automatic processing handler is deprecated in favor of
//! user-defined trigger functions that use the Resource API and `raisin.ai.*` SDK.
//!
//! See `examples/launchpad/package/content/functions/lib/launchpad/process-asset/`
//! for the recommended approach.
//!
//! # Submodules
//!
//! - `types` - Result types and callback definitions
//! - `handler` - AssetProcessingHandler struct and model management
//! - `helpers` - Helper functions for extracting node properties

// This module implements the deprecated AssetProcessingHandler
#![allow(deprecated)]

mod handler;
pub(crate) mod helpers;
mod types;

#[cfg(test)]
mod tests;

pub use handler::AssetProcessingHandler;
pub use types::{AssetProcessingResult, BinaryRetrievalCallback};

// The extraction artifact — the text, the outcome, the provenance and the
// fingerprint that stops it being extracted forever — is defined ONCE in
// `raisin_models::nodes::extraction` and re-exported here for the two readers
// in this crate: the JOB that writes it and the EVENT HANDLER that reads the
// fingerprint to decide whether to enqueue the job at all.
//
// It moved out of this module because the vocabulary is no longer only this
// job's business. The properties are `__`-prefixed engine-owned keys, so the
// write-path shield in `raisin-core` has to know them; and they are what
// PUBLISH carries to another branch, so the embedding job on that branch has to
// read them. Three crates naming the same property is exactly the drift that a
// single definition prevents.
pub(crate) use raisin_models::nodes::extraction::{
    ExtractStatus, ExtractionArtifact, EXTRACTED_TEXT_PROP, EXTRACTION_ARTIFACT_VERSION,
    EXTRACT_FINGERPRINT_PROP as EXTRACTION_FINGERPRINT_PROP, EXTRACT_VERSION_PROP,
};
