//! Streaming accumulation utilities for AI completions.
//!
//! Provides helpers to consume a channel of streaming JSON chunks from an AI
//! provider, accumulate text/tool_calls into a single response [`Value`], and
//! emit real-time events for text and thought content.

use crate::tool_call_extraction;
use crate::types::StreamChunk;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use tokio::sync::mpsc;

/// Convert a [`StreamChunk`] into a [`serde_json::Value`] suitable for the
/// accumulation channel.
pub fn stream_chunk_to_json(chunk: StreamChunk) -> Value {
    serde_json::to_value(chunk).unwrap_or_default()
}

/// Events emitted during stream accumulation.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A chunk of assistant text content.
    TextChunk(String),
    /// A chunk of thinking/reasoning content.
    ThoughtChunk(String),
}

/// Incrementally separates model-emitted `<think>...</think>` blocks from
/// ordinary assistant text. Some OpenAI-compatible providers (notably models
/// served by Groq) return reasoning this way instead of using a provider-native
/// thinking field, and the tags may be split across streaming chunks.
struct ThinkTagDetector {
    buffer: String,
    in_thought: bool,
}

impl ThinkTagDetector {
    fn new() -> Self {
        Self {
            buffer: String::new(),
            in_thought: false,
        }
    }

    fn feed(&mut self, delta: &str) -> Vec<(bool, String)> {
        self.buffer.push_str(delta);
        self.drain(false)
    }

    fn flush(&mut self) -> Vec<(bool, String)> {
        self.drain(true)
    }

    fn drain(&mut self, flush: bool) -> Vec<(bool, String)> {
        let mut output = Vec::new();

        loop {
            let tag = if self.in_thought {
                "</think>"
            } else {
                "<think>"
            };
            if let Some(index) = self.buffer.find(tag) {
                if index > 0 {
                    output.push((self.in_thought, self.buffer[..index].to_string()));
                }
                self.buffer.drain(..index + tag.len());
                self.in_thought = !self.in_thought;
                continue;
            }

            let safe_len = if flush {
                self.buffer.len()
            } else {
                safe_tag_emit_len(&self.buffer, tag)
            };
            if safe_len > 0 {
                output.push((self.in_thought, self.buffer[..safe_len].to_string()));
                self.buffer.drain(..safe_len);
            }
            break;
        }

        output
    }
}

/// Hold back a suffix that could become `tag` when the next delta arrives.
fn safe_tag_emit_len(buffer: &str, tag: &str) -> usize {
    let max_prefix = buffer.len().min(tag.len().saturating_sub(1));
    for len in (1..=max_prefix).rev() {
        if buffer.ends_with(&tag[..len]) {
            return buffer.len() - len;
        }
    }
    buffer.len()
}

