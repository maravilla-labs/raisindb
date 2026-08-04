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

//! Streamable HTTP transport for the outbound MCP client.
//!
//! One JSON-RPC message per POST. The peer may answer with either
//! `application/json` (a single response) or `text/event-stream` (the response
//! wrapped in SSE frames, possibly preceded by notifications) — a conforming
//! client MUST accept both, and which one arrives is the server's choice, not
//! ours. RaisinDB's own server only ever replies with JSON, so the SSE branch
//! here is exercised solely against third-party servers and the test mock.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use serde_json::{json, Value};
use url::Url;

use super::error::{RemoteToolError, Result};
use super::notification::{notification_parts, NotificationSink};
use crate::protocol::{JsonRpcResponse, JSONRPC_VERSION};

/// Header carrying the negotiated MCP revision on every request.
const HEADER_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";
/// Header carrying the server-assigned session id, when the server issues one.
const HEADER_SESSION_ID: &str = "Mcp-Session-Id";

/// Default cap on a single response body. A remote server is untrusted: without
/// a cap, one reply can exhaust the process.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Extra headers injected on every request (auth, and nothing else).
pub type ExtraHeaders = Vec<(String, String)>;

/// A Streamable HTTP connection to one remote MCP server.
pub struct StreamableHttpTransport {
    http: reqwest::Client,
    url: Url,
    timeout: Duration,
    max_response_bytes: usize,
    /// Server-assigned session id, replayed on subsequent requests.
    session_id: Mutex<Option<String>>,
    /// Revision sent in `MCP-Protocol-Version`; set once negotiated.
    protocol_version: Mutex<Option<String>>,
    /// Where server-initiated notifications go. `None` drops them, which is
    /// what every caller did before the sink existed.
    notifications: Option<Arc<dyn NotificationSink>>,
}

impl StreamableHttpTransport {
    /// Build a transport over a shared `reqwest::Client`.
    ///
    /// The client is shared process-wide on purpose — a per-connection client
    /// means a per-connection connection pool and TLS handshake cache, which is
    /// the same mistake `shared_http_client()` documents on the functions side.
    pub fn new(http: reqwest::Client, url: Url, timeout: Duration) -> Self {
        Self {
            http,
            url,
            timeout,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            session_id: Mutex::new(None),
            protocol_version: Mutex::new(None),
            notifications: None,
        }
    }

    /// Override the response byte cap.
    pub fn with_max_response_bytes(mut self, max: usize) -> Self {
        self.max_response_bytes = max;
        self
    }

    /// Route server-initiated notifications to `sink` instead of dropping them.
    pub fn with_notification_sink(mut self, sink: Arc<dyn NotificationSink>) -> Self {
        self.notifications = Some(sink);
        self
    }

    /// The endpoint this transport talks to.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Record the revision to advertise on subsequent requests.
    pub fn set_protocol_version(&self, version: impl Into<String>) {
        *self.protocol_version.lock().expect("poisoned") = Some(version.into());
    }

    /// The current session id, if the server issued one.
    pub fn session_id(&self) -> Option<String> {
        self.session_id.lock().expect("poisoned").clone()
    }

    /// Forget the session so the next call re-initializes.
    pub fn clear_session(&self) {
        *self.session_id.lock().expect("poisoned") = None;
    }

