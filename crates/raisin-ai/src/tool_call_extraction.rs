//! Provider-agnostic tool call extraction from raw content.
//!
//! Some models (e.g. Llama via Groq, Ollama) occasionally emit tool calls as
//! raw text in the content field instead of structured `tool_calls`. This module
//! provides shared utilities to detect, extract, and clean these patterns so
//! that ALL providers benefit from the same post-processing.

use crate::types::{FunctionCall, ToolCall};
use regex::Regex;
use std::sync::LazyLock;

/// Regex matching `<function=name>{args}</function>` blocks.
/// Tool names may contain word chars and hyphens (e.g. `kanban-boards`).
static FUNCTION_CALL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<function=([\w-]+)>([\s\S]*?)</function>").unwrap()
});

/// Regex matching model control tokens that should never appear in user-facing content.
static CONTROL_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<\|(python_tag|eom_id|eot_id|start_header_id|end_header_id|begin_of_text|end_of_text)\|>").unwrap()
});

/// Scan content for raw `<function=name>{args}</function>` patterns and extract
/// them as structured [`ToolCall`] values.
///
/// Returns `None` if no patterns are found.
pub fn extract_tool_calls_from_content(content: &str) -> Option<Vec<ToolCall>> {
    let mut calls = Vec::new();

    for cap in FUNCTION_CALL_RE.captures_iter(content) {
        let name = cap[1].trim();
        if name.is_empty() {
            continue;
        }

        let raw_args = cap[2].trim();
        let arguments = if serde_json::from_str::<serde_json::Value>(raw_args).is_ok() {
            raw_args.to_string()
        } else {
            "{}".to_string()
        };

        let id = format!(
            "call_{}",
            &uuid::Uuid::new_v4().to_string().replace('-', "")[..24]
        );

        calls.push(ToolCall {
            id,
            call_type: "function".to_string(),
            function: FunctionCall {
                name: name.to_string(),
                arguments,
            },
            index: None,
        });
    }

    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
}

/// Remove all `<function=name>...</function>` blocks and any `<|python_tag|>`
/// or similar control tokens from content.
pub fn strip_tool_call_syntax(content: &str) -> String {
    let cleaned = FUNCTION_CALL_RE.replace_all(content, "");
    strip_model_control_tokens(&cleaned)
        .trim()
        .to_string()
}

/// Strip model control tokens (`<|python_tag|>`, `<|eom_id|>`, etc.) from
/// content.
pub fn strip_model_control_tokens(content: &str) -> String {
    CONTROL_TOKEN_RE.replace_all(content, "").to_string()
}

/// Output produced by [`StreamingToolCallDetector::feed`] and
/// [`StreamingToolCallDetector::flush`].
pub struct DetectorOutput {
    /// Clean text safe to emit as a `TextChunk` to the client.
    pub text: String,
    /// Complete tool calls extracted from raw function syntax.
    pub tool_calls: Vec<ToolCall>,
}

/// Incremental detector that intercepts raw `<function=...>...</function>`
/// syntax during streaming so that garbled tool-call text is never emitted to
/// the client.
///
/// Call [`feed`](Self::feed) for every content delta and
/// [`flush`](Self::flush) when the stream ends.
pub struct StreamingToolCallDetector {
    buffer: String,
    extracted_any: bool,
}

