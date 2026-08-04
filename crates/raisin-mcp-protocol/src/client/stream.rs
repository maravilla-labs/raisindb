// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Reading an SSE stream that stays open.
//!
//! [`StreamableHttpTransport`](super::transport::StreamableHttpTransport) parses
//! SSE too, but only ever for a body it has already buffered whole. Four
//! properties of that path make it unusable for a stream held open for hours,
//! and each one is a requirement here:
//!
//! | Request path | Here |
//! |---|---|
//! | buffers the entire body, then parses | parses incrementally, frame by frame |
//! | reqwest's **total-elapsed** timeout (30s default, 120s hard cap) | an **idle** timeout — silence, not duration, ends a stream |
//! | one **cumulative** byte cap for the whole body | a **per-frame** cap; a long-lived stream must never accumulate |
//! | discards `id:` | captures it, for `Last-Event-ID` on reconnect |
//!
//! Getting the second one wrong is the subtle failure: a total timeout would
//! kill a perfectly healthy subscription on a fixed schedule, which looks
//! exactly like a flaky remote server.

use std::time::Duration;

use futures::{Stream, StreamExt};
use serde_json::Value;

use super::error::{RemoteToolError, Result};

/// Default ceiling on a single SSE frame.
///
/// Per frame, never cumulative. A subscription that runs for a week must not be
/// closer to its limit than one that just opened.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Default silence after which a stream is considered dead.
///
/// Generous, because a quiet server is the normal case: a connection whose tools
/// never change should emit nothing at all. This exists to notice a socket that
/// is open but no longer connected to anything, not to police latency.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(600);

/// One decoded SSE event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `id:` field, when the server sent one. Replayed as `Last-Event-ID`.
    pub id: Option<String>,
    /// The joined `data:` payload.
    pub data: String,
}

/// Incremental reader over a live SSE body.
pub struct SseReader {
    /// Chunks are copied into `Vec<u8>` rather than kept as reqwest's `Bytes`,
    /// purely so this crate needs no `bytes` dependency to name the type. The
    /// copy is irrelevant for this workload: a notification stream carries a few
    /// hundred bytes an hour, not a payload.
    body: std::pin::Pin<Box<dyn Stream<Item = reqwest::Result<Vec<u8>>> + Send>>,
    /// Bytes received but not yet forming a complete frame.
    buffer: Vec<u8>,
    max_frame_bytes: usize,
    idle_timeout: Duration,
    last_event_id: Option<String>,
}

impl SseReader {
    /// Wrap a streaming response body.
    pub fn new(response: reqwest::Response) -> Self {
        Self::from_stream(
            response
                .bytes_stream()
                .map(|chunk| chunk.map(|b| b.to_vec())),
        )
    }

    /// Wrap any chunk stream.
    ///
    /// Exists so the reassembly can be driven from a synthetic chunk sequence:
    /// the interesting cases are all about chunk boundaries falling in the wrong
    /// place, which a real socket will not reproduce on demand.
    pub fn from_stream<S>(chunks: S) -> Self
    where
        S: Stream<Item = reqwest::Result<Vec<u8>>> + Send + 'static,
    {
        Self {
            body: Box::pin(chunks),
            buffer: Vec::new(),
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            last_event_id: None,
        }
    }

    /// Override the per-frame byte cap.
    pub fn with_max_frame_bytes(mut self, max: usize) -> Self {
        self.max_frame_bytes = max;
        self
    }

    /// Override how long silence is tolerated.
    pub fn with_idle_timeout(mut self, idle: Duration) -> Self {
        self.idle_timeout = idle;
        self
    }

    /// The most recent `id:`, to resume from after a drop.
    pub fn last_event_id(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }

    /// Next event, or `None` once the server closes the stream.
    ///
    /// Errors mean the stream is finished either way — a transport failure, a
    /// frame over the cap, or silence past the idle timeout. The caller
    /// reconnects; it never retries on the same reader.
    pub async fn next_event(&mut self) -> Result<Option<SseEvent>> {
        loop {
            // A complete frame may already be sitting in the buffer from the
            // chunk that delivered the previous one — chunk boundaries and frame
            // boundaries have nothing to do with each other.
            if let Some(event) = self.take_frame()? {
                return Ok(Some(event));
            }

            let chunk = match tokio::time::timeout(self.idle_timeout, self.body.next()).await {
                Ok(Some(chunk)) => chunk?,
                // Clean end of stream. Anything left in the buffer is a partial
                // frame the server never terminated, and is discarded rather
                // than guessed at.
                Ok(None) => return Ok(None),
                Err(_) => {
                    return Err(RemoteToolError::Transient(format!(
                        "no data for {}s; treating the stream as dead",
                        self.idle_timeout.as_secs()
                    )))
                }
            };

            if self.buffer.len() + chunk.len() > self.max_frame_bytes {
                return Err(RemoteToolError::Transient(format!(
                    "a single SSE frame exceeded {} bytes",
                    self.max_frame_bytes
                )));
            }
            self.buffer.extend_from_slice(&chunk);
        }
    }

