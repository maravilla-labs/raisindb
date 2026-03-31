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
//! - `captioner` - Cached captioner supporting BLIP and Moondream models
//! - `handler` - AssetProcessingHandler struct and model management
//! - `helpers` - Helper functions for extracting node properties

// This module implements the deprecated AssetProcessingHandler
#![allow(deprecated)]

mod captioner;
mod handler;
mod helpers;
mod types;

#[cfg(test)]
mod tests;

pub use handler::AssetProcessingHandler;
pub use types::{AssetProcessingResult, BinaryRetrievalCallback};