    /// Send a JSON-RPC request and return its `result`.
    pub async fn request(
        &self,
        id: u64,
        method: &str,
        params: Value,
        auth: &ExtraHeaders,
    ) -> Result<Value> {
        let body = json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        });
        let text = self.post(&body, auth).await?;
        let response = self.decode(&text, id)?;

        if let Some(err) = response.error {
            return Err(RemoteToolError::from_jsonrpc(err.code, &err.message));
        }
        Ok(response.result.unwrap_or(Value::Null))
    }

    /// Send a JSON-RPC notification (no `id`, no response expected).
    pub async fn notify(&self, method: &str, params: Value, auth: &ExtraHeaders) -> Result<()> {
        let body = json!({ "jsonrpc": JSONRPC_VERSION, "method": method, "params": params });
        self.post(&body, auth).await.map(|_| ())
    }

    /// Open a long-lived stream, returning the response unread.
    ///
    /// `body` decides the method: `Some` POSTs it (the 2026-07-28
    /// `subscriptions/listen`), `None` issues the legacy GET. Either way the
    /// caller drives an [`SseReader`](super::stream::SseReader) over the result.
    ///
    /// **No `.timeout()` is set**, and that is the point. The request timeout
    /// used by `post` is total-elapsed, so applying it here would sever a
    /// perfectly healthy subscription every 30 seconds. Liveness on a stream is
    /// the reader's idle timeout instead.
    pub async fn open_stream(
        &self,
        body: Option<&Value>,
        auth: &ExtraHeaders,
        last_event_id: Option<&str>,
    ) -> Result<reqwest::Response> {
        let mut req = match body {
            Some(body) => self
                .http
                .post(self.url.clone())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(body),
            None => self.http.get(self.url.clone()),
        }
        .header(reqwest::header::ACCEPT, "text/event-stream");

        req = self.apply_session_headers(req, auth);
        if let Some(id) = last_event_id {
            // Resume where the dropped stream left off, so a reconnect does not
            // silently lose whatever was emitted during the gap.
            req = req.header("Last-Event-ID", id);
        }

        let response = req.send().await?;
        let status = response.status();
        self.adopt_session(&response);

        if !status.is_success() {
            if status.as_u16() == 404 && self.session_id().is_some() {
                self.clear_session();
                return Err(RemoteToolError::SessionExpired);
            }
            // 405 on the GET is how a server that does not offer a listening
            // stream says so. It is a capability answer, not a fault.
            let body = String::new();
            return Err(RemoteToolError::from_status(status.as_u16(), &body, None));
        }
        Ok(response)
    }

    /// Attach the protocol version, session id and auth headers.
    fn apply_session_headers(
        &self,
        mut req: reqwest::RequestBuilder,
        auth: &ExtraHeaders,
    ) -> reqwest::RequestBuilder {
        if let Some(version) = self.protocol_version.lock().expect("poisoned").as_ref() {
            req = req.header(HEADER_PROTOCOL_VERSION, version);
        }
        if let Some(session) = self.session_id.lock().expect("poisoned").as_ref() {
            req = req.header(HEADER_SESSION_ID, session);
        }
        for (name, value) in auth {
            req = req.header(name.as_str(), value.as_str());
        }
        req
    }

    /// Record a session id the server assigned or rotated on this response.
    fn adopt_session(&self, response: &reqwest::Response) {
        if let Some(session) = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|v| v.to_str().ok())
        {
            *self.session_id.lock().expect("poisoned") = Some(session.to_string());
        }
    }

    /// POST one message and return the raw response body.
    async fn post(&self, body: &Value, auth: &ExtraHeaders) -> Result<String> {
        let mut req = self
            .http
            .post(self.url.clone())
            .timeout(self.timeout)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            // Both are required: the server picks the response encoding.
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .json(body);

        if let Some(version) = self.protocol_version.lock().expect("poisoned").as_ref() {
            req = req.header(HEADER_PROTOCOL_VERSION, version);
        }
        let sent_session = self.session_id.lock().expect("poisoned").clone();
        if let Some(session) = sent_session.as_ref() {
            req = req.header(HEADER_SESSION_ID, session);
        }
        for (name, value) in auth {
            req = req.header(name.as_str(), value.as_str());
        }

        let response = req.send().await?;
        let status = response.status();

        // A server may assign or rotate the session id on any response.
        if let Some(session) = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|v| v.to_str().ok())
        {
            *self.session_id.lock().expect("poisoned") = Some(session.to_string());
        }

        if !status.is_success() {
            // 404 against a request that carried a session id means the server
            // dropped that session, NOT that the endpoint is gone. Conflating
            // the two turns a recoverable reconnect into a permanent
            // `config_error` that an operator has to clear by hand.
            if status.as_u16() == 404 && sent_session.is_some() {
                self.clear_session();
                return Err(RemoteToolError::SessionExpired);
            }
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let body = self.read_capped(response).await.unwrap_or_default();
            return Err(RemoteToolError::from_status(
                status.as_u16(),
                &body,
                retry_after,
            ));
        }

        self.read_capped(response).await
    }

    /// Read a response body, refusing to buffer more than the cap.
    ///
    /// Enforced while streaming rather than after `bytes()`, or the cap would
    /// only report an exhaustion that already happened.
    async fn read_capped(&self, response: reqwest::Response) -> Result<String> {
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if buf.len() + chunk.len() > self.max_response_bytes {
                return Err(RemoteToolError::Transient(format!(
                    "response exceeded {} bytes",
                    self.max_response_bytes
                )));
            }
            buf.extend_from_slice(&chunk);
        }
        String::from_utf8(buf)
            .map_err(|e| RemoteToolError::Transient(format!("response is not utf-8: {e}")))
    }

    /// Decode a response body that may be raw JSON or an SSE stream.
    fn decode(&self, text: &str, id: u64) -> Result<JsonRpcResponse> {
        let trimmed = text.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return parse_message(trimmed, id)?.ok_or_else(|| {
                RemoteToolError::Protocol(format!("no response for request id {id}"))
            });
        }
        // SSE: the reply to our request is the first `data:` frame carrying a
        // response with our id. Earlier frames may be notifications, which are
        // routed to the sink rather than mistaken for the answer — or, before
        // the sink existed, silently dropped.
        for frame in sse_data_frames(text) {
            if let Some(response) = self.dispatch_frame(&frame, id)? {
                return Ok(response);
            }
        }
        Err(RemoteToolError::Protocol(format!(
            "no response for request id {id} in event stream"
        )))
    }

    /// Route one SSE frame: our response is returned, a notification is
    /// delivered to the sink, anything else is ignored.
    ///
    /// A server may legitimately put `notifications/tools/list_changed` ahead of
    /// the reply to an ordinary `tools/call`, which is free freshness the
    /// response scan alone throws away.
    fn dispatch_frame(&self, frame: &str, id: u64) -> Result<Option<JsonRpcResponse>> {
        let value: Value = serde_json::from_str(frame)
            .map_err(|e| RemoteToolError::Protocol(format!("malformed json-rpc message: {e}")))?;

        if value.get("id").and_then(Value::as_u64) == Some(id) {
            return serde_json::from_value(value).map(Some).map_err(|e| {
                RemoteToolError::Protocol(format!("malformed json-rpc response: {e}"))
            });
        }

        if let (Some(sink), Some((method, params))) =
            (self.notifications.as_ref(), notification_parts(&value))
        {
            // Never fallible and never awaited: this runs inside an in-flight
            // request, which on the POST route is an agent's `tools/call`.
            sink.on_notification(method, params);
        }
        Ok(None)
    }
}