impl Default for StreamingToolCallDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingToolCallDetector {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            extracted_any: false,
        }
    }

    /// Returns `true` if at least one tool call was extracted during the
    /// lifetime of this detector.
    pub fn extracted_any(&self) -> bool {
        self.extracted_any
    }

    /// Feed a new content delta into the detector.
    ///
    /// Returns any clean text that can be safely emitted to the client plus
    /// any complete tool calls that were extracted.
    pub fn feed(&mut self, delta: &str) -> DetectorOutput {
        self.buffer.push_str(delta);

        // Strip complete control tokens from the buffer
        self.buffer = CONTROL_TOKEN_RE.replace_all(&self.buffer, "").to_string();

        let mut text = String::new();
        let mut tool_calls = Vec::new();

        loop {
            if let Some(start) = self.buffer.find("<function=") {
                // Emit everything before the function tag as safe text
                let before = &self.buffer[..start];
                if !before.is_empty() {
                    text.push_str(before);
                }

                // Look for closing tag
                let rest = &self.buffer[start..];
                if let Some(close_offset) = rest.find("</function>") {
                    let block_end = close_offset + "</function>".len();
                    let block = &rest[..block_end];

                    // Parse with existing regex
                    if let Some(cap) = FUNCTION_CALL_RE.captures(block) {
                        let name = cap[1].trim();
                        if !name.is_empty() {
                            let raw_args = cap[2].trim();
                            let arguments =
                                if serde_json::from_str::<serde_json::Value>(raw_args).is_ok() {
                                    raw_args.to_string()
                                } else {
                                    "{}".to_string()
                                };

                            let id = format!(
                                "call_{}",
                                &uuid::Uuid::new_v4().to_string().replace('-', "")[..24]
                            );

                            tool_calls.push(ToolCall {
                                id,
                                call_type: "function".to_string(),
                                function: FunctionCall {
                                    name: name.to_string(),
                                    arguments,
                                },
                                index: None,
                            });
                            self.extracted_any = true;
                        }
                    }

                    // Remove the processed block from buffer
                    self.buffer = self.buffer[start + block_end..].to_string();
                    // Continue loop to process more blocks
                    continue;
                } else {
                    // No closing tag yet — keep everything from `<function=` buffered
                    self.buffer = self.buffer[start..].to_string();
                    break;
                }
            } else {
                // No `<function=` found — emit safe portion, buffer trailing ambiguity
                let safe_len = safe_emit_len(&self.buffer);
                if safe_len > 0 {
                    text.push_str(&self.buffer[..safe_len]);
                    self.buffer = self.buffer[safe_len..].to_string();
                }
                break;
            }
        }

        DetectorOutput { text, tool_calls }
    }

    /// Flush any remaining buffered content. Call this when the stream ends.
    ///
    /// Incomplete function blocks are emitted as plain text since they can no
    /// longer complete.
    pub fn flush(&mut self) -> DetectorOutput {
        let text = std::mem::take(&mut self.buffer);
        DetectorOutput {
            text,
            tool_calls: Vec::new(),
        }
    }
}

