//! Tests for the Anthropic provider.

use super::trait_impl::parse_anthropic_sse_events;
use super::types::*;
use super::*;
use crate::provider::{AIProviderTrait, ProviderError};
use crate::types::{
    CompletionResponse, FunctionCall, JsonSchemaSpec, Message, ResponseFormat, Role, ToolCall,
};

#[test]
fn test_validate_chat_model() {
    assert!(AnthropicProvider::validate_chat_model("claude-opus-4-5").is_ok());
    assert!(AnthropicProvider::validate_chat_model("claude-sonnet-4-5").is_ok());
    assert!(AnthropicProvider::validate_chat_model("claude-3-5-sonnet").is_ok());
    assert!(AnthropicProvider::validate_chat_model("claude-3-5-sonnet-20241022").is_ok());
    assert!(AnthropicProvider::validate_chat_model("claude-3-5-haiku").is_ok());
    assert!(AnthropicProvider::validate_chat_model("claude-3-haiku-20240307").is_ok());
    assert!(AnthropicProvider::validate_chat_model("invalid-model").is_err());
}

#[test]
fn test_provider_capabilities() {
    let provider = AnthropicProvider::new("test-key");
    assert_eq!(provider.provider_name(), "anthropic");
    assert!(provider.supports_streaming());
    assert!(provider.supports_tools());
    assert!(!provider.available_models().is_empty());
}

#[test]
fn test_build_chat_request_basic() {
    let request = CompletionRequest::new(
        "claude-sonnet-4-5".to_string(),
        vec![Message::user("Hello")],
    );

    let anthropic_request = AnthropicProvider::build_chat_request(&request, false);

    assert_eq!(anthropic_request.model, "claude-sonnet-4-5");
    assert_eq!(anthropic_request.messages.len(), 1);
    assert_eq!(anthropic_request.messages[0].role, "user");
    assert_eq!(anthropic_request.max_tokens, 4096);
    assert!(anthropic_request.stream.is_none());
    assert!(anthropic_request.tools.is_none());
    assert!(anthropic_request.tool_choice.is_none());
}

#[test]
fn test_build_chat_request_with_system() {
    let request = CompletionRequest::new(
        "claude-sonnet-4-5".to_string(),
        vec![Message::system("You are helpful"), Message::user("Hello")],
    );

    let anthropic_request = AnthropicProvider::build_chat_request(&request, false);

    // System messages go into the `system` field, not into messages
    assert_eq!(
        anthropic_request.system,
        Some("You are helpful".to_string())
    );
    // Only the user message should be in the messages array
    assert_eq!(anthropic_request.messages.len(), 1);
    assert_eq!(anthropic_request.messages[0].role, "user");
}

#[test]
fn test_build_chat_request_with_tool_calls() {
    let tool_call = ToolCall {
        id: "call_123".to_string(),
        call_type: "function".to_string(),
        function: FunctionCall {
            name: "get_weather".to_string(),
            arguments: r#"{"location":"Paris"}"#.to_string(),
        },
        index: None,
    };

    let request = CompletionRequest::new(
        "claude-sonnet-4-5".to_string(),
        vec![
            Message::user("What's the weather?"),
            Message::assistant("").with_tool_calls(vec![tool_call]),
            Message::tool(
                r#"{"temperature":20}"#,
                "call_123".to_string(),
                Some("get_weather".to_string()),
            ),
        ],
    );

    let anthropic_request = AnthropicProvider::build_chat_request(&request, false);

    assert_eq!(anthropic_request.messages.len(), 3);

    // First message: user
    assert_eq!(anthropic_request.messages[0].role, "user");

    // Second message: assistant with tool_use content
    assert_eq!(anthropic_request.messages[1].role, "assistant");
    assert!(matches!(
        &anthropic_request.messages[1].content[0],
        AnthropicContent::ToolUse { name, .. } if name == "get_weather"
    ));

    // Third message: tool result (sent as user role in Anthropic)
    assert_eq!(anthropic_request.messages[2].role, "user");
    assert!(matches!(
        &anthropic_request.messages[2].content[0],
        AnthropicContent::ToolResult { tool_use_id, .. } if tool_use_id == "call_123"
    ));
}

#[test]
fn test_build_chat_request_streaming() {
    let request = CompletionRequest::new(
        "claude-sonnet-4-5".to_string(),
        vec![Message::user("Hello")],
    );

    let anthropic_request = AnthropicProvider::build_chat_request(&request, true);
    assert_eq!(anthropic_request.stream, Some(true));
}

#[test]
fn test_convert_tools() {
    let tools = vec![crate::types::ToolDefinition::function(
        "get_weather".to_string(),
        "Get weather for a location".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": {
                "location": { "type": "string" }
            },
            "required": ["location"]
        }),
    )];

    let anthropic_tools = AnthropicProvider::convert_tools(&tools);
    assert_eq!(anthropic_tools.len(), 1);
    assert_eq!(anthropic_tools[0].name, "get_weather");
    assert_eq!(anthropic_tools[0].description, "Get weather for a location");
    assert_eq!(
        anthropic_tools[0].input_schema["properties"]["location"]["type"],
        "string"
    );
}

