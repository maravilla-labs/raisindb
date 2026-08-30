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
pub use embedder::{EmbedderId, EmbeddingKind, EmbeddingPartition, EmbeddingSettings};
pub use processing::{ProcessingDefaults, DEFAULT_CAPTION_MODEL, DEFAULT_IMAGE_EMBEDDING_MODEL};
pub use provider::{
    validate_slug, AIModelConfig, AIProvider, AIProviderConfig, AIUseCase, SlugError, MAX_SLUG_LEN,
};

use serde::{Deserialize, Serialize};

/// Tenant-level AI configuration.
///
/// Contains all AI provider configurations for a specific tenant.
///
/// Deserialization goes through `StoredTenantAIConfig` so that a config whose
/// entries collide on a slug is repaired on the way in — see
/// [`provider::dedupe_slugs`] for why that has to happen at read time and why
/// renaming is the repair. A slug is an address; nothing above this layer is
/// prepared for two entries answering to one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "StoredTenantAIConfig")]
pub struct TenantAIConfig {
    /// Unique identifier for the tenant
    pub tenant_id: String,
    /// List of configured AI providers, guaranteed slug-unique after a read.
    pub providers: Vec<AIProviderConfig>,
    /// Embedding settings for this tenant
    #[serde(default)]
    pub embedding_settings: Option<EmbeddingSettings>,
    /// Default settings for asset processing (image captioning, embeddings)
    #[serde(default)]
    pub processing_defaults: Option<ProcessingDefaults>,
}

/// On-the-wire shape of [`TenantAIConfig`]; only exists to run the slug
/// deduplication on read. Never construct it directly.
#[derive(Deserialize)]
struct StoredTenantAIConfig {
    tenant_id: String,
    providers: Vec<AIProviderConfig>,
    #[serde(default)]
    embedding_settings: Option<EmbeddingSettings>,
    #[serde(default)]
    processing_defaults: Option<ProcessingDefaults>,
}

impl From<StoredTenantAIConfig> for TenantAIConfig {
    fn from(stored: StoredTenantAIConfig) -> Self {
        Self {
            tenant_id: stored.tenant_id,
            providers: provider::dedupe_slugs(stored.providers),
            embedding_settings: stored.embedding_settings,
            processing_defaults: stored.processing_defaults,
        }
    }
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

    /// Gets a provider entry by its slug.
    ///
    /// Slugs are unique per tenant, so this is the only exact way to address a
    /// provider — several entries can share a [`AIProvider`] kind.
    pub fn get_provider(&self, slug: &str) -> Option<&AIProviderConfig> {
        self.providers.iter().find(|p| p.slug == slug)
    }

    /// Gets a specific model configuration by ID.
    ///
    /// Supports two formats:
    /// - `slug:model` - Explicitly specifies the provider entry (e.g., `openai:gpt-4o`)
    /// - `model` - Searches all providers for a matching model (backward compatible)
    ///
    /// Legacy ids keep working because entries stored before slugs existed get
    /// `slug = kind.serde_name()`, so `openai:gpt-4o` still hits the OpenAI entry.
    pub fn get_model(&self, model_id: &str) -> Option<(&AIProviderConfig, &AIModelConfig)> {
        // Parse slug:model format
        if let Some((prefix, model_name)) = model_id.split_once(':') {
            // Find provider by slug
            if let Some(provider) = self.get_provider(prefix) {
                if let Some(model) = provider.models.iter().find(|m| m.model_id == model_name) {
                    return Some((provider, model));
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

    /// Parse a model ID with optional provider-slug prefix.
    ///
    /// Returns `(slug, model_name)` where `slug` is `None` if no prefix was
    /// specified, or if the prefix is not a slug this tenant has configured —
    /// model ids legitimately contain `:` (`qwen2.5-coder:latest`,
    /// `text-embedding-3:small`), so an unrecognised prefix means "no prefix",
    /// not "unknown provider".
    ///
    /// Takes `&self` because prefix validity is now tenant-dependent: the set of
    /// valid prefixes is this tenant's configured slugs, not the static enum
    /// table.
    pub fn parse_model_id<'a>(&self, model_id: &'a str) -> (Option<&'a str>, &'a str) {
        if let Some((prefix, model_name)) = model_id.split_once(':') {
            if self.get_provider(prefix).is_some() {
                return (Some(prefix), model_name);
            }
        }
        (None, model_id)
    }
}
