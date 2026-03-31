//! HuggingFace model management for local AI models.
//!
//! This module provides functionality to:
//! - Track available models from HuggingFace Hub
//! - Download models for local inference
//! - Manage cached models and disk usage
//!
//! # Example
//!
//! ```no_run
//! use raisin_ai::huggingface::{ModelRegistry, ModelInfo, ModelType};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let registry = ModelRegistry::new()?;
//!
//! // List available models
//! let models = registry.list_models().await;
//! for model in &models {
//!     println!("{}: {:?} - downloaded: {}", model.model_id, model.model_type, model.is_downloaded());
//! }
//!
//! // Download a model (None for no progress callback)
//! registry.download_model("openai/clip-vit-base-patch32", None).await?;
//! # Ok(())
//! # }
//! ```

mod registry;
mod types;

pub use registry::*;
pub use types::*;
