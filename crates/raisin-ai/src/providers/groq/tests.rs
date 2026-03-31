//! Tests for the Groq provider.

use super::trait_impl::parse_groq_sse_events;
use super::*;
use crate::provider::{AIProviderTrait, ProviderError};
use crate::types::{
    CompletionResponse, FunctionCall, JsonSchemaSpec, ResponseFormat, Role, ToolCall,
};

#[test]
fn test_validate_chat_model() {
    assert!(GroqProvider::validate_chat_model("llama-3.3-70b-versatile").is_ok());
    assert!(GroqProvider::validate_chat_model("llama-3.1-8b-instant").is_ok());
    assert!(GroqProvider::validate_chat_model("mixtral-8x7b-32768").is_ok());
    assert!(GroqProvider::validate_chat_model("gemma2-9b-it").is_ok());
    assert!(GroqProvider::validate_chat_model("invalid-model").is_err());
}

#[test]
fn test_provider_capabilities() {
    let provider = GroqProvider::new("test-key");
    assert_eq!(provider.provider_name(), "groq");
    assert!(provider.supports_streaming());
    assert!(provider.supports_tools());
    assert!(!provider.available_models().is_empty());
}

#[test]
fn test_convert_message() {
    let user_msg = Message::user("Hello");
    let groq_msg = GroqProvider::convert_message(&user_msg);
    assert_eq!(groq_msg.role, "user");
    assert_eq!(groq_msg.content, Some("Hello".to_string()));

    let assistant_msg = Message::assistant("Hi there");
    let groq_msg = GroqProvider::convert_message(&assistant_msg);
    assert_eq!(groq_msg.role, "assistant");
    assert_eq!(groq_msg.content, Some("Hi there".to_string()));

    let system_msg = Message::system("You are helpful");
    let groq_msg = GroqProvider::convert_message(&system_msg);
    assert_eq!(groq_msg.role, "system");
    assert_eq!(groq_msg.content, Some("You are helpful".to_string()));
}

#[test]
fn test_convert_message_with_tool_calls() {
    let tool_call = ToolCall {
        id: "call_123".to_string(),
        call_type: "function".to_string(),
        function: FunctionCall {
            name: "get_weather".to_string(),
            arguments: r#"{"location": "Paris"}"#.to_string(),
        },
        index: None,
    };

    let assistant_msg = Message::assistant("").with_tool_calls(vec![tool_call]);
    let groq_msg = GroqProvider::convert_message(&assistant_msg);

    assert_eq!(groq_msg.role, "assistant");
    assert!(groq_msg.content.is_none());
    assert!(groq_msg.tool_calls.is_some());

    let tool_calls = groq_msg.tool_calls.unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].id, "call_123");
    assert_eq!(tool_calls[0].function.name, "get_weather");
}

#[test]
fn test_convert_tool_message() {
    let tool_msg = Message::tool(
        r#"{"temperature": 20}"#,
        "call_123".to_string(),
        Some("get_weather".to_string()),
    );
    let groq_msg = GroqProvider::convert_message(&tool_msg);

    assert_eq!(groq_msg.role, "tool");
    assert_eq!(groq_msg.content, Some(r#"{"temperature": 20}"#.to_string()));
    assert_eq!(groq_msg.tool_call_id, Some("call_123".to_string()));
    assert_eq!(groq_msg.name, Some("get_weather".to_string()));
}

#[tokio::test]
async fn test_embedding_not_supported() {
    let provider = GroqProvider::new("test-key");
    let result = provider.generate_embedding("test text", "some-model").await;

    assert!(matches!(
        result,
        Err(ProviderError::UnsupportedOperation(_))
    ));
}

#[test]
fn test_model_conversion() {
    let provider = GroqProvider::new("test-key");

    // Test Llama 3.3 model with extended context
    let llama_model = GroqModel {
        id: "llama-3.3-70b-versatile".to_string(),
        created: 1234567890,
        owned_by: "Meta".to_string(),
        active: Some(true),
    };

    let model_info = provider.convert_groq_model(llama_model);
    assert_eq!(model_info.id, "llama-3.3-70b-versatile");
    assert_eq!(model_info.context_window, Some(128000));
    assert!(model_info.capabilities.chat);
    assert!(model_info.capabilities.streaming);
    assert!(model_info.capabilities.tools);
    assert!(!model_info.capabilities.embeddings);
    assert!(!model_info.capabilities.vision);

    // Test Mixtral model with 32K context
    let mixtral_model = GroqModel {
        id: "mixtral-8x7b-32768".to_string(),
        created: 1234567890,
        owned_by: "Mistral".to_string(),
        active: Some(true),
    };

    let model_info = provider.convert_groq_model(mixtral_model);
    assert_eq!(model_info.context_window, Some(32768));
}

// --- SSE streaming parser tests ---

#[test]
fn test_parse_sse_text_delta() {
    let sse = r#"data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}],"model":"llama-3.3-70b-versatile"}

data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}],"model":"llama-3.3-70b-versatile"}

