// SPDX-License-Identifier: BSL-1.1

//! HTTP handlers for tenant AI configuration.
//!
//! Provides REST API endpoints for:
//! - Getting and setting full AI configuration
//! - Listing configured providers
//! - Testing provider connections
//! - Discovering available models dynamically
//! - Querying per-model capabilities
//! - Managing HuggingFace local models

mod capabilities;
mod config;
mod huggingface;
mod models;
mod test_connection;
pub mod types;

// Re-export all handler functions to preserve `crate::handlers::ai::*` paths
pub use capabilities::get_model_capabilities;
pub use config::{get_ai_config, list_providers, set_ai_config};
pub use huggingface::{delete_huggingface_model, get_huggingface_model, list_huggingface_models};
#[cfg(feature = "storage-rocksdb")]
pub use huggingface::{download_huggingface_model, list_local_caption_models};
pub use models::{list_all_models, list_models_by_use_case};
pub use test_connection::test_provider_connection;

// Re-export all types for backward compatibility
pub use types::*;
