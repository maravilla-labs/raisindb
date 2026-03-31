//! Shared SSE (Server-Sent Events) parsing utilities for AI providers.
//!
//! This module extracts common SSE line-parsing logic so that each provider
//! only needs a thin `parse_*_chunk` function to convert its JSON format into
//! a [`StreamChunk`].

/// A parsed SSE event with optional event type and data payload.
#[derive(Debug)]
pub struct SseEvent<'a> {
    /// The `event:` line value, if present.
    pub event_type: Option<&'a str>,
    /// The `data:` line value.
    pub data: &'a str,
}

/// Extract `data:` payloads from SSE text, skipping non-data lines.
///
/// Stops at the `[DONE]` sentinel. This is suitable for OpenAI-compatible
/// providers (OpenAI, Groq, OpenRouter, etc.) that do not use `event:` lines.
pub fn parse_sse_data_lines(text: &str) -> Vec<&str> {
    let mut payloads = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        payloads.push(data);
    }

    payloads
}

/// Extract SSE events with both `event:` and `data:` lines.
///
/// This is suitable for Anthropic's Messages API which uses `event:` lines
/// to distinguish `content_block_delta`, `message_delta`, `message_start`, etc.
///
/// Each returned [`SseEvent`] pairs the most recent `event:` line with the
/// following `data:` line. The `[DONE]` sentinel terminates parsing.
pub fn parse_sse_event_lines(text: &str) -> Vec<SseEvent<'_>> {
    let mut events = Vec::new();
    let mut current_event_type: Option<&str> = None;

    for line in text.lines() {
        let line = line.trim();

        if let Some(event_type) = line.strip_prefix("event: ") {
            current_event_type = Some(event_type);
            continue;
        }

        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }

        events.push(SseEvent {
            event_type: current_event_type.take(),
            data,
        });
    }

    events
}

