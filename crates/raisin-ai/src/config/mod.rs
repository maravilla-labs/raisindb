//! Configuration models for tenant AI settings and providers.
//!
//! This module defines the configuration structure for managing AI/LLM providers
//! at the tenant level. Each tenant can have multiple providers configured with
//! different models and use cases.

mod chunking;
mod embedder;
mod processing;
mod provider;
#[cfg(test)]
mod tests;

pub use chunking::{ChunkingConfig, OverlapConfig, SplitterType};
pub use embedder::{EmbedderId, EmbeddingKind, EmbeddingSettings};
pub use processing::{ProcessingDefaults, DEFAULT_CAPTION_MODEL, DEFAULT_IMAGE_EMBEDDING_MODEL};
pub use provider::{AIModelConfig, AIProvider, AIProviderConfig, AIUseCase};

use serde::{Deserialize, Serialize};

/// Tenant-level AI configuration.
///
/// Contains all AI provider configurations for a specific tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantAIConfig {
    /// Unique identifier for the tenant
    pub tenant_id: String,
    /// List of configured AI providers
    pub providers: Vec<AIProviderConfig>,
    /// Embedding settings for this tenant
    #[serde(default)]
    pub embedding_settings: Option<EmbeddingSettings>,
    /// Default settings for asset processing (image captioning, embeddings)
    #[serde(default)]
    pub processing_defaults: Option<ProcessingDefaults>,
}

impl TenantAIConfig {
    /// Creates a new tenant AI configuration.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Unique identifier for the tenant
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_ai::config::TenantAIConfig;
    ///
    /// let config = TenantAIConfig::new("tenant-123".to_string());
    /// assert_eq!(config.tenant_id, "tenant-123");
    /// assert_eq!(config.providers.len(), 0);
    /// ```
    pub fn new(tenant_id: String) -> Self {
        Self {
            tenant_id,
            providers: Vec::new(),
            embedding_settings: None,
            processing_defaults: None,
        }
    }

    /// Gets the default provider for a specific use case.
    ///
    /// Returns the first enabled provider that supports the use case.
    pub fn get_default_provider(&self, use_case: AIUseCase) -> Option<&AIProviderConfig> {
        self.providers.iter().find(|p| {
            p.enabled
                && p.models
                    .iter()
                    .any(|m| m.use_cases.contains(&use_case) && m.is_default)
        })
    }

    /// Gets a specific model configuration by ID.
    ///
    /// Supports two formats:
    /// - `provider:model` - Explicitly specifies the provider (e.g., `openai:gpt-4o`)
    /// - `model` - Searches all providers for a matching model (backward compatible)
    pub fn get_model(&self, model_id: &str) -> Option<(&AIProviderConfig, &AIModelConfig)> {
        // Parse provider:model format
        if let Some((prefix, model_name)) = model_id.split_once(':') {
            // Find provider by prefix
            if let Some(target_provider) = AIProvider::from_serde_name(prefix) {
                for provider in &self.providers {
                    if provider.provider == target_provider {
                        if let Some(model) =
                            provider.models.iter().find(|m| m.model_id == model_name)
                        {
                            return Some((provider, model));
                        }
                    }
                }
            }
            return None; // Provider not found or model not in provider
        }

        // Fallback: flat lookup (backward compatible)
        for provider in &self.providers {
            if let Some(model) = provider.models.iter().find(|m| m.model_id == model_id) {
                return Some((provider, model));
            }
        }
        None
    }

    /// Parse a model ID with optional provider prefix.
    ///
    /// Returns `(provider_name, model_name)` where provider_name is `None`
    /// if no prefix was specified.
    pub fn parse_model_id(model_id: &str) -> (Option<&str>, &str) {
        if let Some((prefix, model_name)) = model_id.split_once(':') {
            // Verify it's a valid provider prefix
            if AIProvider::from_serde_name(prefix).is_some() {
                return (Some(prefix), model_name);
            }
        }
        (None, model_id)
    }
}