    /// Split one complete frame off the buffer, if there is one.
    fn take_frame(&mut self) -> Result<Option<SseEvent>> {
        let Some((end, skip)) = find_frame_end(&self.buffer) else {
            return Ok(None);
        };
        let raw = self.buffer.drain(..end + skip).collect::<Vec<u8>>();
        let text = std::str::from_utf8(&raw[..end])
            .map_err(|e| RemoteToolError::Protocol(format!("SSE frame is not utf-8: {e}")))?;

        let event = parse_frame(text);
        if let Some(id) = event.as_ref().and_then(|e| e.id.clone()) {
            self.last_event_id = Some(id);
        }
        match event {
            Some(event) => Ok(Some(event)),
            // A frame carrying only comments or a `retry:` is well-formed and
            // means nothing to us. Keep reading rather than reporting an end.
            None => self.take_frame(),
        }
    }
}

/// Offset of the first frame terminator, and its length.
///
/// Safe to scan bytewise: every byte of a multi-byte UTF-8 sequence is `>= 0x80`,
/// so a `\n` can never appear inside one and a chunk splitting a character
/// cannot produce a false boundary.
fn find_frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let mut lf = memchr_lf(buffer, 0);
    while let Some(index) = lf {
        if buffer[index + 1..].starts_with(b"\n") {
            return Some((index, 2));
        }
        if buffer[index + 1..].starts_with(b"\r\n") {
            return Some((index, 3));
        }
        lf = memchr_lf(buffer, index + 1);
    }
    None
}

/// First `\n` at or after `from`.
fn memchr_lf(buffer: &[u8], from: usize) -> Option<usize> {
    buffer
        .get(from..)?
        .iter()
        .position(|b| *b == b'\n')
        .map(|i| i + from)
}

/// Decode one frame's fields. `None` when it carries no `data:`.
fn parse_frame(text: &str) -> Option<SseEvent> {
    let mut data: Vec<&str> = Vec::new();
    let mut id = None;

    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest));
        } else if let Some(rest) = line.strip_prefix("id:") {
            // Per the EventSource spec an id containing NUL is ignored outright
            // rather than sanitized.
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            if !rest.contains('\0') {
                id = Some(rest.to_string());
            }
        }
        // `event:`, `retry:` and comments carry nothing this client acts on.
    }

    if data.is_empty() {
        return None;
    }
    Some(SseEvent {
        id,
        data: data.join("\n"),
    })
}