/// Calculate how many bytes from the start of `buf` are safe to emit.
///
/// We hold back any trailing `<` that could be the start of `<function=`,
/// `</function>`, or a control token like `<|python_tag|>`.
fn safe_emit_len(buf: &str) -> usize {
    // All special sequences we need to watch for
    let prefixes: &[&str] = &[
        "<function=",
        "</function>",
        "<|python_tag|>",
        "<|eom_id|>",
        "<|eot_id|>",
        "<|start_header_id|>",
        "<|end_header_id|>",
        "<|begin_of_text|>",
        "<|end_of_text|>",
    ];
    // Scan backwards from end for a `<`
    if let Some(lt_pos) = buf.rfind('<') {
        let tail = &buf[lt_pos..];
        // Check if tail is a prefix of any of our special sequences
        for prefix in prefixes {
            if prefix.starts_with(tail) {
                return lt_pos;
            }
        }
    }
    buf.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_simple_tool_call() {
        let content = r#"<function=weather>{"city":"Bern"}</function>"#;
        let calls = extract_tool_calls_from_content(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "weather");
        assert_eq!(calls[0].function.arguments, r#"{"city":"Bern"}"#);
        assert!(calls[0].id.starts_with("call_"));
    }

    #[test]
    fn test_extract_hyphenated_tool_name() {
        let content = r#"<|python_tag|><function=kanban-boards>{"op":"move"}</function>"#;
        let calls = extract_tool_calls_from_content(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "kanban-boards");
        assert_eq!(calls[0].function.arguments, r#"{"op":"move"}"#);
    }

    #[test]
    fn test_extract_multiple_tool_calls() {
        let content = r#"<function=weather>{"city":"Bern"}</function><function=calendar>{"date":"today"}</function>"#;
        let calls = extract_tool_calls_from_content(content).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "weather");
        assert_eq!(calls[1].function.name, "calendar");
    }

    #[test]
    fn test_extract_invalid_json_falls_back_to_empty() {
        let content = r#"<function=weather>not valid json</function>"#;
        let calls = extract_tool_calls_from_content(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.arguments, "{}");
    }

    #[test]
    fn test_extract_no_match() {
        let content = "Hello, how can I help you?";
        assert!(extract_tool_calls_from_content(content).is_none());
    }

    #[test]
    fn test_strip_tool_call_syntax() {
        let content = r#"Here is some text <function=weather>{"city":"Bern"}</function> and more text"#;
        let cleaned = strip_tool_call_syntax(content);
        assert_eq!(cleaned, "Here is some text  and more text");
    }

    #[test]
    fn test_strip_control_tokens() {
        let content = "<|python_tag|>Hello <|eom_id|>world";
        let cleaned = strip_model_control_tokens(content);
        assert_eq!(cleaned, "Hello world");
    }

    #[test]
    fn test_strip_combined() {
        let content = r#"<|python_tag|><function=kanban-boards>{"op":"move"}</function><|eom_id|>"#;
        let cleaned = strip_tool_call_syntax(content);
        assert_eq!(cleaned, "");
    }

    #[test]
    fn test_extract_with_surrounding_text() {
        let content = r#"I'll call the tool now <function=weather>{"city":"Zurich"}</function> Done!"#;
        let calls = extract_tool_calls_from_content(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "weather");
        let cleaned = strip_tool_call_syntax(content);
        assert_eq!(cleaned, "I'll call the tool now  Done!");
    }

    // ---- StreamingToolCallDetector tests ----

    #[test]
    fn test_detector_clean_text_passthrough() {
        let mut d = StreamingToolCallDetector::new();
        let out = d.feed("Hello world");
        assert_eq!(out.text, "Hello world");
        assert!(out.tool_calls.is_empty());
        assert!(!d.extracted_any());
    }

    #[test]
    fn test_detector_complete_function_single_feed() {
        let mut d = StreamingToolCallDetector::new();
        let out = d.feed(r#"<function=weather>{"city":"Bern"}</function>"#);
        assert_eq!(out.text, "");
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].function.name, "weather");
        assert_eq!(out.tool_calls[0].function.arguments, r#"{"city":"Bern"}"#);
        assert!(d.extracted_any());
    }

    #[test]
    fn test_detector_function_split_across_feeds() {
        let mut d = StreamingToolCallDetector::new();

        let out1 = d.feed(r#"<function=weather>{"ci"#);
        assert_eq!(out1.text, "");
        assert!(out1.tool_calls.is_empty());

        let out2 = d.feed(r#"ty":"Bern"}</function>"#);
        assert_eq!(out2.text, "");
        assert_eq!(out2.tool_calls.len(), 1);
        assert_eq!(out2.tool_calls[0].function.name, "weather");
        assert!(d.extracted_any());
    }

    #[test]
    fn test_detector_control_tokens_stripped() {
        let mut d = StreamingToolCallDetector::new();
        let out = d.feed("<|python_tag|>Hello<|eom_id|>");
        assert_eq!(out.text, "Hello");
        assert!(out.tool_calls.is_empty());
    }

    #[test]
    fn test_detector_split_control_token() {
        let mut d = StreamingToolCallDetector::new();
        // Control token split across chunks: "<|python" then "_tag|>"
        let out1 = d.feed("Hello<|python");
        // The trailing "<|python" is buffered (prefix of "<|...")
        assert_eq!(out1.text, "Hello");

        let out2 = d.feed("_tag|>World");
        assert_eq!(out2.text, "World");
    }

    #[test]
    fn test_detector_text_before_function() {
        let mut d = StreamingToolCallDetector::new();
        let out = d.feed(r#"I'll call it <function=weather>{"city":"Bern"}</function>"#);
        assert_eq!(out.text, "I'll call it ");
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].function.name, "weather");
    }

    #[test]
    fn test_detector_multiple_functions() {
        let mut d = StreamingToolCallDetector::new();
        let out = d.feed(
            r#"<function=weather>{"city":"Bern"}</function><function=calendar>{"date":"today"}</function>"#,
        );
        assert_eq!(out.text, "");
        assert_eq!(out.tool_calls.len(), 2);
        assert_eq!(out.tool_calls[0].function.name, "weather");
        assert_eq!(out.tool_calls[1].function.name, "calendar");
    }

    #[test]
    fn test_detector_flush_partial() {
        let mut d = StreamingToolCallDetector::new();
        let out1 = d.feed("<function=weather>{\"ci");
        assert_eq!(out1.text, "");
        assert!(out1.tool_calls.is_empty());

        // Stream ends — incomplete function emitted as text
        let out2 = d.flush();
        assert_eq!(out2.text, "<function=weather>{\"ci");
        assert!(out2.tool_calls.is_empty());
    }

    #[test]
    fn test_detector_trailing_lt_buffered() {
        let mut d = StreamingToolCallDetector::new();
        let out1 = d.feed("Hello<");
        // Trailing `<` could start `<function=`, so it's buffered
        assert_eq!(out1.text, "Hello");

        let out2 = d.feed("b>done");
        // `<b>done` is not a function tag, so safe to emit
        assert_eq!(out2.text, "<b>done");
    }

    #[test]
    fn test_detector_extracted_any_tracking() {
        let mut d = StreamingToolCallDetector::new();
        assert!(!d.extracted_any());

        d.feed("Hello world");
        assert!(!d.extracted_any());

        d.feed(r#"<function=test>{"x":1}</function>"#);
        assert!(d.extracted_any());
    }
}