#[tokio::test]
async fn test_embedding_not_supported() {
    let provider = AnthropicProvider::new("test-key");
    let result = provider.generate_embedding("test text", "some-model").await;

    assert!(matches!(
        result,
        Err(ProviderError::UnsupportedOperation(_))
    ));
}

// --- Structured output (tool-injection pattern) tests ---

#[test]
fn test_apply_response_format_text_is_noop() {
    let mut tools = None;
    let mut tool_choice = None;

    AnthropicProvider::apply_response_format(
        Some(&ResponseFormat::Text),
        &mut tools,
        &mut tool_choice,
    );

    assert!(tools.is_none());
    assert!(tool_choice.is_none());
}

#[test]
fn test_apply_response_format_json_object_is_noop() {
    let mut tools = None;
    let mut tool_choice = None;

    AnthropicProvider::apply_response_format(
        Some(&ResponseFormat::JsonObject),
        &mut tools,
        &mut tool_choice,
    );

    // Anthropic doesn't have a native json_object mode
    assert!(tools.is_none());
    assert!(tool_choice.is_none());
}

#[test]
fn test_apply_response_format_json_schema_injects_tool() {
    let schema = ResponseFormat::JsonSchema {
        schema: JsonSchemaSpec {
            name: Some("keywords".to_string()),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "keywords": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["keywords"]
            }),
            strict: true,
        },
    };

    let mut tools = None;
    let mut tool_choice = None;

    AnthropicProvider::apply_response_format(Some(&schema), &mut tools, &mut tool_choice);

    // Tool should be injected
    let tools = tools.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "keywords");
    assert_eq!(
        tools[0].input_schema["properties"]["keywords"]["type"],
        "array"
    );

    // tool_choice should force the specific tool
    let tc = tool_choice.unwrap();
    let serialized = serde_json::to_value(&tc).unwrap();
    assert_eq!(serialized["type"], "tool");
    assert_eq!(serialized["name"], "keywords");
}

#[test]
fn test_apply_response_format_json_schema_uses_default_name() {
    let schema = ResponseFormat::JsonSchema {
        schema: JsonSchemaSpec {
            name: None,
            schema: serde_json::json!({"type": "object"}),
            strict: false,
        },
    };

    let mut tools = None;
    let mut tool_choice = None;

    AnthropicProvider::apply_response_format(Some(&schema), &mut tools, &mut tool_choice);

    let tools = tools.unwrap();
    assert_eq!(tools[0].name, "__structured_output");
}

#[test]
fn test_apply_response_format_json_schema_appends_to_existing_tools() {
    let schema = ResponseFormat::JsonSchema {
        schema: JsonSchemaSpec {
            name: Some("output".to_string()),
            schema: serde_json::json!({"type": "object"}),
            strict: false,
        },
    };

    let mut tools = Some(vec![AnthropicTool {
        name: "existing_tool".to_string(),
        description: "A user tool".to_string(),
        input_schema: serde_json::json!({}),
    }]);
    let mut tool_choice = None;

    AnthropicProvider::apply_response_format(Some(&schema), &mut tools, &mut tool_choice);

    let tools = tools.unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name, "existing_tool");
    assert_eq!(tools[1].name, "output");
}

#[test]
fn test_extract_structured_output_moves_tool_call_to_content() {
    let schema = ResponseFormat::JsonSchema {
        schema: JsonSchemaSpec {
            name: Some("keywords".to_string()),
            schema: serde_json::json!({}),
            strict: false,
        },
    };

    let mut response = CompletionResponse {
        message: Message {
            role: Role::Assistant,
            content: String::new(),
            content_parts: None,
            tool_calls: Some(vec![ToolCall {
                id: "call_123".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "keywords".to_string(),
                    arguments: r#"{"keywords":["rust","async"]}"#.to_string(),
                },
                index: None,
            }]),
            tool_call_id: None,
            name: None,
        },
        model: "claude-sonnet-4-5".to_string(),
        usage: None,
        stop_reason: Some("tool_use".to_string()),
    };

    AnthropicProvider::extract_structured_output(&mut response, Some(&schema));

    assert_eq!(response.message.content, r#"{"keywords":["rust","async"]}"#);
    assert!(response.message.tool_calls.is_none());
    assert_eq!(response.stop_reason.as_deref(), Some("stop"));
}