data: [DONE]
"#;

    let chunks = parse_groq_sse_events(sse);
    assert_eq!(chunks.len(), 2);

    let c0 = chunks[0].as_ref().unwrap();
    assert_eq!(c0.delta, "Hello");
    assert!(c0.tool_calls.is_none());
    assert!(c0.usage.is_none());
    assert!(c0.stop_reason.is_none());

    let c1 = chunks[1].as_ref().unwrap();
    assert_eq!(c1.delta, " world");
}

#[test]
fn test_parse_sse_finish_with_usage() {
    let sse = r#"data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}],"model":"llama-3.3-70b-versatile"}

data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15},"model":"llama-3.3-70b-versatile"}

data: [DONE]
"#;

    let chunks = parse_groq_sse_events(sse);
    assert_eq!(chunks.len(), 2);

    // First chunk is text
    let c0 = chunks[0].as_ref().unwrap();
    assert_eq!(c0.delta, "Hi");

    // Second chunk is the final chunk with stop reason and usage
    let c1 = chunks[1].as_ref().unwrap();
    assert_eq!(c1.stop_reason.as_deref(), Some("stop"));
    assert_eq!(c1.model.as_deref(), Some("llama-3.3-70b-versatile"));
    let usage = c1.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 15);
}

#[test]
fn test_parse_sse_tool_call() {
    let sse = r#"data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_xyz","type":"function","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}],"model":"llama-3.3-70b-versatile"}

data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"loc"}}]},"finish_reason":null}],"model":"llama-3.3-70b-versatile"}

data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ation\":\"Paris\"}"}}]},"finish_reason":null}],"model":"llama-3.3-70b-versatile"}

data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":20,"completion_tokens":15,"total_tokens":35},"model":"llama-3.3-70b-versatile"}

data: [DONE]
"#;

    let chunks = parse_groq_sse_events(sse);
    assert_eq!(chunks.len(), 4);

    // First chunk: tool call start with id and name
    let c0 = chunks[0].as_ref().unwrap();
    assert!(c0.delta.is_empty());
    let tc = &c0.tool_calls.as_ref().unwrap()[0];
    assert_eq!(tc.id, "call_xyz");
    assert_eq!(tc.function.name, "get_weather");

    // Second and third: argument deltas
    let c1 = chunks[1].as_ref().unwrap();
    let tc1 = &c1.tool_calls.as_ref().unwrap()[0];
    assert_eq!(tc1.function.arguments, "{\"loc");

    let c2 = chunks[2].as_ref().unwrap();
    let tc2 = &c2.tool_calls.as_ref().unwrap()[0];
    assert_eq!(tc2.function.arguments, "ation\":\"Paris\"}");

    // Final chunk: stop reason
    let c3 = chunks[3].as_ref().unwrap();
    assert_eq!(c3.stop_reason.as_deref(), Some("tool_calls"));
}

#[test]
fn test_parse_sse_empty_input() {
    let chunks = parse_groq_sse_events("");
    assert!(chunks.is_empty());
}

#[test]
fn test_parse_sse_ignores_non_data_lines() {
    let sse = ": comment line\nevent: some_event\nid: 123\nretry: 5000\n\ndata: {\"id\":\"abc\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}],\"model\":\"m\"}\n\ndata: [DONE]\n";

    let chunks = parse_groq_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].as_ref().unwrap().delta, "ok");
}

#[test]
fn test_parse_sse_skips_malformed_json() {
    let sse = "data: {broken json}\n\ndata: {\"id\":\"abc\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"valid\"},\"finish_reason\":null}],\"model\":\"m\"}\n\ndata: [DONE]\n";

    let chunks = parse_groq_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].as_ref().unwrap().delta, "valid");
}

// --- Structured output (tool-injection pattern) tests ---

#[test]
fn test_apply_response_format_text_is_noop() {
    let mut response_format = None;
    let mut tools = None;
    let mut tool_choice = None;

    GroqProvider::apply_response_format(
        Some(&ResponseFormat::Text),
        &mut response_format,
        &mut tools,
        &mut tool_choice,
    );

    assert!(response_format.is_none());
    assert!(tools.is_none());
    assert!(tool_choice.is_none());
}

