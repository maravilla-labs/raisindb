//! Shared structured output extraction for AI providers.
//!
//! When a provider does not natively support `JsonSchema` response format,
//! a synthetic tool is injected to force structured output. After the model
//! responds with a tool call to that synthetic tool, its JSON payload needs
//! to be moved into `message.content` for a uniform response shape.
//!
//! This module contains the shared extraction logic used by Anthropic and Groq.

use crate::types::{CompletionResponse, ResponseFormat};
use crate::utils::strip_markdown_fences;

/// Whether this request asked for a schema-shaped answer, i.e. whether a
/// provider that lacks native JSON-schema support injected a synthetic tool and
/// pinned `tool_choice` to it.
pub fn is_schema_request(response_format: Option<&ResponseFormat>) -> bool {
    matches!(response_format, Some(ResponseFormat::JsonSchema { .. }))
}

/// Recover the model's answer from a rejected structured-output request.
///
/// THE FAILURE THIS EXISTS FOR. A provider without native schema support gets
/// one by injecting a synthetic tool and forcing `tool_choice`. A model that
/// answers in CONTENT instead of calling that tool — which several do, reasoning
/// models especially — makes the API reject the whole request
/// ("Tool choice is required, but model did not call a tool"), and the perfectly
/// good JSON is returned only inside the error's `failed_generation`. Throwing
/// that away turns a usable answer into a hard failure for every caller that
/// asked for a schema, which is every `ai_agent` flow step.
///
/// So: pull the text back out, strip a markdown fence, and hand it over ONLY if
/// it parses as JSON. That last condition is the whole guarantee — a caller
/// receiving this cannot tell it from a normal completion, so anything that is
/// not valid JSON must stay an error rather than become a confusing one.
pub fn salvage_failed_generation(error_msg: &str) -> Option<String> {
    let marker = crate::providers::http_helpers::FAILED_GENERATION_MARKER;
    let at = error_msg.find(marker)?;
    let raw = error_msg[at + marker.len()..].trim();
    if raw.is_empty() {
        return None;
    }
    let cleaned = strip_markdown_fences(raw);
    let cleaned = cleaned.trim();
    serde_json::from_str::<serde_json::Value>(cleaned).ok()?;
    Some(cleaned.to_string())
}

/// Default name for the synthetic tool when no explicit name is provided.
pub const STRUCTURED_OUTPUT_TOOL: &str = "__structured_output";