#[test]
fn test_extract_structured_output_noop_without_schema() {
    let mut response = CompletionResponse {
        message: Message {
            role: Role::Assistant,
            content: "Hello".to_string(),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
        model: "claude-sonnet-4-5".to_string(),
        usage: None,
        stop_reason: Some("end_turn".to_string()),
    };

    AnthropicProvider::extract_structured_output(&mut response, None);
    assert_eq!(response.message.content, "Hello");
}

// --- SSE streaming parser tests ---

#[test]
fn test_parse_sse_text_delta() {
    let sse = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_abc\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-5\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":0,\"output_tokens\":5}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n";

    let chunks = parse_anthropic_sse_events(sse);

    // message_start -> text delta "Hello" -> text delta " world" -> message_delta (stop)
    // content_block_start for text is skipped, message_stop is skipped
    assert!(chunks.len() >= 3);

    // First chunk: message_start with model info
    let c0 = chunks[0].as_ref().unwrap();
    assert_eq!(c0.model.as_deref(), Some("claude-sonnet-4-5"));
    assert!(c0.usage.is_some());

    // Find text deltas
    let text_chunks: Vec<_> = chunks
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter(|c| !c.delta.is_empty())
        .collect();

    assert_eq!(text_chunks.len(), 2);
    assert_eq!(text_chunks[0].delta, "Hello");
    assert_eq!(text_chunks[1].delta, " world");

    // Last meaningful chunk should have stop_reason
    let stop_chunk = chunks
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .find(|c| c.stop_reason.is_some());
    assert!(stop_chunk.is_some());
    assert_eq!(stop_chunk.unwrap().stop_reason.as_deref(), Some("end_turn"));
}

#[test]
fn test_parse_sse_tool_use() {
    let sse = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_abc\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-5\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":20,\"output_tokens\":0}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_xyz\",\"name\":\"get_weather\",\"input\":{}}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"loc\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"ation\\\":\\\"Paris\\\"}\"}}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"input_tokens\":0,\"output_tokens\":15}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n";

    let chunks = parse_anthropic_sse_events(sse);

    // Find tool call start
    let tool_start = chunks.iter().filter_map(|r| r.as_ref().ok()).find(|c| {
        c.tool_calls
            .as_ref()
            .is_some_and(|tc| !tc.is_empty() && !tc[0].id.is_empty())
    });
    assert!(tool_start.is_some());
    let tc = &tool_start.unwrap().tool_calls.as_ref().unwrap()[0];
    assert_eq!(tc.id, "toolu_xyz");
    assert_eq!(tc.function.name, "get_weather");

    // Find argument deltas
    let arg_chunks: Vec<_> = chunks
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter(|c| {
            c.tool_calls
                .as_ref()
                .is_some_and(|tc| !tc.is_empty() && !tc[0].function.arguments.is_empty())
        })
        .collect();

    assert_eq!(arg_chunks.len(), 2);
    assert_eq!(
        arg_chunks[0].tool_calls.as_ref().unwrap()[0]
            .function
            .arguments,
        "{\"loc"
    );
    assert_eq!(
        arg_chunks[1].tool_calls.as_ref().unwrap()[0]
            .function
            .arguments,
        "ation\":\"Paris\"}"
    );

    // Stop reason
    let stop_chunk = chunks
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .find(|c| c.stop_reason.is_some());
    assert!(stop_chunk.is_some());
    assert_eq!(stop_chunk.unwrap().stop_reason.as_deref(), Some("tool_use"));
}

#[test]
fn test_parse_sse_empty_input() {
    let chunks = parse_anthropic_sse_events("");
    assert!(chunks.is_empty());
}

#[test]
fn test_parse_sse_skips_unknown_events() {
    let sse = "\
event: ping\n\
data: {}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\
\n\
data: [DONE]\n";

    let chunks = parse_anthropic_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].as_ref().unwrap().delta, "ok");
}

#[test]
fn test_tool_choice_serialization() {
    // Mode variant
    let mode = AnthropicToolChoice::Mode(AnthropicToolChoiceMode {
        choice_type: "auto".to_string(),
    });
    let json = serde_json::to_value(&mode).unwrap();
    assert_eq!(json["type"], "auto");

    // Specific variant
    let specific = AnthropicToolChoice::Specific(AnthropicToolChoiceSpecific {
        choice_type: "tool".to_string(),
        name: "my_tool".to_string(),
    });
    let json = serde_json::to_value(&specific).unwrap();
    assert_eq!(json["type"], "tool");
    assert_eq!(json["name"], "my_tool");
}

#[test]
fn test_known_models_not_empty() {
    let models = AnthropicProvider::get_known_models();
    assert!(!models.is_empty());

    // All models should have chat and tools capabilities
    for model in &models {
        assert!(model.capabilities.chat);
        assert!(model.capabilities.tools);
        assert!(model.capabilities.streaming);
        assert!(!model.capabilities.embeddings);
    }

    // Check that we have both opus and sonnet tiers
    let has_opus = models.iter().any(|m| m.id.contains("opus"));
    let has_sonnet = models.iter().any(|m| m.id.contains("sonnet"));
    let has_haiku = models.iter().any(|m| m.id.contains("haiku"));
    assert!(has_opus);
    assert!(has_sonnet);
    assert!(has_haiku);
}
