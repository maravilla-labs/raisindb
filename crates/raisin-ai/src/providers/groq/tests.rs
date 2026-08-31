//! Tests for the Groq provider.

use super::trait_impl::parse_groq_sse_events;
use super::*;
use crate::provider::{AIProviderTrait, ProviderError};
use crate::types::{
    CompletionResponse, FunctionCall, JsonSchemaSpec, ResponseFormat, Role, ToolCall,
};

#[test]
fn test_validate_chat_model() {
    // Any non-empty model name is accepted — Groq's API is the source of truth,
    // including newer models that no hardcoded allowlist would know about.
    assert!(GroqProvider::validate_chat_model("llama-3.3-70b-versatile").is_ok());
    assert!(GroqProvider::validate_chat_model("allam-2-7b").is_ok());
    assert!(GroqProvider::validate_chat_model("qwen/qwen3-32b").is_ok());
    // Only an empty/blank name is rejected locally.
    assert!(GroqProvider::validate_chat_model("").is_err());
    assert!(GroqProvider::validate_chat_model("   ").is_err());
}

#[test]
fn test_groq_model_supports_tools() {
    // Chat / LLM models support tool use (per Groq's docs).
    assert!(super::groq_model_supports_tools("llama-3.3-70b-versatile"));
    assert!(super::groq_model_supports_tools("allam-2-7b"));
    assert!(super::groq_model_supports_tools("qwen/qwen3-32b"));
    assert!(super::groq_model_supports_tools("openai/gpt-oss-120b"));
    assert!(super::groq_model_supports_tools("gemma2-9b-it"));
    assert!(super::groq_model_supports_tools("groq/compound"));
    // STT / TTS / moderation models do not.
    assert!(!super::groq_model_supports_tools("whisper-large-v3"));
    assert!(!super::groq_model_supports_tools("whisper-large-v3-turbo"));
    assert!(!super::groq_model_supports_tools(
        "canopylabs/orpheus-arabic-saudi"
    ));
    assert!(!super::groq_model_supports_tools("playai-tts"));
    assert!(!super::groq_model_supports_tools(
        "meta-llama/llama-prompt-guard-2-22m"
    ));
    assert!(!super::groq_model_supports_tools(
        "openai/gpt-oss-safeguard-20b"
    ));
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
        kind: None,
        dimensions: None,
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
        kind: None,
        dimensions: None,
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
fn test_parse_sse_finish_with_x_groq_usage() {
    // Real Groq streaming shape: usage is nested under x_groq on the final chunk
    let sse = r#"data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"model":"llama-3.1-8b-instant","x_groq":{"id":"req_123","usage":{"prompt_tokens":42,"completion_tokens":7,"total_tokens":49}}}

data: [DONE]
"#;

    let chunks = parse_groq_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    let c = chunks[0].as_ref().unwrap();
    assert_eq!(c.stop_reason.as_deref(), Some("stop"));
    let usage = c.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 42);
    assert_eq!(usage.completion_tokens, 7);
    assert_eq!(usage.total_tokens, 49);
}

#[test]
fn test_parse_sse_usage_only_trailing_chunk() {
    // OpenAI stream_options.include_usage style: trailing chunk with empty choices
    let sse = r#"data: {"id":"chatcmpl-abc","choices":[],"usage":{"prompt_tokens":11,"completion_tokens":3,"total_tokens":14},"model":"llama-3.1-8b-instant"}

data: [DONE]
"#;

    let chunks = parse_groq_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    let c = chunks[0].as_ref().unwrap();
    assert!(c.stop_reason.is_none());
    let usage = c.usage.as_ref().unwrap();
    assert_eq!(usage.total_tokens, 14);
}