/// Checks whether the response contains a structured output tool call
/// and, if so, moves its JSON payload into `message.content` so callers
/// get a uniform response shape.
///
/// The `expected_name` is determined from the schema's `name` field,
/// falling back to [`STRUCTURED_OUTPUT_TOOL`].
///
/// Returns whether the expected tool call was found. A `false` here is the
/// interesting case: a schema was requested and the model answered some other
/// way, which is only safe if what it answered with happens to be JSON — see
/// [`structured_output_missing`].
pub fn extract_structured_output(
    response: &mut CompletionResponse,
    response_format: Option<&ResponseFormat>,
) -> bool {
    let Some(ResponseFormat::JsonSchema { schema }) = response_format else {
        return false;
    };

    let expected_name = schema.name.as_deref().unwrap_or(STRUCTURED_OUTPUT_TOOL);

    let Some(tool_calls) = response.message.tool_calls.take() else {
        return false;
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

    found
}

/// Did a schema request come back as something the caller can parse?
///
/// THE SILENT FAILURE THIS NAMES. A provider without native schema support gets
/// one by forcing a synthetic tool call. Groq's API rejects the request when the
/// model answers in content instead, which is loud and recoverable
/// ([`salvage_failed_generation`]). Anthropic's does not: a model that ignores
/// the forced tool simply returns prose, `extract_structured_output` finds no
/// tool call to move, and the prose sits in `message.content` looking exactly
/// like a successful completion. The caller's `JSON.parse` then fails somewhere
/// far away, and the flow step reports a malformed answer rather than a
/// structured-output failure — a diagnosis that sends you to the prompt when the
/// problem is the request.
///
/// So: after extraction, if a schema was asked for and no tool call supplied the
/// content, the content must at least BE json. When it is not, the provider
/// should say so itself.
pub fn structured_output_missing(
    response: &CompletionResponse,
    response_format: Option<&ResponseFormat>,
    found_tool_call: bool,
) -> bool {
    if found_tool_call || !is_schema_request(response_format) {
        return false;
    }
    let content = strip_markdown_fences(&response.message.content);
    let content = content.trim();
    content.is_empty() || serde_json::from_str::<serde_json::Value>(content).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FunctionCall, JsonSchemaSpec, Message, Role, ToolCall};

    /// The exact string Groq returns when a model answers a forced-tool
    /// structured-output request in content — reproduced from a live call to
    /// `groq:openai/gpt-oss-20b` on 2026-08-29.
    const GROQ_TOOL_CHOICE_ERROR: &str = concat!(
        "Tool choice is required, but model did not call a tool",
        "\nFailed generation: ",
        r#"{"variants":[{"text":"Spring cohort opens","angle":"direct","rationale":"x"}],"notes":""}"#
    );

    #[test]
    fn test_salvage_recovers_the_stranded_json() {
        let got = salvage_failed_generation(GROQ_TOOL_CHOICE_ERROR).expect("nothing salvaged");
        let parsed: serde_json::Value = serde_json::from_str(&got).unwrap();
        assert_eq!(parsed["variants"][0]["text"], "Spring cohort opens");
    }

    #[test]
    fn test_salvage_strips_a_markdown_fence() {
        let msg = "Tool choice is required, but model did not call a tool\nFailed generation: ```json\n{\"ok\":true}\n```";
        assert_eq!(
            salvage_failed_generation(msg).as_deref(),
            Some(r#"{"ok":true}"#)
        );
    }

    #[test]
    fn test_salvage_refuses_anything_that_is_not_json() {
        // The guarantee: a caller cannot tell a salvaged answer from a normal
        // completion, so prose must stay an error rather than become one that
        // looks like success.
        assert!(salvage_failed_generation(
            "Tool choice is required, but model did not call a tool\nFailed generation: I'm sorry, I can't help with that."
        )
        .is_none());
        assert!(salvage_failed_generation(
            "Tool choice is required, but model did not call a tool\nFailed generation: "
        )
        .is_none());
        // No marker at all — an ordinary failure, nothing to recover.
        assert!(salvage_failed_generation("Rate limit exceeded").is_none());
    }

    #[test]
    fn test_is_schema_request_only_for_json_schema() {
        let schema = ResponseFormat::JsonSchema {
            schema: JsonSchemaSpec {
                name: Some("x".to_string()),
                schema: serde_json::json!({}),
                strict: true,
            },
        };
        assert!(is_schema_request(Some(&schema)));
        assert!(!is_schema_request(Some(&ResponseFormat::JsonObject)));
        assert!(!is_schema_request(Some(&ResponseFormat::Text)));
        assert!(!is_schema_request(None));
    }

    /// The marker is a contract between two modules; a test pins it so a tidy-up
    /// of the error string cannot silently sever the salvage path.
    #[test]
    fn test_marker_round_trips_through_the_error_mapper() {
        use crate::provider::ProviderError;
        use crate::providers::http_helpers::{
            map_openai_style_error, OpenAIStyleError, OpenAIStyleErrorDetail,
        };

        let err = map_openai_style_error(OpenAIStyleError {
            error: OpenAIStyleErrorDetail {
                message: "Tool choice is required, but model did not call a tool".to_string(),
                error_type: Some("invalid_request_error".to_string()),
                code: None,
                failed_generation: Some(r#"{"variants":[],"notes":"n"}"#.to_string()),
            },
        });
        let ProviderError::RequestFailed(msg) = err else {
            panic!("wrong error variant");
        };
        assert_eq!(
            salvage_failed_generation(&msg).as_deref(),
            Some(r#"{"variants":[],"notes":"n"}"#)
        );
    }

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
