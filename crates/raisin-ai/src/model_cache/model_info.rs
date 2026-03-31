//! Core model types: ModelInfo, ModelCapabilities, SchemaTransformerType.

/// Represents information about an AI model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    /// Unique identifier for the model
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Model description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Model capabilities
    pub capabilities: ModelCapabilities,

    /// Context window size (in tokens)
    pub context_window: Option<u32>,

    /// Maximum output tokens
    pub max_output_tokens: Option<u32>,

    /// Whether this model is currently available
    pub available: bool,

    /// Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl ModelInfo {
    /// Creates a new ModelInfo with the given ID and name.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            capabilities: ModelCapabilities::default(),
            context_window: None,
            max_output_tokens: None,
            available: true,
            metadata: None,
        }
    }

    /// Sets the model description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the model capabilities.
    pub fn with_capabilities(mut self, capabilities: ModelCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Sets the context window size.
    pub fn with_context_window(mut self, tokens: u32) -> Self {
        self.context_window = Some(tokens);
        self
    }

    /// Sets the maximum output tokens.
    pub fn with_max_output_tokens(mut self, tokens: u32) -> Self {
        self.max_output_tokens = Some(tokens);
        self
    }

    /// Sets whether the model is available.
    pub fn with_availability(mut self, available: bool) -> Self {
        self.available = available;
        self
    }

    /// Sets additional metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Capabilities of an AI model.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ModelCapabilities {
    /// Supports chat/conversation
    pub chat: bool,

    /// Supports embeddings
    pub embeddings: bool,

    /// Supports vision/image inputs
    pub vision: bool,

    /// Supports tool/function calling
    pub tools: bool,

    /// Supports streaming responses
    pub streaming: bool,
}

impl ModelCapabilities {
    /// Creates capabilities for a chat model.
    pub fn chat() -> Self {
        Self {
            chat: true,
            streaming: true,
            ..Default::default()
        }
    }

    /// Creates capabilities for a chat model with tools.
    pub fn chat_with_tools() -> Self {
        Self {
            chat: true,
            streaming: true,
            tools: true,
            ..Default::default()
        }
    }

    /// Creates capabilities for a vision model.
    pub fn vision() -> Self {
        Self {
            chat: true,
            streaming: true,
            vision: true,
            ..Default::default()
        }
    }

    /// Creates capabilities for an embeddings model.
    pub fn embeddings() -> Self {
        Self {
            embeddings: true,
            ..Default::default()
        }
    }
}

/// Schema transformation type for structured output.
///
/// Different providers require different transformations to JSON schemas
/// for structured output support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SchemaTransformerType {
    /// OpenAI: Sets additionalProperties:false, makes all properties required
    OpenAI,

    /// Anthropic: Sets additionalProperties:false
    Anthropic,

    /// Google: Minimal transformation
    Google,

    /// No transformation, pass-through
    None,
}
