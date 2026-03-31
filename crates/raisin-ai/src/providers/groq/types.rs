//! API request and response types for Groq provider.

use serde::{Deserialize, Serialize};

/// Groq chat completion request (OpenAI-compatible)
#[derive(Debug, Serialize)]
pub(super) struct GroqChatRequest {
    pub model: String,
    pub messages: Vec<GroqMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<GroqToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<GroqToolChoice>,
    /// Response format for structured output (json_object mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<GroqResponseFormat>,
    /// Whether to stream the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

/// Response format for Groq (OpenAI-compatible json_object mode)
#[derive(Debug, Serialize)]
pub(super) struct GroqResponseFormat {
    #[serde(rename = "type")]
    pub format_type: String,
}

/// Tool choice for Groq (OpenAI-compatible).
///
/// Can be a simple string like `"auto"` / `"none"` / `"required"`,
/// or an object that forces a specific function.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum GroqToolChoice {
    /// A simple string value: "auto", "none", or "required"
    Mode(String),
    /// Force a specific tool by name
    Specific(GroqToolChoiceSpecific),
}

/// Forces the model to call a specific function.
#[derive(Debug, Serialize)]
pub(super) struct GroqToolChoiceSpecific {
    #[serde(rename = "type")]
    pub choice_type: String,
    pub function: GroqToolChoiceFunction,
}

/// Function name inside a specific tool choice.
#[derive(Debug, Serialize)]
pub(super) struct GroqToolChoiceFunction {
    pub name: String,
}

/// Groq message format (OpenAI-compatible)
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct GroqMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<GroqToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Tool call in Groq format
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct GroqToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: GroqFunctionCall,
}

/// Function call details
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct GroqFunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Tool definition for Groq (OpenAI-compatible)
#[derive(Debug, Serialize)]
pub(super) struct GroqToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: GroqFunctionDefinition,
}

/// Function definition for tools
#[derive(Debug, Serialize)]
pub(super) struct GroqFunctionDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

/// Groq chat completion response (OpenAI-compatible)
#[derive(Debug, Deserialize)]
pub(super) struct GroqChatResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<GroqChoice>,
    pub usage: GroqUsage,
}

/// Response choice
#[derive(Debug, Deserialize)]
pub(super) struct GroqChoice {
    pub index: usize,
    pub message: GroqMessage,
    pub finish_reason: Option<String>,
}

/// Usage statistics
#[derive(Debug, Deserialize)]
pub(super) struct GroqUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Groq models list response
#[derive(Debug, Deserialize)]
pub(super) struct GroqModelsResponse {
    pub data: Vec<GroqModel>,
}

/// Groq model object
#[derive(Debug, Deserialize)]
pub(super) struct GroqModel {
    pub id: String,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub owned_by: String,
    #[serde(default)]
    pub active: Option<bool>,
}

// --- Streaming types (OpenAI-compatible SSE format) ---

/// A single SSE chunk from Groq's streaming chat completions endpoint.
#[derive(Debug, Deserialize)]
pub(super) struct GroqStreamChunk {
    #[serde(default)]
    pub id: String,
    pub choices: Vec<GroqStreamChoice>,
    #[serde(default)]
    pub usage: Option<GroqUsage>,
    #[serde(default)]
    pub model: Option<String>,
}

/// A choice inside a streaming chunk.
#[derive(Debug, Deserialize)]
pub(super) struct GroqStreamChoice {
    #[serde(default)]
    pub index: usize,
    pub delta: GroqStreamDelta,
    pub finish_reason: Option<String>,
}

/// The delta payload inside a streaming choice.
#[derive(Debug, Deserialize)]
pub(super) struct GroqStreamDelta {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<GroqStreamToolCall>>,
}

/// A tool call delta in a streaming chunk.
#[derive(Debug, Deserialize)]
pub(super) struct GroqStreamToolCall {
    pub index: usize,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub call_type: Option<String>,
    pub function: Option<GroqStreamFunctionCall>,
}

/// Function call delta in a streaming tool call.
#[derive(Debug, Deserialize)]
pub(super) struct GroqStreamFunctionCall {
    pub name: Option<String>,
    pub arguments: Option<String>,
}
