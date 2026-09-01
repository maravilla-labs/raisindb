// TODO(v0.2): Serde response struct fields kept for deserialization compatibility
#![allow(dead_code)]

//! AI/LLM provider integration for RaisinDB.
//!
//! This crate provides the foundational infrastructure for managing AI/LLM providers
//! in RaisinDB, including:
//!
//! - Tenant-level AI configuration supporting multiple providers
//! - Secure API key storage with AES-256-GCM encryption
//! - Unified provider abstraction for completions and chat
//! - Storage abstractions for configuration persistence
//!
//! # Architecture
//!
//! The AI system is organized into several modules:
//!
//! - [`config`] - Configuration models for tenant AI settings and providers
//! - [`crypto`] - API key encryption/decryption using AES-256-GCM
//! - [`storage`] - Storage trait for configuration persistence
//! - [`types`] - Common types for AI requests and responses
//! - [`provider`] - Provider trait and implementations
//!
//! # Supported Providers
//!
//! - OpenAI (GPT-4, GPT-3.5, etc.)
//! - Anthropic (Claude models)
//! - Google Gemini (Gemini 1.5, 2.0 with tool calling)
//! - Azure OpenAI (Azure-hosted OpenAI models)
//! - Groq (fast inference for open-source models)
//! - OpenRouter (multi-provider router with unified API)
//! - AWS Bedrock (Claude, Nova, Llama models via AWS)
//! - Ollama (local models)
//! - Custom providers
//!
//! # Usage
//!
//! ```rust,ignore
//! use raisin_ai::config::{TenantAIConfig, AIProviderConfig, AIProvider};
//! use raisin_ai::crypto::ApiKeyEncryptor;
//!
//! // Create a new tenant configuration
//! let mut config = TenantAIConfig {
//!     tenant_id: "my-tenant".to_string(),
//!     providers: vec![],
//! };
//!
//! // Encrypt API key
//! let master_key = [0u8; 32]; // Use secure key in production
//! let encryptor = ApiKeyEncryptor::new(&master_key);
//! let encrypted = encryptor.encrypt("sk-my-api-key").unwrap();
//!
//! // Add provider configuration
//! let provider_config = AIProviderConfig {
//!     slug: "openai".to_string(), // per-tenant id, also the `slug:model` prefix
//!     kind: AIProvider::OpenAI,
//!     display_name: None,
//!     icon_url: None,
//!     api_key_encrypted: Some(encrypted),
//!     api_endpoint: None,
//!     enabled: true,
//!     models: vec![],
//! };
//! config.providers.push(provider_config);
//!
//! // Store configuration (requires a storage implementation)
//! // store.set_config(&config).await.unwrap();
//! ```
//!
//! # Security Considerations
//!
//! - API keys are encrypted using AES-256-GCM before storage
//! - Master encryption keys should be stored securely (env vars, secrets manager)
//! - Never return encrypted API keys to clients
//! - Use separate storage from main data for enhanced security

pub mod chunking;
pub mod config;
pub mod crypto;
pub mod huggingface;
pub mod model_cache;
pub mod model_classifier;
pub mod pdf;
pub mod provider;
pub mod providers;
pub mod rules;
pub mod storage;
pub mod streaming;
pub mod tool_call_extraction;
pub mod types;
pub mod utils;
pub mod validation;

// Candle-based local AI inference (requires "candle" feature)
#[cfg(feature = "candle")]
pub mod candle;

// Re-export commonly used types
pub use chunking::{ChunkingError, TextChunk, TextChunker};
pub use config::{
    validate_slug, AIModelConfig, AIProvider, AIProviderConfig, AIUseCase, ChunkingConfig,
    EmbedderId, EmbeddingKind, EmbeddingSettings, OverlapConfig, ProcessingDefaults, SlugError,
    SplitterType, TenantAIConfig, DEFAULT_CAPTION_MODEL, DEFAULT_IMAGE_EMBEDDING_MODEL,
    MAX_SLUG_LEN,
};
pub use crypto::{ApiKeyEncryptor, CryptoError};
pub use model_cache::{
    ModelCache, ModelCapabilities, ModelInfo, ModelProfile, SchemaTransformerType,
};
pub use provider::AIProviderTrait;
pub use providers::{
    AnthropicProvider, AzureOpenAIProvider, BedrockProvider, GeminiProvider, GroqProvider,
    OllamaProvider, OpenAIProvider, OpenRouterProvider,
};
pub use storage::{StorageError, TenantAIConfigStore};
pub use types::{
    CompletionRequest, CompletionResponse, Message, Role, StreamChunk, ToolCall, ToolDefinition,
};
pub use validation::{validate_output, validate_with_details, ValidationError};

// HuggingFace model management
pub use huggingface::{
    DownloadStatus, ModelCapability, ModelError, ModelInfo as HFModelInfo, ModelRegistry,
    ModelResult, ModelType,
};

// PDF processing - core types (always available)
pub use pdf::{
    // OCR types
    get_default_ocr_provider,
    // New storage-based API
    process_pdf_from_storage,
    // Router types for backward compatibility
    ExtractionMethod,
    OcrError,
    OcrOptions,
    OcrProvider,
    PdfProcessedResult,
    PdfProcessingError,
    PdfProcessingOptions,
    PdfProcessor,
    PdfStrategy,
    StoragePdfError,
    StoragePdfOptions,
    StoragePdfResult,
};

// Legacy native extraction (only with pdf feature)
#[cfg(feature = "pdf")]
pub use pdf::{PdfExtractError, PdfTextResult};

// Processing rules
pub use rules::{
    is_text_bearing, mime_matches, ProcessingRule, ProcessingRuleSet, ProcessingSettings,
    RuleMatchContext, RuleMatcher,
};
// The task vocabulary a rule routes a mimetype to, and the planner that says
// which of those tasks THIS process can actually run. See `rules::tasks`.
pub use rules::{
    is_valid_task_slug, lookup_task, plan_tasks, BlockedReason, BlockedTask, PipelinePlan,
    PlannedTask, TaskProvider, TaskSpec, KNOWN_TASKS,
};
// What this process can actually perform, injected from `main.rs` after
// plugins load. `plan_tasks_here` is the form engine code should use; a
// hard-coded predicate is a second answer to the same question. See
// `rules::capabilities`.
pub use rules::{
    capability_available, capability_probe_installed, install_capability_probe, plan_tasks_here,
    CapabilityProbe,
};

// Candle-based local AI inference (requires "candle" feature)
#[cfg(feature = "candle")]
pub use candle::{
    blip::BlipCaptioner, clip::ClipEmbedder, default_caption_model, is_blip_model,
    is_moondream_model, moondream::MoondreamCaptioner, select_device, CandleError, CandleResult,
    CaptionModelInfo, AVAILABLE_CAPTION_MODELS, DEFAULT_MOONDREAM_MODEL,
};
