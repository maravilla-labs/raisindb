//! API request and response types for Azure OpenAI provider.

use serde::{Deserialize, Serialize};

/// Azure OpenAI chat completion request
#[derive(Debug, Serialize)]
pub(super) struct AzureChatRequest {
    pub messages: Vec<AzureChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AzureTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Include usage in streaming responses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<AzureStreamOptions>,
}

/// Options for streaming responses
#[derive(Debug, Serialize)]
pub(super) struct AzureStreamOptions {
    pub include_usage: bool,
}

/// Azure OpenAI chat message variants
#[derive(Debug, Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub(super) enum AzureChatMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: String,
    },
    #[serde(rename = "assistant")]
    AssistantWithTools {
        content: Option<String>,
        tool_calls: Vec<AzureToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

/// Azure OpenAI tool call in assistant message
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct AzureToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: AzureFunctionCall,
}

/// Azure OpenAI function call details
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct AzureFunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Azure OpenAI tool definition
#[derive(Debug, Serialize)]
pub(super) struct AzureTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: AzureFunctionDefinition,
}

/// Azure OpenAI function definition
#[derive(Debug, Serialize)]
pub(super) struct AzureFunctionDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

/// Azure OpenAI chat completion response
#[derive(Debug, Deserialize)]
pub(super) struct AzureChatResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<AzureChoice>,
    pub usage: AzureUsage,
}

/// Azure OpenAI response choice
#[derive(Debug, Deserialize)]
pub(super) struct AzureChoice {
    pub index: usize,
    pub message: AzureResponseMessage,
    pub finish_reason: Option<String>,
}

/// Azure OpenAI response message
#[derive(Debug, Deserialize)]
pub(super) struct AzureResponseMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<AzureToolCall>,
}

/// Azure OpenAI usage statistics
#[derive(Debug, Deserialize)]
pub(super) struct AzureUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Azure OpenAI error response
#[derive(Debug, Deserialize)]
pub(super) struct AzureError {
    pub error: AzureErrorDetail,
}

#[derive(Debug, Deserialize)]
pub(super) struct AzureErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: Option<String>,
    pub code: Option<String>,
}

// --- Streaming types (OpenAI-compatible SSE format) ---

/// A single SSE chunk from Azure OpenAI's streaming chat completions endpoint.
#[derive(Debug, Deserialize)]
pub(super) struct AzureStreamChunk {
    #[serde(default)]
    pub choices: Vec<AzureStreamChoice>,
    #[serde(default)]
    pub usage: Option<AzureUsage>,
    #[serde(default)]
    pub model: Option<String>,
}

/// A choice inside a streaming chunk.
#[derive(Debug, Deserialize)]
pub(super) struct AzureStreamChoice {
    pub delta: AzureStreamDelta,
    pub finish_reason: Option<String>,
}

/// The delta payload inside a streaming choice.
#[derive(Debug, Deserialize)]
pub(super) struct AzureStreamDelta {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<AzureStreamToolCall>>,
}

/// A tool call delta in a streaming chunk.
#[derive(Debug, Deserialize)]
pub(super) struct AzureStreamToolCall {
    #[serde(default)]
    pub index: usize,
    pub id: Option<String>,
    pub function: Option<AzureStreamFunctionCall>,
}

/// Function call delta in a streaming tool call.
#[derive(Debug, Deserialize)]
pub(super) struct AzureStreamFunctionCall {
    pub name: Option<String>,
    pub arguments: Option<String>,
}