/// Consume streaming chunks from a channel, accumulate them into a final
/// OpenAI-compatible response object, and invoke `on_event` for each text or
/// thought delta.
///
/// The channel receives `serde_json::Value` items. Each item is expected to
/// have the shape of a `StreamChunk` (with `delta`, optional `tool_calls`,
/// optional `stop_reason`, optional `model`, optional `usage`).
///
/// Returns the accumulated response as a JSON object with `content`,
/// `tool_calls`, `model`, `stop_reason`, and `usage` fields.
pub async fn accumulate_stream<F, Fut>(
    rx: &mut mpsc::Receiver<Value>,
    mut on_event: F,
    tool_map: &mut HashMap<String, String>,
) -> Value
where
    F: FnMut(StreamEvent) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut content = String::new();
    let mut thinking = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    let mut model = String::new();
    let mut stop_reason: Option<String> = None;
    let mut usage: Option<Value> = None;
    let mut end_session = false;
    let mut handoff_to: Option<String> = None;
    let mut detector = tool_call_extraction::StreamingToolCallDetector::new();
    let mut think_detector = ThinkTagDetector::new();

    while let Some(chunk) = rx.recv().await {
        // Extract text: streaming chunks carry `delta`; complete
        // (non-streaming) responses forwarded as a single chunk - the
        // fallback path when no streaming provider is configured - carry
        // the full text in `content`.
        let text_delta = chunk
            .get("delta")
            .and_then(|d| d.as_str())
            .or_else(|| chunk.get("content").and_then(|c| c.as_str()));

        if let Some(delta) = text_delta {
            if !delta.is_empty() {
                // Determine if this is thinking content
                let is_thinking = chunk
                    .get("content_type")
                    .and_then(|ct| ct.as_str())
                    .map(|ct| ct == "thinking")
                    .unwrap_or(false);

                if is_thinking {
                    thinking.push_str(delta);
                    on_event(StreamEvent::ThoughtChunk(delta.to_string())).await;
                } else {
                    // Some compatible providers put reasoning in literal
                    // <think> blocks. Separate those before tool-call parsing
                    // so neither tags nor private reasoning become message text.
                    for (is_tagged_thought, text) in think_detector.feed(delta) {
                        if is_tagged_thought {
                            thinking.push_str(&text);
                            on_event(StreamEvent::ThoughtChunk(text)).await;
                            continue;
                        }

                        // Feed visible text through the detector to intercept
                        // raw function syntax.
                        let output = detector.feed(&text);
                        if !output.text.is_empty() {
                            content.push_str(&output.text);
                            on_event(StreamEvent::TextChunk(output.text)).await;
                        }
                        for tc in output.tool_calls {
                            let idx = tool_calls.len();
                            tool_calls.push(serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.function.name,
                                    "arguments": tc.function.arguments
                                },
                                "index": idx
                            }));
                        }
                    }
                }
            }
        }

        // Merge tool call deltas
        if let Some(tcs) = chunk.get("tool_calls").and_then(|v| v.as_array()) {
            for tc_delta in tcs {
                merge_streaming_tool_call(&mut tool_calls, tc_delta);
            }
        }

        // Capture model
        if let Some(m) = chunk.get("model").and_then(|v| v.as_str()) {
            if !m.is_empty() {
                model = m.to_string();
            }
        }

        // Capture stop reason (complete responses use "finish_reason")
        if let Some(sr) = chunk
            .get("stop_reason")
            .or_else(|| chunk.get("finish_reason"))
            .and_then(|v| v.as_str())
        {
            stop_reason = Some(sr.to_string());
        }

        // Capture usage. Providers may report usage incrementally across
        // chunks (e.g. Anthropic sends input tokens on `message_start` and
        // output tokens on `message_delta`), so merge rather than replace.
        if let Some(u) = chunk.get("usage") {
            if !u.is_null() {
                usage = Some(match usage.take() {
                    None => u.clone(),
                    Some(prev) => merge_usage_values(&prev, u),
                });
            }
        }

        // Capture session-control signals (chat steps)
        if let Some(es) = chunk.get("end_session").and_then(|v| v.as_bool()) {
            if es {
                end_session = true;
            }
        }
        if let Some(h) = chunk.get("handoff_to").and_then(|v| v.as_str()) {
            if !h.is_empty() {
                handoff_to = Some(h.to_string());
            }
        }
    }

    // Flush any remaining tagged reasoning before the tool-call detector.
    for (is_tagged_thought, text) in think_detector.flush() {
        if is_tagged_thought {
            thinking.push_str(&text);
            on_event(StreamEvent::ThoughtChunk(text)).await;
        } else {
            let output = detector.feed(&text);
            if !output.text.is_empty() {
                content.push_str(&output.text);
                on_event(StreamEvent::TextChunk(output.text)).await;
            }
            for tc in output.tool_calls {
                let idx = tool_calls.len();
                tool_calls.push(serde_json::json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.function.name,
                        "arguments": tc.function.arguments
                    },
                    "index": idx
                }));
            }
        }
    }

    // Flush any remaining buffered content from the tool-call detector.
    let flush = detector.flush();
    if !flush.text.is_empty() {
        content.push_str(&flush.text);
        on_event(StreamEvent::TextChunk(flush.text)).await;
    }
    for tc in flush.tool_calls {
        let idx = tool_calls.len();
        tool_calls.push(serde_json::json!({
            "id": tc.id,
            "type": "function",
            "function": {
                "name": tc.function.name,
                "arguments": tc.function.arguments
            },
            "index": idx
        }));
    }

    // If the detector intercepted tool calls, override stop_reason
    if detector.extracted_any()
        && !tool_calls.is_empty()
        && (stop_reason.as_deref() == Some("stop") || stop_reason.is_none())
    {
        stop_reason = Some("tool_calls".to_string());
    }

    // Build tool_map for caller
    for tc in &tool_calls {
        if let (Some(id), Some(name)) = (
            tc.get("id").and_then(|v| v.as_str()),
            tc.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str()),
        ) {
            tool_map.insert(id.to_string(), name.to_string());
        }
    }

    // Build accumulated response
    let mut result = serde_json::json!({
        "content": content,
        "model": model,
    });

    if !thinking.is_empty() {
        result["thinking"] = Value::String(thinking);
    }

    if !tool_calls.is_empty() {
        result["tool_calls"] = Value::Array(tool_calls);
    }

    if let Some(sr) = stop_reason {
        // Emit under both keys: providers/consumers are split between the
        // OpenAI-style "finish_reason" and the Anthropic-style "stop_reason".
        result["stop_reason"] = Value::String(sr.clone());
        result["finish_reason"] = Value::String(sr);
    }

    if let Some(u) = usage {
        result["usage"] = u;
    }

    if end_session {
        result["end_session"] = Value::Bool(true);
    }

    if let Some(h) = handoff_to {
        result["handoff_to"] = Value::String(h);
    }

    result
}

