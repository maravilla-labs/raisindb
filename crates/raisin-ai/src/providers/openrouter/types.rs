//! API request and response types for OpenRouter provider.

use serde::{Deserialize, Serialize};

/// OpenAI-compatible chat request format
#[derive(Debug, Serialize)]
pub(super) struct OpenAIChatRequest {
    pub model: String,
    pub messages: Vec<OpenAIChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Response format for structured output (json_object mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<OpenRouterResponseFormat>,
}

/// Response format for OpenRouter (OpenAI-compatible json_object and json_schema modes)
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum OpenRouterResponseFormat {
    /// JSON mode - model must output valid JSON
    JsonObject,
    /// JSON Schema mode - model must follow the schema (depends on underlying model)
    JsonSchema {
        #[serde(rename = "json_schema")]
        schema: OpenRouterJsonSchema,
    },
}

/// JSON schema for structured output
#[derive(Debug, Serialize)]
pub(super) struct OpenRouterJsonSchema {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct OpenAIChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAIToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct OpenAIToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAIFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct OpenAIFunctionCall {
    pub name: String,
    pub arguments: String,
}

/// OpenAI-compatible chat response format
#[derive(Debug, Deserialize)]
pub(super) struct OpenAIChatResponse {
    pub id: String,
    pub choices: Vec<OpenAIChoice>,
    pub model: String,
    pub usage: OpenAIUsage,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAIChoice {
    pub message: OpenAIChatMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAIUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// OpenAI API error response
#[derive(Debug, Deserialize)]
pub(super) struct OpenAIError {
    pub error: OpenAIErrorDetail,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAIErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: Option<String>,
}

/// OpenRouter models list response
#[derive(Debug, Deserialize)]
pub(super) struct OpenRouterModelsResponse {
    pub data: Vec<OpenRouterModel>,
}

/// OpenRouter model object
#[derive(Debug, Deserialize)]
pub(super) struct OpenRouterModel {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub context_length: Option<i64>,
    pub pricing: OpenRouterPricing,
    #[serde(default)]
    pub architecture: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenRouterPricing {
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub completion: String,
}

// --- Streaming types (OpenAI-compatible SSE format) ---

/// A single SSE chunk from OpenRouter's streaming chat completions endpoint.
#[derive(Debug, Deserialize)]
pub(super) struct OpenRouterStreamChunk {
    #[serde(default)]
    pub choices: Vec<OpenRouterStreamChoice>,
    #[serde(default)]
    pub usage: Option<OpenAIUsage>,
    #[serde(default)]
    pub model: Option<String>,
}

/// A choice inside a streaming chunk.
#[derive(Debug, Deserialize)]
pub(super) struct OpenRouterStreamChoice {
    pub delta: OpenRouterStreamDelta,
    pub finish_reason: Option<String>,
}

/// The delta payload inside a streaming choice.
#[derive(Debug, Deserialize)]
pub(super) struct OpenRouterStreamDelta {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<OpenRouterStreamToolCall>>,
}

/// A tool call delta in a streaming chunk.
#[derive(Debug, Deserialize)]
pub(super) struct OpenRouterStreamToolCall {
    #[serde(default)]
    pub index: usize,
    pub id: Option<String>,
    pub function: Option<OpenRouterStreamFunctionCall>,
}

/// Function call delta in a streaming tool call.
#[derive(Debug, Deserialize)]
pub(super) struct OpenRouterStreamFunctionCall {
    pub name: Option<String>,
    pub arguments: Option<String>,
}
