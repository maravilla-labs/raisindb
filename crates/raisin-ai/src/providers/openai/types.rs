//! OpenAI API request and response types.

use serde::{Deserialize, Serialize};

/// OpenAI Responses API request format
#[derive(Debug, Serialize)]
pub(crate) struct OpenAIResponsesRequest {
    pub model: String,
    pub input: Vec<OpenAIInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<OpenAITextSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

/// Text settings for response format
#[derive(Debug, Serialize)]
pub(crate) struct OpenAITextSettings {
    pub format: OpenAIResponseFormat,
}

/// Response format for structured output
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum OpenAIResponseFormat {
    Text,
    JsonObject,
    JsonSchema {
        #[serde(rename = "json_schema")]
        schema: OpenAIJsonSchema,
    },
}

/// JSON schema for structured output
#[derive(Debug, Serialize)]
pub(crate) struct OpenAIJsonSchema {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

/// OpenAI input item (message or function call/output)
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum OpenAIInputItem {
    Message(OpenAIMessage),
    FunctionCall(OpenAIFunctionCallInput),
    FunctionCallOutput(OpenAIFunctionCallOutputInput),
}

/// OpenAI message format
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpenAIMessage {
    pub role: String,
    pub content: String,
}

/// Function call in input
#[derive(Debug, Serialize)]
pub(crate) struct OpenAIFunctionCallInput {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

/// Function call output in input
#[derive(Debug, Serialize)]
pub(crate) struct OpenAIFunctionCallOutputInput {
    pub call_id: String,
    pub output: String,
}

/// Tool definition for Responses API
#[derive(Debug, Serialize)]
pub(crate) struct OpenAIResponsesToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

/// OpenAI Responses API response format
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIResponsesResponse {
    pub id: String,
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub created_at: u64,
    pub status: String,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
    #[serde(default)]
    pub incomplete_details: Option<serde_json::Value>,
    pub model: String,
    pub output: Vec<OpenAIOutputItem>,
    #[serde(default)]
    pub output_text: Option<String>,
    pub usage: OpenAIUsage,
    #[serde(default)]
    pub reasoning: Option<serde_json::Value>,
    #[serde(default)]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default)]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub store: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub text: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub truncation: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// OpenAI output item
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum OpenAIOutputItem {
    Message(OpenAIOutputMessage),
    FunctionCall(OpenAIFunctionCallOutput),
}

/// Output message
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIOutputMessage {
    pub role: String,
    pub content: Vec<OpenAIContentItem>,
}

/// Content item in output message
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum OpenAIContentItem {
    OutputText {
        text: String,
        #[serde(default)]
        annotations: Vec<serde_json::Value>,
    },
}

/// Function call in output
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIFunctionCallOutput {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

/// Token details for input tokens
#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpenAIInputTokenDetails {
    #[serde(default)]
    pub cached_tokens: u32,
}

/// Token details for output tokens
#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpenAIOutputTokenDetails {
    #[serde(default)]
    pub reasoning_tokens: u32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    #[serde(default)]
    pub input_tokens_details: Option<OpenAIInputTokenDetails>,
    #[serde(default)]
    pub output_tokens_details: Option<OpenAIOutputTokenDetails>,
}

/// OpenAI models list response
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIModelsResponse {
    pub data: Vec<OpenAIModel>,
}

/// OpenAI model object
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIModel {
    pub id: String,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub owned_by: String,
}

/// SSE streaming event from OpenAI Responses API
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIStreamEvent {
    /// Event type (e.g., "response.output_text.delta")
    #[serde(rename = "type")]
    pub event_type: String,
    /// Text delta (for text delta events)
    #[serde(default)]
    pub delta: Option<String>,
    /// Output index
    #[serde(default)]
    pub output_index: Option<usize>,
    /// Content index
    #[serde(default)]
    pub content_index: Option<usize>,
    /// Item (for output_item events)
    #[serde(default)]
    pub item: Option<serde_json::Value>,
    /// Response object (for response.completed)
    #[serde(default)]
    pub response: Option<OpenAIStreamResponseData>,
}

/// Response data in a streaming completed event
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIStreamResponseData {
    pub model: String,
    pub usage: OpenAIUsage,
    pub status: String,
}

/// OpenAI embeddings request format
#[derive(Debug, Serialize)]
pub(crate) struct OpenAIEmbeddingRequest {
    pub model: String,
    pub input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<String>,
}

/// OpenAI embeddings response format
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIEmbeddingResponse {
    pub data: Vec<OpenAIEmbeddingData>,
    pub model: String,
    pub usage: OpenAIEmbeddingUsage,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIEmbeddingData {
    pub embedding: Vec<f32>,
    pub index: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIEmbeddingUsage {
    pub prompt_tokens: u32,
    pub total_tokens: u32,
}