/// Merge two usage objects reported on different stream chunks.
///
/// Token counts reported during streaming are cumulative, so taking the
/// per-field maximum is correct both for providers that re-send the full
/// usage on the final chunk and for providers that split usage across
/// chunks (e.g. Anthropic: input tokens on `message_start`, output tokens
/// on `message_delta`). `total_tokens` is recomputed when the reported
/// totals don't cover the merged prompt + completion counts.
pub fn merge_usage_values(prev: &Value, next: &Value) -> Value {
    let field = |v: &Value, key: &str| v.get(key).and_then(|x| x.as_u64()).unwrap_or(0);

    let prompt = field(prev, "prompt_tokens").max(field(next, "prompt_tokens"));
    let completion = field(prev, "completion_tokens").max(field(next, "completion_tokens"));
    let total = field(prev, "total_tokens")
        .max(field(next, "total_tokens"))
        .max(prompt + completion);

    // Preserve any provider-specific extra fields (later chunks win).
    let mut merged = prev.clone();
    if let (Some(m), Some(n)) = (merged.as_object_mut(), next.as_object()) {
        for (k, v) in n {
            m.insert(k.clone(), v.clone());
        }
    }
    merged["prompt_tokens"] = prompt.into();
    merged["completion_tokens"] = completion.into();
    merged["total_tokens"] = total.into();
    merged
}