/// Convert a raw byte stream into a stream of text blocks containing only
/// complete lines. Partial lines at chunk boundaries are buffered and prepended
/// to the next chunk, preventing data loss when SSE events are split across
/// HTTP byte chunks.
///
/// Each yielded `Ok(String)` contains one or more complete lines (ending with `\n`).
/// Providers can safely pass these text blocks to their existing SSE parsers.
pub fn buffered_text_stream<S, B, E>(
    byte_stream: S,
) -> impl futures::stream::Stream<Item = std::result::Result<String, E>> + Send
where
    S: futures::stream::Stream<Item = std::result::Result<B, E>> + Send,
    B: AsRef<[u8]>,
    E: Send,
{
    use futures::stream::StreamExt;

    byte_stream
        .scan(String::new(), |buffer, result| {
            let output = match result {
                Err(e) => Some(Err(e)),
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(bytes.as_ref()));
                    match buffer.rfind('\n') {
                        Some(last_nl) => {
                            let complete = buffer[..=last_nl].to_string();
                            *buffer = buffer[last_nl + 1..].to_string();
                            Some(Ok(complete))
                        }
                        None => None,
                    }
                }
            };
            futures::future::ready(Some(output))
        })
        .filter_map(|opt| futures::future::ready(opt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sse_data_lines_basic() {
        let text = "data: {\"a\":1}\n\ndata: {\"b\":2}\n\ndata: [DONE]\n";
        let lines = parse_sse_data_lines(text);
        assert_eq!(lines, vec![r#"{"a":1}"#, r#"{"b":2}"#]);
    }

    #[test]
    fn test_parse_sse_data_lines_skips_non_data() {
        let text = ": comment\nevent: some_event\nid: 123\nretry: 5000\n\ndata: {\"ok\":true}\n\ndata: [DONE]\n";
        let lines = parse_sse_data_lines(text);
        assert_eq!(lines, vec![r#"{"ok":true}"#]);
    }

    #[test]
    fn test_parse_sse_data_lines_empty() {
        assert!(parse_sse_data_lines("").is_empty());
    }

    #[test]
    fn test_parse_sse_data_lines_stops_at_done() {
        let text = "data: first\ndata: [DONE]\ndata: should_not_appear\n";
        let lines = parse_sse_data_lines(text);
        assert_eq!(lines, vec!["first"]);
    }

    #[test]
    fn test_parse_sse_event_lines_basic() {
        let text = "event: content_block_delta\ndata: {\"delta\":\"hi\"}\n\nevent: message_delta\ndata: {\"stop\":true}\n\ndata: [DONE]\n";
        let events = parse_sse_event_lines(text);
        assert_eq!(events.len(), 2);

        assert_eq!(events[0].event_type, Some("content_block_delta"));
        assert_eq!(events[0].data, r#"{"delta":"hi"}"#);

        assert_eq!(events[1].event_type, Some("message_delta"));
        assert_eq!(events[1].data, r#"{"stop":true}"#);
    }

    #[test]
    fn test_parse_sse_event_lines_data_without_event() {
        let text = "data: {\"no_event\":true}\n\ndata: [DONE]\n";
        let events = parse_sse_event_lines(text);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, None);
        assert_eq!(events[0].data, r#"{"no_event":true}"#);
    }

    #[test]
    fn test_parse_sse_event_lines_empty() {
        assert!(parse_sse_event_lines("").is_empty());
    }

    #[test]
    fn test_buffered_text_stream_handles_split_boundaries() {
        use futures::stream::StreamExt;

        // Simulate an SSE event split across two byte chunks:
        // "data: {"delta":"Hello"}\n" is split mid-token
        let chunks: Vec<Result<Vec<u8>, String>> = vec![
            Ok(b"data: {\"delta\":\"Hel".to_vec()),
            Ok(b"lo\"}\ndata: {\"delta\":\" world\"}\n".to_vec()),
        ];

        let stream = buffered_text_stream(futures::stream::iter(chunks));
        let texts: Vec<Result<String, String>> = futures::executor::block_on(stream.collect());

        // First chunk has no complete line → buffered.
        // Second chunk completes both lines → one text block emitted.
        assert_eq!(texts.len(), 1);
        let text = texts[0].as_ref().unwrap();
        assert!(text.contains(r#"data: {"delta":"Hello"}"#));
        assert!(text.contains(r#"data: {"delta":" world"}"#));
    }

    #[test]
    fn test_buffered_text_stream_no_data_loss() {
        use futures::stream::StreamExt;

        // Three SSE events where the boundary splits mid-line
        let chunks: Vec<Result<Vec<u8>, String>> = vec![
            Ok(b"data: {\"t\":\"You\"}\ndata: {\"t\":".to_vec()),
            Ok(b"\" are\"}\ndata: {\"t\":\" great\"}\n".to_vec()),
        ];

        let stream = buffered_text_stream(futures::stream::iter(chunks));
        let texts: Vec<Result<String, String>> = futures::executor::block_on(stream.collect());

        // Chunk 1: "You" line complete, partial "are" buffered → 1 block
        // Chunk 2: completes "are" + full "great" → 1 block
        assert_eq!(texts.len(), 2);

        let first = texts[0].as_ref().unwrap();
        assert!(first.contains(r#"data: {"t":"You"}"#));

        let second = texts[1].as_ref().unwrap();
        assert!(second.contains(r#"data: {"t":" are"}"#));
        assert!(second.contains(r#"data: {"t":" great"}"#));
    }

    #[test]
    fn test_buffered_text_stream_complete_chunks_pass_through() {
        use futures::stream::StreamExt;

        // Chunks that already end on line boundaries should pass through unchanged
        let chunks: Vec<Result<Vec<u8>, String>> = vec![
            Ok(b"data: first\n".to_vec()),
            Ok(b"data: second\ndata: third\n".to_vec()),
        ];

        let stream = buffered_text_stream(futures::stream::iter(chunks));
        let texts: Vec<Result<String, String>> = futures::executor::block_on(stream.collect());

        assert_eq!(texts.len(), 2);
        assert_eq!(texts[0].as_ref().unwrap(), "data: first\n");
        assert!(texts[1].as_ref().unwrap().contains("data: second\n"));
        assert!(texts[1].as_ref().unwrap().contains("data: third\n"));
    }

    #[test]
    fn test_buffered_text_stream_error_propagation() {
        use futures::stream::StreamExt;

        let chunks: Vec<Result<Vec<u8>, String>> =
            vec![Ok(b"data: ok\n".to_vec()), Err("network error".to_string())];

        let stream = buffered_text_stream(futures::stream::iter(chunks));
        let texts: Vec<Result<String, String>> = futures::executor::block_on(stream.collect());

        assert_eq!(texts.len(), 2);
        assert!(texts[0].is_ok());
        assert!(texts[1].is_err());
        assert_eq!(texts[1].as_ref().unwrap_err(), "network error");
    }
}