#[test]
fn test_parse_sse_usage_on_content_chunk_without_finish() {
    // Usage attached to a chunk whose choice has content but no finish_reason:
    // both the text delta and a usage-only chunk must be emitted.
    let sse = r#"data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{"content":"done"},"finish_reason":null}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7},"model":"llama-3.1-8b-instant"}

data: [DONE]
"#;

    let chunks = parse_groq_sse_events(sse);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].as_ref().unwrap().delta, "done");
    let usage = chunks[1].as_ref().unwrap().usage.as_ref().unwrap();
    assert_eq!(usage.total_tokens, 7);
}

#[test]
fn test_parse_sse_usage_on_finish_chunk_not_duplicated() {
    // Usage on the finish chunk itself must be captured exactly once.
    let sse = r#"data: {"id":"chatcmpl-abc","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15},"model":"llama-3.1-8b-instant"}

data: [DONE]
"#;

    let chunks = parse_groq_sse_events(sse);
    assert_eq!(chunks.len(), 1);
    let c = chunks[0].as_ref().unwrap();
    assert_eq!(c.stop_reason.as_deref(), Some("stop"));
    assert_eq!(c.usage.as_ref().unwrap().total_tokens, 15);
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

// ── forced-tool structured output: recovery when the model answers in content ──
//
// Groq gets JSON-schema output by injecting a synthetic tool and pinning
// `tool_choice` to it. A model that replies in CONTENT instead makes the API
// reject the whole request, stranding a perfectly good answer in the error's
// `failed_generation`. Every `ai_agent` flow step asks for a schema, so this was
// a hard failure for all of them.
//
// These drive the real `complete()` against a stub that speaks the exact bytes
// Groq returns — no mocking library, and no API key, so they run in CI. What
// they pin is the behaviour a unit test of the helper cannot: that `complete()`
// RECOVERS, that the retry is actually sent, and that the retry drops the forced
// tool rather than repeating the request that just failed.
mod forced_tool_recovery {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// The 400 Groq returns when the model did not call the pinned tool.
    fn tool_choice_error(failed_generation: Option<&str>) -> String {
        let detail = match failed_generation {
            Some(g) => format!(
                r#","failed_generation":{}"#,
                serde_json::to_string(g).unwrap()
            ),
            None => String::new(),
        };
        format!(
            r#"{{"error":{{"message":"Tool choice is required, but model did not call a tool","type":"invalid_request_error"{detail}}}}}"#
        )
    }

    /// A normal Groq chat completion carrying `content`.
    fn ok_completion(content: &str) -> String {
        format!(
            r#"{{"id":"x","object":"chat.completion","created":0,"model":"stub","choices":[{{"index":0,"message":{{"role":"assistant","content":{}}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}}}"#,
            serde_json::to_string(content).unwrap()
        )
    }

    /// Serve `responses` in order, one per request, recording each request body.
    /// Returns the base URL and the shared record of what was received.
    async fn stub(responses: Vec<(u16, String)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);

        tokio::spawn(async move {
            for (status, body) in responses {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                // Read headers, then exactly Content-Length bytes of body. A
                // short read here would race the assertion on the request body.
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    let Some(head_end) = text.find("\r\n\r\n") else {
                        continue;
                    };
                    let len: usize = text[..head_end]
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse().ok())?
                        })
                        .unwrap_or(0);
                    if buf.len() >= head_end + 4 + len {
                        recorder
                            .lock()
                            .unwrap()
                            .push(text[head_end + 4..].to_string());
                        break;
                    }
                }
                let reason = if status == 200 { "OK" } else { "Bad Request" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });

        (format!("http://{addr}"), seen)
    }

    fn schema_request(base: &str) -> (GroqProvider, CompletionRequest) {
        let provider = GroqProvider::with_base_url("test-key", base.to_string());
        let mut request = CompletionRequest::new(
            "openai/gpt-oss-20b".to_string(),
            vec![Message::user("write three variants")],
        );
        request.response_format = Some(ResponseFormat::JsonSchema {
            schema: JsonSchemaSpec {
                name: Some("copy_variants".to_string()),
                schema: serde_json::json!({"type":"object"}),
                strict: true,
            },
        });
        (provider, request)
    }

    const ANSWER: &str = r#"{"variants":[{"text":"Spring cohort opens","angle":"direct","rationale":"x"}],"notes":""}"#;

    #[tokio::test]
    async fn salvages_the_answer_stranded_in_failed_generation() {
        let (base, seen) = stub(vec![(400, tool_choice_error(Some(ANSWER)))]).await;
        let (provider, request) = schema_request(&base);

        let response = provider.complete(request).await.expect("should recover");

        assert_eq!(response.message.content, ANSWER);
        assert_eq!(response.stop_reason.as_deref(), Some("stop"));
        // One request only — a salvageable answer must not cost a second call.
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn retries_in_json_object_mode_when_there_is_nothing_to_salvage() {
        let (base, seen) = stub(vec![
            (400, tool_choice_error(None)),
            (200, ok_completion(ANSWER)),
        ])
        .await;
        let (provider, request) = schema_request(&base);

        let response = provider.complete(request).await.expect("should recover");
        assert_eq!(response.message.content, ANSWER);

        let sent = seen.lock().unwrap().clone();
        assert_eq!(sent.len(), 2, "the retry was not sent");
        let first: serde_json::Value = serde_json::from_str(&sent[0]).unwrap();
        let second: serde_json::Value = serde_json::from_str(&sent[1]).unwrap();
        // The first attempt is the forced synthetic tool…
        assert_eq!(first["tool_choice"]["function"]["name"], "copy_variants");
        // …and the retry must NOT repeat it, or it fails exactly the same way.
        assert!(second.get("tool_choice").is_none_or(|v| v.is_null()));
        assert!(second.get("tools").is_none_or(|v| v.is_null()));
        assert_eq!(second["response_format"]["type"], "json_object");
    }

    #[tokio::test]
    async fn a_fenced_answer_is_still_salvaged() {
        let fenced = format!("```json\n{ANSWER}\n```");
        let (base, _) = stub(vec![(400, tool_choice_error(Some(&fenced)))]).await;
        let (provider, request) = schema_request(&base);

        let response = provider.complete(request).await.expect("should recover");
        assert_eq!(response.message.content, ANSWER);
    }

    #[tokio::test]
    async fn prose_is_not_passed_off_as_an_answer() {
        // The guarantee that makes salvage safe: a caller cannot distinguish a
        // recovered response from a normal one, so anything that is not JSON has
        // to stay an error. Here the retry ALSO fails, so the original error is
        // what the caller sees.
        let (base, _) = stub(vec![
            (
                400,
                tool_choice_error(Some("I'm sorry, I can't help with that.")),
            ),
            (400, tool_choice_error(None)),
        ])
        .await;
        let (provider, request) = schema_request(&base);

        let err = provider.complete(request).await.unwrap_err();
        let ProviderError::RequestFailed(msg) = err else {
            panic!("wrong error variant");
        };
        assert!(msg.contains("Tool choice is required"), "{msg}");
    }

    #[tokio::test]
    async fn an_ordinary_failure_is_untouched() {
        // No schema asked for → no recovery path, no retry, no swallowed error.
        let (base, seen) = stub(vec![(
            400,
            r#"{"error":{"message":"Rate limit reached","type":"rate_limit_error"}}"#.to_string(),
        )])
        .await;
        let provider = GroqProvider::with_base_url("test-key", base);
        let request = CompletionRequest::new(
            "llama-3.3-70b-versatile".to_string(),
            vec![Message::user("hello")],
        );

        assert!(provider.complete(request).await.is_err());
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "an unrelated error was retried"
        );
    }
}

