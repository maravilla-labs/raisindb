//! Response types for AI completions.
//!
//! Contains the unified [`CompletionResponse`] and usage statistics.

use serde::{Deserialize, Serialize};

use super::message::Message;
use super::tools::ToolCall;

/// A streaming chunk from an AI provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    /// Text delta for this chunk (empty if not a text event)
    #[serde(default)]
    pub delta: String,

    /// Tool call deltas (incremental tool call data)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    /// Usage statistics (typically only on the final chunk)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,

    /// Stop reason (set on the final chunk)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,

    /// Model that generated this chunk
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// A chat completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// The generated message
    pub message: Message,

    /// The model that generated the response
    pub model: String,

    /// Usage statistics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,

    /// Stop reason (e.g., "stop", "length", "tool_use")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

/// Usage statistics for a completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Number of tokens in the prompt
    pub prompt_tokens: u32,
    /// Number of tokens in the completion
    pub completion_tokens: u32,
    /// Total tokens used
    pub total_tokens: u32,
}
