//! Request types for AI completions.
//!
//! Contains the unified [`CompletionRequest`] and related types that can be
//! translated to provider-specific formats (OpenAI, Anthropic, etc.).

use serde::{Deserialize, Serialize};

use super::message::Message;
use super::tools::ToolDefinition;

/// A chat completion request.
///
/// This is a unified request type that can be translated to provider-specific
/// formats (OpenAI, Anthropic, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// The model to use for completion
    pub model: String,

    /// The conversation messages
    pub messages: Vec<Message>,

    /// Optional system prompt
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,

    /// Sampling temperature (0.0-2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Maximum tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// Optional tool definitions for agentic workflows
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,

    /// Whether to stream the response
    #[serde(default)]
    pub stream: bool,

    /// Response format for structured output (JSON schema)
    ///
    /// Use this to get clean JSON responses instead of prose.
    /// The format is automatically converted to each provider's native format:
    /// - OpenAI: `response_format.type: "json_schema"`
    /// - Ollama: `format: "json"` or schema
    /// - Groq: `response_format.type: "json_object"`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
}

/// Response format for structured output.
///
/// This is a unified format that gets converted to each provider's native format:
/// - OpenAI: `response_format: { type: "json_schema", json_schema: {...} }`
/// - Ollama: `format: "json"` or `format: { schema: {...} }`
/// - Groq: `response_format: { type: "json_object" }`
///
/// # Example
///
/// ```rust
/// use raisin_ai::types::{ResponseFormat, JsonSchemaSpec};
/// use serde_json::json;
///
/// // Simple JSON mode (any valid JSON)
/// let json_mode = ResponseFormat::JsonObject;
///
/// // Strict JSON schema
/// let schema_mode = ResponseFormat::JsonSchema {
///     schema: JsonSchemaSpec {
///         name: Some("keywords".to_string()),
///         schema: json!({
///             "type": "object",
///             "properties": {
///                 "keywords": { "type": "array", "items": { "type": "string" } }
///             },
///             "required": ["keywords"]
///         }),
///         strict: true,
///     }
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Plain text (default behavior)
    Text,
    /// JSON mode - model must output valid JSON
    JsonObject,
    /// JSON Schema mode - model must follow the provided schema
    JsonSchema {
        /// The JSON schema specification
        #[serde(rename = "json_schema")]
        schema: JsonSchemaSpec,
    },
}

/// JSON schema specification for structured output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSchemaSpec {
    /// Optional name for the schema (helps with debugging)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The actual JSON schema
    pub schema: serde_json::Value,
    /// Whether to strictly enforce the schema (default: false)
    #[serde(default)]
    pub strict: bool,
}

impl CompletionRequest {
    /// Creates a new completion request.
    ///
    /// # Arguments
    ///
    /// * `model` - The model to use for completion
    /// * `messages` - The conversation messages
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_ai::types::{CompletionRequest, Message, Role};
    ///
    /// let request = CompletionRequest::new(
    ///     "gpt-4".to_string(),
    ///     vec![Message::user("Hello, world!")],
    /// );
    /// ```
    pub fn new(model: String, messages: Vec<Message>) -> Self {
        Self {
            model,
            messages,
            system: None,
            temperature: None,
            max_tokens: None,
            tools: None,
            stream: false,
            response_format: None,
        }
    }

    /// Sets the system prompt.
    pub fn with_system(mut self, system: String) -> Self {
        self.system = Some(system);
        self
    }

    /// Sets the temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Sets the maximum tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Sets the tools.
    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Enables streaming.
    pub fn with_streaming(mut self) -> Self {
        self.stream = true;
        self
    }
}