/// This client also serves `AIProvider::Custom` — every OpenAI-shaped gateway.
/// These cover the two extension keys such a gateway publishes (`kind` and
/// `dimensions`), end to end from the wire bytes to the use cases a tenant's
/// config would offer.
mod gateway_model_metadata {
    use super::*;
    use crate::config::AIUseCase;
    use crate::model_classifier::{
        classify, ClassificationContext, METADATA_EMBEDDING_LENGTH,
        METADATA_EMBEDDING_UNAVAILABLE_REASON, METADATA_KIND,
    };

    fn parse(body: &str) -> GroqModelsResponse {
        serde_json::from_str(body).expect("gateway listing must parse")
    }

    fn convert(body: &str) -> ModelInfo {
        let provider = GroqProvider::new("test-key");
        provider.convert_groq_model(parse(body).data.into_iter().next().unwrap())
    }

    const REAL_GROQ: &str = r#"{"data":[
        {"id":"llama-3.3-70b-versatile","created":1,"owned_by":"Meta","active":true}]}"#;

    #[test]
    fn a_listing_without_the_extension_keys_still_parses() {
        // Real Groq, and any gateway predating the contract.
        let parsed = parse(REAL_GROQ);
        assert_eq!(parsed.data.len(), 1);
        assert!(parsed.data[0].kind.is_none());
        assert!(parsed.data[0].dimensions.is_none());
    }

    #[test]
    fn unknown_keys_never_fail_the_listing() {
        // No struct on this hop may gain `deny_unknown_fields`: that attribute
        // is what turns an additive change on the gateway into an outage.
        let parsed = parse(
            r#"{"object":"list","data":[{"id":"maravilla/balanced","object":"model","created":0,
                "owned_by":"maravilla","tier":"standard","kind":"chat",
                "something_invented_next_year":{"a":1}}]}"#,
        );
        assert_eq!(parsed.data[0].kind.as_deref(), Some("chat"));
    }

    #[test]
    fn an_embedding_alias_arrives_with_its_width_and_is_offered() {
        let info = convert(
            r#"{"data":[{"id":"maravilla/embed-multilingual","object":"model","created":0,
                "owned_by":"maravilla","tier":"standard","kind":"embedding","dimensions":3584}]}"#,
        );

        // The gateway's declaration overrides this client's chat-by-default guess.
        assert!(info.capabilities.embeddings);
        assert!(!info.capabilities.chat);
        assert!(!info.capabilities.tools);

        let out = classify(&info, ClassificationContext::default());
        assert_eq!(out.use_cases, vec![AIUseCase::Embedding]);
        let meta = out.metadata.expect("metadata");
        assert_eq!(meta[METADATA_EMBEDDING_LENGTH], serde_json::json!(3584));
        assert_eq!(meta[METADATA_KIND], serde_json::json!("embedding"));
    }

    #[test]
    fn an_embedding_alias_without_a_width_is_listed_but_unusable() {
        let info = convert(
            r#"{"data":[{"id":"maravilla/embed-broken","object":"model","created":0,
                "owned_by":"maravilla","kind":"embedding"}]}"#,
        );

        assert!(!info.capabilities.embeddings);
        let out = classify(&info, ClassificationContext::default());
        assert!(
            out.use_cases.is_empty(),
            "a width we cannot know must not be guessed into the embedder identity"
        );
        let meta = out.metadata.expect("metadata");
        assert!(meta.get(METADATA_EMBEDDING_LENGTH).is_none());
        assert!(meta.get(METADATA_EMBEDDING_UNAVAILABLE_REASON).is_some());
        // Still listed, so an operator can see why rather than hunting a
        // model that simply does not appear.
        assert_eq!(info.id, "maravilla/embed-broken");
    }

    #[test]
    fn a_real_groq_model_is_unchanged() {
        let info = convert(REAL_GROQ);

        assert!(info.capabilities.chat);
        assert!(!info.capabilities.embeddings);
        assert!(info.capabilities.tools);
        assert!(
            info.metadata.as_ref().unwrap().get(METADATA_KIND).is_none(),
            "nothing is invented for a provider that publishes no kind"
        );
        assert_eq!(
            classify(&info, ClassificationContext::default()).use_cases,
            vec![AIUseCase::Chat, AIUseCase::Completion, AIUseCase::Agent]
        );
    }
}
