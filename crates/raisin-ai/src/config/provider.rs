//! AI provider types, model configuration, and use cases.

use serde::{Deserialize, Serialize};

/// AI use cases supported by the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AIUseCase {
    /// Text embeddings for semantic search
    Embedding,
    /// Chat/conversational AI
    Chat,
    /// Agentic workflows with tool use
    Agent,
    /// Text completion/generation
    Completion,
    /// Text classification tasks
    Classification,
}

/// Supported AI providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AIProvider {
    /// OpenAI (GPT models)
    #[serde(rename = "openai")]
    OpenAI,
    /// Anthropic (Claude models)
    #[serde(rename = "anthropic")]
    Anthropic,
    /// Google (Gemini, PaLM)
    #[serde(rename = "google")]
    Google,
    /// Ollama (local models via API)
    #[serde(rename = "ollama")]
    Ollama,
    /// Azure OpenAI Service
    #[serde(rename = "azure_openai")]
    AzureOpenAI,
    /// Groq (fast inference for open-source models)
    #[serde(rename = "groq")]
    Groq,
    /// OpenRouter (multi-provider router with unified API)
    #[serde(rename = "openrouter")]
    OpenRouter,
    /// AWS Bedrock (Claude, Nova, Llama models via AWS)
    #[serde(rename = "bedrock")]
    Bedrock,
    /// Custom provider
    #[serde(rename = "custom")]
    Custom,
    /// Local Candle models (Moondream, BLIP, CLIP) - runs in-process
    #[serde(rename = "local")]
    Local,
}

impl AIProvider {
    /// Returns the serde serialization name for this provider.
    ///
    /// Used for matching model prefixes like `openai:gpt-4o`.
    pub fn serde_name(&self) -> &'static str {
        match self {
            AIProvider::OpenAI => "openai",
            AIProvider::Anthropic => "anthropic",
            AIProvider::Google => "google",
            AIProvider::Ollama => "ollama",
            AIProvider::AzureOpenAI => "azure_openai",
            AIProvider::Groq => "groq",
            AIProvider::OpenRouter => "openrouter",
            AIProvider::Bedrock => "bedrock",
            AIProvider::Custom => "custom",
            AIProvider::Local => "local",
        }
    }

    /// Parse a provider from its serde name.
    ///
    /// Returns `None` if the name doesn't match any known provider.
    pub fn from_serde_name(name: &str) -> Option<Self> {
        match name {
            "openai" => Some(AIProvider::OpenAI),
            "anthropic" => Some(AIProvider::Anthropic),
            "google" => Some(AIProvider::Google),
            "ollama" => Some(AIProvider::Ollama),
            "azure_openai" => Some(AIProvider::AzureOpenAI),
            "groq" => Some(AIProvider::Groq),
            "openrouter" => Some(AIProvider::OpenRouter),
            "bedrock" => Some(AIProvider::Bedrock),
            "custom" => Some(AIProvider::Custom),
            "local" => Some(AIProvider::Local),
            _ => None,
        }
    }

    /// Returns the default API endpoint for this provider.
    ///
    /// Returns `None` for providers that require custom endpoints or run locally.
    pub fn default_endpoint(&self) -> Option<&'static str> {
        match self {
            AIProvider::OpenAI => Some("https://api.openai.com/v1"),
            AIProvider::Anthropic => Some("https://api.anthropic.com/v1"),
            AIProvider::Google => Some("https://generativelanguage.googleapis.com/v1"),
            AIProvider::Ollama => Some("http://localhost:11434"),
            AIProvider::Groq => Some("https://api.groq.com/openai/v1"),
            AIProvider::OpenRouter => Some("https://openrouter.ai/api/v1"),
            AIProvider::AzureOpenAI => None, // Requires tenant-specific endpoint
            AIProvider::Bedrock => None,     // Requires AWS region-specific endpoint
            AIProvider::Custom => None,      // Requires custom endpoint
            AIProvider::Local => None,       // Runs in-process, no endpoint needed
        }
    }

    /// Returns whether this provider requires an API key.
    pub fn requires_api_key(&self) -> bool {
        match self {
            AIProvider::OpenAI
            | AIProvider::Anthropic
            | AIProvider::Google
            | AIProvider::AzureOpenAI
            | AIProvider::Groq
            | AIProvider::OpenRouter
            | AIProvider::Bedrock => true,
            AIProvider::Ollama | AIProvider::Custom | AIProvider::Local => false,
        }
    }

    /// Returns whether this provider runs in-process (no network call).
    pub fn is_local(&self) -> bool {
        matches!(self, AIProvider::Local)
    }
}

/// Configuration for a single AI provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIProviderConfig {
    /// The AI provider type
    pub provider: AIProvider,
    /// Encrypted API key (using AES-256-GCM)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_encrypted: Option<Vec<u8>>,
    /// Optional custom API endpoint (for self-hosted or custom providers)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_endpoint: Option<String>,
    /// Whether this provider is enabled
    pub enabled: bool,
    /// List of models available from this provider
    pub models: Vec<AIModelConfig>,
}

impl AIProviderConfig {
    /// Creates a new provider configuration.
    pub fn new(provider: AIProvider) -> Self {
        Self {
            provider,
            api_key_encrypted: None,
            api_endpoint: None,
            enabled: true,
            models: Vec::new(),
        }
    }

    /// Gets the default model for a specific use case.
    pub fn get_default_model(&self, use_case: AIUseCase) -> Option<&AIModelConfig> {
        self.models
            .iter()
            .find(|m| m.use_cases.contains(&use_case) && m.is_default)
    }
}

/// Configuration for a specific AI model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIModelConfig {
    /// Unique identifier for the model (e.g., "gpt-4", "claude-3-opus")
    pub model_id: String,
    /// Human-readable display name
    pub display_name: String,
    /// Use cases this model supports
    pub use_cases: Vec<AIUseCase>,
    /// Default temperature for sampling (0.0-2.0)
    pub default_temperature: f32,
    /// Default maximum tokens to generate
    pub default_max_tokens: u32,
    /// Whether this is the default model for its use cases
    pub is_default: bool,
    /// Optional metadata (architecture, embedding_length, etc. from provider)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl AIModelConfig {
    /// Creates a new model configuration.
    pub fn new(model_id: String, display_name: String) -> Self {
        Self {
            model_id,
            display_name,
            use_cases: Vec::new(),
            default_temperature: 0.7,
            default_max_tokens: 1024,
            is_default: false,
            metadata: None,
        }
    }

    /// Creates a new model configuration with metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}