/// Parse one JSON message, returning it only when it answers `id`.
fn parse_message(text: &str, id: u64) -> Result<Option<JsonRpcResponse>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| RemoteToolError::Protocol(format!("malformed json-rpc message: {e}")))?;
    // Notifications carry no `id`; responses to other requests carry a
    // different one. Neither is our answer.
    if value.get("id").and_then(Value::as_u64) != Some(id) {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|e| RemoteToolError::Protocol(format!("malformed json-rpc response: {e}")))
}

/// Extract the `data:` payloads from an SSE body, one string per event.
///
/// Multi-line `data:` fields are joined with `\n` per the EventSource spec.
fn sse_data_frames(text: &str) -> Vec<String> {
    let mut frames = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            if !current.is_empty() {
                frames.push(current.join("\n"));
                current.clear();
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            current.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
        // `event:`, `id:`, `retry:` and comments carry nothing we need.
    }
    if !current.is_empty() {
        frames.push(current.join("\n"));
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_sse_frames() {
        let body = "event: message\ndata: {\"a\":1}\n\ndata: {\"b\":2}\n\n";
        assert_eq!(sse_data_frames(body), vec!["{\"a\":1}", "{\"b\":2}"]);
    }

    #[test]
    fn joins_multiline_sse_data() {
        let body = "data: {\"a\":\ndata: 1}\n\n";
        assert_eq!(sse_data_frames(body), vec!["{\"a\":\n1}"]);
    }

    #[test]
    fn skips_notifications_before_the_answer() {
        let transport = test_transport();
        let body = concat!(
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}\n\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n",
        );
        let response = transport.decode(body, 7).expect("must find the answer");
        assert_eq!(response.result.unwrap(), json!({ "ok": true }));
    }

    #[test]
    fn ignores_a_response_to_a_different_request() {
        let transport = test_transport();
        let body = "data: {\"jsonrpc\":\"2.0\",\"id\":6,\"result\":{}}\n\n";
        assert!(transport.decode(body, 7).is_err());
    }

    #[test]
    fn decodes_plain_json_bodies() {
        let transport = test_transport();
        let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":1}}";
        assert_eq!(
            transport.decode(body, 1).unwrap().result.unwrap(),
            json!({ "ok": 1 })
        );
    }

    /// Records what a sink was handed, so the tests can assert on it.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl NotificationSink for Recorder {
        fn on_notification(&self, method: &str, _params: &Value) {
            self.0.lock().expect("poisoned").push(method.to_string());
        }
    }

    /// THE change: a notification riding along with an ordinary response used to
    /// be dropped, because it carried an id that was not ours and nothing else
    /// looked at it.
    #[test]
    fn a_notification_beside_the_answer_reaches_the_sink() {
        let sink = Arc::new(Recorder::default());
        let transport = test_transport().with_notification_sink(sink.clone());
        let body = concat!(
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n",
        );

        let response = transport.decode(body, 7).expect("the answer still decodes");
        assert_eq!(response.result.unwrap(), json!({ "ok": true }));
        assert_eq!(
            sink.0.lock().unwrap().as_slice(),
            ["notifications/tools/list_changed"]
        );
    }

    /// Frames AFTER the answer are never reached — `decode` returns at the
    /// response. A server that wants a notification seen must send it first,
    /// which is what the spec's ordering guarantees.
    #[test]
    fn a_notification_after_the_answer_is_not_delivered() {
        let sink = Arc::new(Recorder::default());
        let transport = test_transport().with_notification_sink(sink.clone());
        let body = concat!(
            "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}\n\n",
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n",
        );

        transport.decode(body, 7).expect("the answer decodes");
        assert!(sink.0.lock().unwrap().is_empty());
    }

    /// A server-to-client REQUEST carries both `method` and `id`. Handing it to
    /// the sink would treat something the server is waiting on as fire-and-forget.
    #[test]
    fn a_server_initiated_request_does_not_reach_the_sink() {
        let sink = Arc::new(Recorder::default());
        let transport = test_transport().with_notification_sink(sink.clone());
        let body = concat!(
            "data: {\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"sampling/createMessage\"}\n\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}\n\n",
        );

        transport.decode(body, 7).expect("the answer decodes");
        assert!(sink.0.lock().unwrap().is_empty());
    }

    /// The blast radius of the change must be zero for every existing caller:
    /// with no sink installed, decode behaves exactly as it did before.
    #[test]
    fn a_sinkless_transport_still_skips_notifications() {
        let transport = test_transport();
        let body = concat!(
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n",
        );
        assert_eq!(
            transport.decode(body, 7).unwrap().result.unwrap(),
            json!({ "ok": true })
        );
    }

    fn test_transport() -> StreamableHttpTransport {
        StreamableHttpTransport::new(
            reqwest::Client::new(),
            Url::parse("https://example.test/mcp").unwrap(),
            Duration::from_secs(5),
        )
    }
}
