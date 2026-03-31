//! Shared structured output extraction for AI providers.
//!
//! When a provider does not natively support `JsonSchema` response format,
//! a synthetic tool is injected to force structured output. After the model
//! responds with a tool call to that synthetic tool, its JSON payload needs
//! to be moved into `message.content` for a uniform response shape.
//!
//! This module contains the shared extraction logic used by Anthropic and Groq.

use crate::types::{CompletionResponse, ResponseFormat};

/// Default name for the synthetic tool when no explicit name is provided.
pub const STRUCTURED_OUTPUT_TOOL: &str = "__structured_output";

/// Checks whether the response contains a structured output tool call
/// and, if so, moves its JSON payload into `message.content` so callers
/// get a uniform response shape.
///
/// The `expected_name` is determined from the schema's `name` field,
/// falling back to [`STRUCTURED_OUTPUT_TOOL`].
pub fn extract_structured_output(
    response: &mut CompletionResponse,
    response_format: Option<&ResponseFormat>,
) {
    let Some(ResponseFormat::JsonSchema { schema }) = response_format else {
        return;
    };

    let expected_name = schema.name.as_deref().unwrap_or(STRUCTURED_OUTPUT_TOOL);

    let Some(tool_calls) = response.message.tool_calls.take() else {
        return;
    };

    let mut remaining = Vec::new();
    let mut found = false;

    for call in tool_calls {
        if !found && call.function.name == expected_name {
            response.message.content = call.function.arguments;
            found = true;
        } else {
            remaining.push(call);
        }
    }

    if !remaining.is_empty() {
        response.message.tool_calls = Some(remaining);
    }

    if found {
        // Normalize stop_reason to indicate normal completion
        response.stop_reason = Some("stop".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FunctionCall, JsonSchemaSpec, Message, Role, ToolCall};

    #[test]
    fn test_extract_moves_tool_call_to_content() {
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
            model: "test-model".to_string(),
            usage: None,
            stop_reason: Some("tool_use".to_string()),
        };

        extract_structured_output(&mut response, Some(&schema));

        assert_eq!(response.message.content, r#"{"keywords":["rust","async"]}"#);
        assert!(response.message.tool_calls.is_none());
        assert_eq!(response.stop_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn test_extract_preserves_other_tool_calls() {
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
            model: "test-model".to_string(),
            usage: None,
            stop_reason: Some("tool_use".to_string()),
        };

        extract_structured_output(&mut response, Some(&schema));

        assert_eq!(response.message.content, r#"{"result":"ok"}"#);
        let remaining = response.message.tool_calls.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].function.name, "user_tool");
    }

    #[test]
    fn test_extract_noop_without_schema() {
        let mut response = CompletionResponse {
            message: Message {
                role: Role::Assistant,
                content: "Hello".to_string(),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            model: "test-model".to_string(),
            usage: None,
            stop_reason: Some("stop".to_string()),
        };

        extract_structured_output(&mut response, None);
        assert_eq!(response.message.content, "Hello");
    }

    #[test]
    fn test_extract_uses_default_name() {
        let schema = ResponseFormat::JsonSchema {
            schema: JsonSchemaSpec {
                name: None,
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
                    id: "call_1".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: STRUCTURED_OUTPUT_TOOL.to_string(),
                        arguments: r#"{"data":true}"#.to_string(),
                    },
                    index: None,
                }]),
                tool_call_id: None,
                name: None,
            },
            model: "test-model".to_string(),
            usage: None,
            stop_reason: Some("tool_use".to_string()),
        };

        extract_structured_output(&mut response, Some(&schema));
        assert_eq!(response.message.content, r#"{"data":true}"#);
        assert_eq!(response.stop_reason.as_deref(), Some("stop"));
    }
}