/// Merge a streaming tool call delta into the accumulated tool_calls array.
///
/// Each delta has an `index` field that identifies which tool call it belongs
/// to. If the index is new, a new tool call entry is created. Otherwise, the
/// `function.arguments` string is appended to the existing entry.
pub fn merge_streaming_tool_call(tool_calls: &mut Vec<Value>, delta: &Value) {
    let index = delta.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    // Ensure the array is large enough
    while tool_calls.len() <= index {
        tool_calls.push(serde_json::json!({
            "id": "",
            "type": "function",
            "function": {
                "name": "",
                "arguments": ""
            }
        }));
    }

    let entry = &mut tool_calls[index];

    // Merge id
    if let Some(id) = delta.get("id").and_then(|v| v.as_str()) {
        if !id.is_empty() {
            entry["id"] = Value::String(id.to_string());
        }
    }

    // Merge type
    if let Some(t) = delta.get("type").and_then(|v| v.as_str()) {
        entry["type"] = Value::String(t.to_string());
    }

    // Merge function name
    if let Some(name) = delta
        .get("function")
        .and_then(|f| f.get("name"))
        .and_then(|n| n.as_str())
    {
        if !name.is_empty() {
            entry["function"]["name"] = Value::String(name.to_string());
        }
    }

    // Append function arguments
    if let Some(args) = delta
        .get("function")
        .and_then(|f| f.get("arguments"))
        .and_then(|a| a.as_str())
    {
        let existing = entry["function"]["arguments"]
            .as_str()
            .unwrap_or("")
            .to_string();
        entry["function"]["arguments"] = Value::String(existing + args);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_streaming_tool_call_new_entry() {
        let mut tool_calls: Vec<Value> = Vec::new();
        let delta = serde_json::json!({
            "index": 0,
            "id": "call_123",
            "type": "function",
            "function": {
                "name": "get_weather",
                "arguments": "{\"city\":"
            }
        });

        merge_streaming_tool_call(&mut tool_calls, &delta);

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "call_123");
        assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
        assert_eq!(tool_calls[0]["function"]["arguments"], "{\"city\":");
    }

    #[test]
    fn test_merge_streaming_tool_call_append_arguments() {
        let mut tool_calls: Vec<Value> = vec![serde_json::json!({
            "id": "call_123",
            "type": "function",
            "function": {
                "name": "get_weather",
                "arguments": "{\"city\":"
            }
        })];

        let delta = serde_json::json!({
            "index": 0,
            "function": {
                "arguments": "\"London\"}"
            }
        });

        merge_streaming_tool_call(&mut tool_calls, &delta);

        assert_eq!(
            tool_calls[0]["function"]["arguments"],
            "{\"city\":\"London\"}"
        );
    }

    #[test]
    fn test_merge_streaming_tool_call_multiple_indices() {
        let mut tool_calls: Vec<Value> = Vec::new();

        merge_streaming_tool_call(
            &mut tool_calls,
            &serde_json::json!({
                "index": 0,
                "id": "call_1",
                "function": { "name": "tool_a", "arguments": "{}" }
            }),
        );

        merge_streaming_tool_call(
            &mut tool_calls,
            &serde_json::json!({
                "index": 1,
                "id": "call_2",
                "function": { "name": "tool_b", "arguments": "{}" }
            }),
        );

        assert_eq!(tool_calls.len(), 2);
        assert_eq!(tool_calls[0]["function"]["name"], "tool_a");
        assert_eq!(tool_calls[1]["function"]["name"], "tool_b");
    }

    #[tokio::test]
    async fn test_accumulate_stream_text_only() {
        let (tx, mut rx) = mpsc::channel(10);

        tx.send(serde_json::json!({ "delta": "Hello", "model": "test" }))
            .await
            .unwrap();
        tx.send(serde_json::json!({ "delta": " world", "stop_reason": "stop" }))
            .await
            .unwrap();
        drop(tx);

        let mut events = Vec::new();
        let mut tool_map = HashMap::new();

        let result = accumulate_stream(
            &mut rx,
            |event| {
                events.push(event);
                async {}
            },
            &mut tool_map,
        )
        .await;

        assert_eq!(result["content"], "Hello world");
        assert_eq!(result["stop_reason"], "stop");
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_accumulate_stream_extracts_think_blocks() {
        let (tx, mut rx) = mpsc::channel(10);

        tx.send(serde_json::json!({
            "delta": "<think>I should inspect the current "
        }))
        .await
        .unwrap();
        tx.send(serde_json::json!({
            "delta": "configuration.</think>Here is the answer.",
            "stop_reason": "stop"
        }))
        .await
        .unwrap();
        drop(tx);

        let mut events = Vec::new();
        let mut tool_map = HashMap::new();
        let result = accumulate_stream(
            &mut rx,
            |event| {
                events.push(event);
                async {}
            },
            &mut tool_map,
        )
        .await;

        assert_eq!(result["content"], "Here is the answer.");
        assert_eq!(
            result["thinking"],
            "I should inspect the current configuration."
        );

        let visible: String = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::TextChunk(text) => Some(text.as_str()),
                StreamEvent::ThoughtChunk(_) => None,
            })
            .collect();
        let thoughts: String = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::ThoughtChunk(text) => Some(text.as_str()),
                StreamEvent::TextChunk(_) => None,
            })
            .collect();

        assert_eq!(visible, "Here is the answer.");
        assert_eq!(thoughts, "I should inspect the current configuration.");
    }

    #[tokio::test]
    async fn test_accumulate_stream_handles_think_tags_split_across_chunks() {
        let (tx, mut rx) = mpsc::channel(10);

        for delta in ["<thi", "nk>private", " reasoning</thi", "nk>public"] {
            tx.send(serde_json::json!({ "delta": delta }))
                .await
                .unwrap();
        }
        drop(tx);

        let mut events = Vec::new();
        let mut tool_map = HashMap::new();
        let result = accumulate_stream(
            &mut rx,
            |event| {
                events.push(event);
                async {}
            },
            &mut tool_map,
        )
        .await;

        assert_eq!(result["content"], "public");
        assert_eq!(result["thinking"], "private reasoning");
        assert!(events.iter().all(|event| match event {
            StreamEvent::TextChunk(text) | StreamEvent::ThoughtChunk(text) => {
                !text.contains("<think>") && !text.contains("</think>")
            }
        }));
    }

    #[tokio::test]
    async fn test_accumulate_stream_treats_unclosed_think_block_as_reasoning() {
        let (tx, mut rx) = mpsc::channel(10);
        tx.send(serde_json::json!({ "delta": "<think>still reasoning" }))
            .await
            .unwrap();
        drop(tx);

        let mut tool_map = HashMap::new();
        let result = accumulate_stream(&mut rx, |_| async {}, &mut tool_map).await;

        assert_eq!(result["content"], "");
        assert_eq!(result["thinking"], "still reasoning");
    }

    #[tokio::test]
    async fn test_accumulate_stream_merges_split_usage() {
        // Anthropic style: input usage on message_start, output usage on
        // message_delta. The accumulator must merge, not take the last.
        let (tx, mut rx) = mpsc::channel(10);

        tx.send(serde_json::json!({
            "delta": "",
            "model": "claude-sonnet-4-5",
            "usage": { "prompt_tokens": 42, "completion_tokens": 1, "total_tokens": 43 }
        }))
        .await
        .unwrap();
        tx.send(serde_json::json!({ "delta": "Hello" }))
            .await
            .unwrap();
        tx.send(serde_json::json!({
            "delta": "",
            "stop_reason": "end_turn",
            "usage": { "prompt_tokens": 0, "completion_tokens": 17, "total_tokens": 17 }
        }))
        .await
        .unwrap();
        drop(tx);

        let mut tool_map = HashMap::new();
        let result = accumulate_stream(&mut rx, |_| async {}, &mut tool_map).await;

        assert_eq!(result["usage"]["prompt_tokens"], 42);
        assert_eq!(result["usage"]["completion_tokens"], 17);
        assert_eq!(result["usage"]["total_tokens"], 59);
    }

    #[tokio::test]
    async fn test_accumulate_stream_usage_on_trailing_chunk() {
        // OpenAI stream_options.include_usage style: usage arrives on a
        // trailing usage-only chunk after the finish chunk.
        let (tx, mut rx) = mpsc::channel(10);

        tx.send(serde_json::json!({ "delta": "Hi", "model": "gpt-4o" }))
            .await
            .unwrap();
        tx.send(serde_json::json!({ "delta": "", "stop_reason": "stop" }))
            .await
            .unwrap();
        tx.send(serde_json::json!({
            "delta": "",
            "usage": { "prompt_tokens": 11, "completion_tokens": 3, "total_tokens": 14 }
        }))
        .await
        .unwrap();
        drop(tx);

        let mut tool_map = HashMap::new();
        let result = accumulate_stream(&mut rx, |_| async {}, &mut tool_map).await;

        assert_eq!(result["usage"]["prompt_tokens"], 11);
        assert_eq!(result["usage"]["completion_tokens"], 3);
        assert_eq!(result["usage"]["total_tokens"], 14);
        assert_eq!(result["stop_reason"], "stop");
    }

    #[test]
    fn test_merge_usage_values_cumulative_updates() {
        // Gemini style: cumulative usageMetadata on several chunks.
        let a =
            serde_json::json!({ "prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12 });
        let b =
            serde_json::json!({ "prompt_tokens": 10, "completion_tokens": 9, "total_tokens": 19 });
        let merged = merge_usage_values(&a, &b);
        assert_eq!(merged["prompt_tokens"], 10);
        assert_eq!(merged["completion_tokens"], 9);
        assert_eq!(merged["total_tokens"], 19);
    }

    #[test]
    fn test_merge_usage_values_preserves_extra_fields() {
        let a =
            serde_json::json!({ "prompt_tokens": 5, "completion_tokens": 0, "total_tokens": 5 });
        let b = serde_json::json!({ "prompt_tokens": 0, "completion_tokens": 7, "total_tokens": 7, "cost": 0.001 });
        let merged = merge_usage_values(&a, &b);
        assert_eq!(merged["prompt_tokens"], 5);
        assert_eq!(merged["completion_tokens"], 7);
        assert_eq!(merged["total_tokens"], 12);
        assert_eq!(merged["cost"], 0.001);
    }

    #[tokio::test]
    async fn test_accumulate_stream_extracts_raw_tool_calls() {
        let (tx, mut rx) = mpsc::channel(10);

        // Simulate model emitting raw function syntax as content deltas
        tx.send(serde_json::json!({
            "delta": r#"<function=weather>{"city":"Bern"}</function>"#,
            "model": "llama-3.3-70b",
            "stop_reason": "stop"
        }))
        .await
        .unwrap();
        drop(tx);

        let mut events = Vec::new();
        let mut tool_map = HashMap::new();
        let result = accumulate_stream(
            &mut rx,
            |event| {
                events.push(event);
                async {}
            },
            &mut tool_map,
        )
        .await;

        // No TextChunk events should be emitted for pure tool call content
        let text_chunks: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::TextChunk(_)))
            .collect();
        assert!(
            text_chunks.is_empty(),
            "No TextChunk events should be emitted for raw tool call syntax"
        );

        // Content should be cleaned (empty after stripping)
        let content = result["content"].as_str().unwrap_or("");
        assert!(
            !content.contains("<function="),
            "Raw function syntax should be stripped from content"
        );

        // Tool calls should be extracted
        let tool_calls = result["tool_calls"]
            .as_array()
            .expect("should have tool_calls");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["function"]["name"], "weather");
        assert_eq!(tool_calls[0]["function"]["arguments"], r#"{"city":"Bern"}"#);

        // Stop reason should be overridden to tool_calls
        assert_eq!(result["stop_reason"], "tool_calls");

        // tool_map should have the extracted call
        assert_eq!(tool_map.len(), 1);
        assert!(tool_map.values().any(|v| v == "weather"));
    }

    #[tokio::test]
    async fn test_accumulate_stream_strips_control_tokens() {
        let (tx, mut rx) = mpsc::channel(10);

        tx.send(serde_json::json!({
            "delta": "<|python_tag|>Hello world<|eom_id|>",
            "model": "test",
            "stop_reason": "stop"
        }))
        .await
        .unwrap();
        drop(tx);

        let mut tool_map = HashMap::new();
        let result = accumulate_stream(&mut rx, |_| async {}, &mut tool_map).await;

        assert_eq!(result["content"], "Hello world");
    }

    #[tokio::test]
    async fn test_accumulate_stream_split_function_call() {
        let (tx, mut rx) = mpsc::channel(10);

        // Function syntax split across two chunks
        tx.send(serde_json::json!({
            "delta": r#"<function=weather>{"ci"#,
            "model": "llama-3.3-70b"
        }))
        .await
        .unwrap();
        tx.send(serde_json::json!({
            "delta": r#"ty":"Bern"}</function>"#,
            "stop_reason": "stop"
        }))
        .await
        .unwrap();
        drop(tx);

        let mut events = Vec::new();
        let mut tool_map = HashMap::new();
        let result = accumulate_stream(
            &mut rx,
            |event| {
                events.push(event);
                async {}
            },
            &mut tool_map,
        )
        .await;

        // No text events for function syntax
        let text_chunks: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::TextChunk(_)))
            .collect();
        assert!(text_chunks.is_empty());

        // Tool call extracted
        let tool_calls = result["tool_calls"]
            .as_array()
            .expect("should have tool_calls");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["function"]["name"], "weather");
        assert_eq!(result["stop_reason"], "tool_calls");
    }

    #[tokio::test]
    async fn test_accumulate_stream_mixed_text_and_function() {
        let (tx, mut rx) = mpsc::channel(10);

        // Text before function — text should be emitted, function intercepted
        tx.send(serde_json::json!({
            "delta": r#"I'll help! <function=weather>{"city":"Bern"}</function>"#,
            "model": "llama-3.3-70b",
            "stop_reason": "stop"
        }))
        .await
        .unwrap();
        drop(tx);

        let mut events = Vec::new();
        let mut tool_map = HashMap::new();
        let result = accumulate_stream(
            &mut rx,
            |event| {
                events.push(event);
                async {}
            },
            &mut tool_map,
        )
        .await;

        // Text before function should be emitted
        let text_events: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::TextChunk(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        let all_text: String = text_events.join("");
        assert_eq!(all_text, "I'll help! ");

        // Content should only have the text portion
        assert_eq!(result["content"], "I'll help! ");

        // Tool call extracted
        let tool_calls = result["tool_calls"]
            .as_array()
            .expect("should have tool_calls");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["function"]["name"], "weather");
        assert_eq!(result["stop_reason"], "tool_calls");
    }
}
