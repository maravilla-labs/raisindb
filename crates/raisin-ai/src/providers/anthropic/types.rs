//! Anthropic API request and response types.

use serde::{Deserialize, Serialize};

/// Anthropic Messages API request format.
#[derive(Debug, Serialize)]
pub(super) struct AnthropicChatRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<AnthropicToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

/// Anthropic message format.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct AnthropicMessage {
    pub role: String,
    pub content: Vec<AnthropicContent>,
}

/// Content block in an Anthropic message.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum AnthropicContent {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

/// Tool definition for Anthropic API.
#[derive(Debug, Serialize)]
pub(super) struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Tool choice for Anthropic API.
///
/// Can be a simple mode (`"auto"`, `"any"`, `"none"`) or force
/// a specific tool by name.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum AnthropicToolChoice {
    /// A simple mode string: "auto", "any", "none"
    Mode(AnthropicToolChoiceMode),
    /// Force a specific tool by name
    Specific(AnthropicToolChoiceSpecific),
}

/// Simple tool choice mode object.
#[derive(Debug, Serialize)]
pub(super) struct AnthropicToolChoiceMode {
    #[serde(rename = "type")]
    pub choice_type: String,
}

/// Force a specific tool.
#[derive(Debug, Serialize)]
pub(super) struct AnthropicToolChoiceSpecific {
    #[serde(rename = "type")]
    pub choice_type: String,
    pub name: String,
}

// ── Response types ────────────────────────────────────────────────

/// Anthropic Messages API response format.
#[derive(Debug, Deserialize)]
pub(super) struct AnthropicChatResponse {
    #[allow(dead_code)]
    pub id: String,
    pub content: Vec<AnthropicResponseContent>,
    pub model: String,
    #[allow(dead_code)]
    pub role: String,
    pub stop_reason: Option<String>,
    pub usage: AnthropicUsage,
}

/// Content block in an Anthropic response.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum AnthropicResponseContent {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

/// Usage statistics from Anthropic.
#[derive(Debug, Deserialize)]
pub(super) struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

// ── Streaming types ───────────────────────────────────────────────

/// A single SSE event from Anthropic's streaming Messages API.
///
/// Anthropic uses `event:` lines to distinguish event types:
/// - `message_start`
/// - `content_block_start`
/// - `content_block_delta`
/// - `content_block_stop`
/// - `message_delta`
/// - `message_stop`
#[derive(Debug, Deserialize)]
pub(super) struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,

    /// Present on `message_start` events.
    #[serde(default)]
    pub message: Option<AnthropicStreamMessage>,

    /// Present on `content_block_start` events.
    #[serde(default)]
    pub content_block: Option<AnthropicStreamContentBlock>,

    /// Present on `content_block_delta` events.
    #[serde(default)]
    pub delta: Option<AnthropicStreamDelta>,

    /// Present on `content_block_delta` and `content_block_start` events.
    #[serde(default)]
    pub index: Option<usize>,

    /// Present on `message_delta` events (contains stop_reason, usage).
    #[serde(default)]
    pub usage: Option<AnthropicUsage>,
}

/// Message object in a `message_start` event.
#[derive(Debug, Deserialize)]
pub(super) struct AnthropicStreamMessage {
    pub model: String,
    pub usage: AnthropicUsage,
}

/// Content block in a `content_block_start` event.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum AnthropicStreamContentBlock {
    Text {
        #[allow(dead_code)]
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
    },
}

/// Delta payload in a `content_block_delta` event.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum AnthropicStreamDelta {
    TextDelta {
        text: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    /// The `message_delta` event uses an untagged `delta` with `stop_reason`.
    #[serde(untagged)]
    MessageDelta {
        stop_reason: Option<String>,
    },
}