/// Parse an event's payload as a JSON-RPC message.
pub fn decode_message(event: &SseEvent) -> Result<Value> {
    serde_json::from_str(&event.data)
        .map_err(|e| RemoteToolError::Protocol(format!("malformed json-rpc message: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_frame() {
        let event = parse_frame("data: {\"a\":1}").expect("has data");
        assert_eq!(event.data, "{\"a\":1}");
        assert_eq!(event.id, None);
    }

    #[test]
    fn joins_multiline_data_and_captures_the_id() {
        let event = parse_frame("id: 42\ndata: {\"a\":\ndata: 1}").expect("has data");
        assert_eq!(event.data, "{\"a\":\n1}");
        assert_eq!(event.id.as_deref(), Some("42"));
    }

    /// A keep-alive comment is well-formed and carries nothing. Reporting it as
    /// an event would hand the caller an empty payload to choke on.
    #[test]
    fn a_comment_only_frame_yields_nothing() {
        assert!(parse_frame(": keep-alive").is_none());
        assert!(parse_frame("retry: 5000").is_none());
    }

    #[test]
    fn finds_frame_boundaries_in_both_line_endings() {
        assert_eq!(find_frame_end(b"data: 1\n\ndata: 2"), Some((7, 2)));
        assert_eq!(find_frame_end(b"data: 1\r\n\r\ndata: 2"), Some((8, 3)));
        assert_eq!(find_frame_end(b"data: 1\n"), None);
    }

    /// A chunk boundary can fall anywhere, including mid-character. Scanning for
    /// `\n` bytewise must not be confused by a split multi-byte sequence.
    #[test]
    fn a_split_multibyte_character_creates_no_false_boundary() {
        // "é" is 0xC3 0xA9 — neither byte is \n, and neither can be mistaken
        // for one.
        let buffer = b"data: \xc3\xa9\n\n";
        let (end, skip) = find_frame_end(buffer).expect("boundary after the payload");
        assert_eq!(skip, 2);
        let text = std::str::from_utf8(&buffer[..end]).expect("valid utf-8");
        assert_eq!(parse_frame(text).unwrap().data, "é");
    }

    #[test]
    fn an_id_containing_nul_is_ignored() {
        let event = parse_frame("id: bad\0id\ndata: {}").expect("has data");
        assert_eq!(event.id, None);
    }

    /// Build a reader over a fixed chunk sequence.
    fn reader(chunks: Vec<&'static str>) -> SseReader {
        SseReader::from_stream(futures::stream::iter(
            chunks.into_iter().map(|c| Ok(c.as_bytes().to_vec())),
        ))
    }

    /// The reason this module exists. A chunk boundary has nothing to do with a
    /// frame boundary — here one frame is split across three chunks and a second
    /// frame arrives inside the same chunk that finished the first.
    #[tokio::test]
    async fn frames_are_reassembled_across_chunk_boundaries() {
        let mut reader = reader(vec![
            "id: 1\nda",
            "ta: {\"a\"",
            ":1}\n\ndata: {\"b\":2}\n\n",
        ]);

        let first = reader.next_event().await.unwrap().expect("first frame");
        assert_eq!(first.data, "{\"a\":1}");
        assert_eq!(first.id.as_deref(), Some("1"));

        let second = reader.next_event().await.unwrap().expect("second frame");
        assert_eq!(second.data, "{\"b\":2}");

        assert!(reader.next_event().await.unwrap().is_none(), "stream ends");
    }

    /// `Last-Event-ID` must survive the frames that carry no id of their own,
    /// or a reconnect would resume from the wrong place.
    #[tokio::test]
    async fn the_last_event_id_persists_across_id_less_frames() {
        let mut reader = reader(vec!["id: 7\ndata: {}\n\n", "data: {}\n\n"]);
        reader.next_event().await.unwrap().unwrap();
        assert_eq!(reader.last_event_id(), Some("7"));
        reader.next_event().await.unwrap().unwrap();
        assert_eq!(
            reader.last_event_id(),
            Some("7"),
            "not cleared by the second"
        );
    }

    /// Keep-alive comments must not surface as events, and must not be mistaken
    /// for the end of the stream.
    #[tokio::test]
    async fn keepalives_are_skipped_without_ending_the_stream() {
        let mut reader = reader(vec![": ping\n\n", ": ping\n\n", "data: {\"a\":1}\n\n"]);
        let event = reader.next_event().await.unwrap().expect("the real frame");
        assert_eq!(event.data, "{\"a\":1}");
    }

    /// A hostile server streaming without ever closing a frame must trip the cap
    /// rather than growing the buffer until the process dies.
    #[tokio::test]
    async fn an_unterminated_flood_trips_the_frame_cap() {
        let mut reader = reader(vec!["data: ", "aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc"])
            .with_max_frame_bytes(16);
        let err = reader
            .next_event()
            .await
            .expect_err("must refuse to buffer");
        assert!(err.to_string().contains("exceeded"), "got {err}");
    }

    /// Silence past the idle timeout ends the stream. A *total* timeout here
    /// would instead kill a healthy subscription on a fixed schedule.
    #[tokio::test]
    async fn silence_past_the_idle_timeout_is_an_error() {
        let mut reader = SseReader::from_stream(futures::stream::pending())
            .with_idle_timeout(Duration::from_millis(50));
        let err = reader.next_event().await.expect_err("silence must end it");
        assert!(err.to_string().contains("dead"), "got {err}");
    }

    /// A stream that stays quiet but is still alive must NOT be torn down: the
    /// normal state of a connection whose tools never change is silence.
    #[tokio::test]
    async fn a_slow_but_live_stream_is_not_torn_down() {
        let chunks = futures::stream::once(async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok(b"data: {\"a\":1}\n\n".to_vec())
        });
        let mut reader = SseReader::from_stream(chunks).with_idle_timeout(Duration::from_secs(5));
        let event = reader
            .next_event()
            .await
            .unwrap()
            .expect("arrives late but arrives");
        assert_eq!(event.data, "{\"a\":1}");
    }
}