#[test]
fn test_apply_response_format_json_object() {
    let mut response_format = None;
    let mut tools = None;
    let mut tool_choice = None;

    GroqProvider::apply_response_format(
        Some(&ResponseFormat::JsonObject),
        &mut response_format,
        &mut tools,
        &mut tool_choice,
    );

    assert!(response_format.is_some());
    assert_eq!(response_format.unwrap().format_type, "json_object");
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

    let mut response_format = None;
    let mut tools = None;
    let mut tool_choice = None;

    GroqProvider::apply_response_format(
        Some(&schema),
        &mut response_format,
        &mut tools,
        &mut tool_choice,
    );

    // No json_object format set for JsonSchema mode
    assert!(response_format.is_none());

    // Tool should be injected
    let tools = tools.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].function.name, "keywords");
    assert_eq!(
        tools[0].function.parameters.as_ref().unwrap()["properties"]["keywords"]["type"],
        "array"
    );

    // tool_choice should force the specific tool
    let tc = tool_choice.unwrap();
    let serialized = serde_json::to_value(&tc).unwrap();
    assert_eq!(serialized["type"], "function");
    assert_eq!(serialized["function"]["name"], "keywords");
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

    let mut response_format = None;
    let mut tools = None;
    let mut tool_choice = None;

    GroqProvider::apply_response_format(
        Some(&schema),
        &mut response_format,
        &mut tools,
        &mut tool_choice,
    );

    let tools = tools.unwrap();
    assert_eq!(tools[0].function.name, "__structured_output");
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

    let mut response_format = None;
    let mut tools = Some(vec![GroqToolDefinition {
        tool_type: "function".to_string(),
        function: GroqFunctionDefinition {
            name: "existing_tool".to_string(),
            description: Some("A user tool".to_string()),
            parameters: Some(serde_json::json!({})),
        },
    }]);
    let mut tool_choice = None;

    GroqProvider::apply_response_format(
        Some(&schema),
        &mut response_format,
        &mut tools,
        &mut tool_choice,
    );

    let tools = tools.unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].function.name, "existing_tool");
    assert_eq!(tools[1].function.name, "output");
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
        model: "llama-3.3-70b-versatile".to_string(),
        usage: None,
        stop_reason: Some("tool_calls".to_string()),
    };

    GroqProvider::extract_structured_output(&mut response, Some(&schema));

    assert_eq!(response.message.content, r#"{"keywords":["rust","async"]}"#);
    assert!(response.message.tool_calls.is_none());
    assert_eq!(response.stop_reason.as_deref(), Some("stop"));
}

#[test]
fn test_extract_structured_output_preserves_other_tool_calls() {
    let schema = ResponseFormat::JsonSchema {
        schema: JsonSchemaSpec {
            name: Some("structured".to_string()),
            schema: serde_json::json!({}),
            strict: false,
        },
    };

    let mut response = CompletionResponse {
        message: Message {
            role: Role::Assistant,
            content: String::new(),
            content_parts: None,
            tool_calls: Some(vec![
                ToolCall {
                    id: "call_1".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: "user_tool".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                },
                ToolCall {
                    id: "call_2".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: "structured".to_string(),
                        arguments: r#"{"result":"ok"}"#.to_string(),
                    },
                    index: None,
                },
            ]),
            tool_call_id: None,
            name: None,
        },
        model: "llama-3.3-70b-versatile".to_string(),
        usage: None,
        stop_reason: Some("tool_calls".to_string()),
    };

    GroqProvider::extract_structured_output(&mut response, Some(&schema));

    assert_eq!(response.message.content, r#"{"result":"ok"}"#);
    let remaining = response.message.tool_calls.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].function.name, "user_tool");
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
        model: "llama-3.3-70b-versatile".to_string(),
        usage: None,
        stop_reason: Some("stop".to_string()),
    };

    GroqProvider::extract_structured_output(&mut response, None);
    assert_eq!(response.message.content, "Hello");
}

#[test]
fn test_tool_choice_serialization() {
    // Mode variant
    let mode = GroqToolChoice::Mode("auto".to_string());
    let json = serde_json::to_value(&mode).unwrap();
    assert_eq!(json, "auto");

    // Specific variant
    let specific = GroqToolChoice::Specific(GroqToolChoiceSpecific {
        choice_type: "function".to_string(),
        function: GroqToolChoiceFunction {
            name: "my_tool".to_string(),
        },
    });
    let json = serde_json::to_value(&specific).unwrap();
    assert_eq!(json["type"], "function");
    assert_eq!(json["function"]["name"], "my_tool");
}
