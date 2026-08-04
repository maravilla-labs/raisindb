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

//! A scriptable remote MCP server for client tests.
//!
//! Hand-rolled over a `TcpListener` rather than built on axum on purpose: this
//! crate's test target would otherwise link a second HTTP server stack for the
//! sake of four canned responses, and CLAUDE.md is explicit that every test
//! binary statically includes its whole dependency graph.
//!
//! It speaks only as much HTTP/1.1 as `reqwest` needs — request line, headers,
//! `Content-Length` body, keep-alive — and exists to exercise the cases a real
//! server puts in front of the client: SSE-framed replies, paginated
//! `tools/list`, a 401 challenge, and a dropped session.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// How the mock frames its replies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Framing {
    /// `application/json` — a bare JSON-RPC response.
    Json,
    /// `text/event-stream` — the response wrapped in an SSE frame, preceded by
    /// an unrelated notification the client must skip.
    Sse,
}

/// Scripted behaviours a test can turn on.
#[derive(Debug, Default, Clone)]
pub struct Script {
    /// Require `Authorization`; answer 401 + RFC 9728 challenge without it.
    pub require_auth: bool,
    /// Issue a session id and reject the request after `initialize` once with a
    /// 404, forcing the client to re-initialize and replay.
    pub expire_session_once: bool,
    /// Split `tools/list` across two pages.
    pub paginate_tools: bool,
    /// Answer `initialize` with "method not found", as a 2026-07-28-only server
    /// would, so the client falls back to `server/discover`.
    pub no_initialize: bool,
    /// Return the same cursor forever.
    pub repeat_cursor: bool,
}

/// Shared mutable state so assertions can inspect what the client sent.
#[derive(Default)]
pub struct Recorder {
    /// Every JSON-RPC method received, in order.
    pub methods: Vec<String>,
    /// Headers of the most recent request, lowercased names.
    pub last_headers: HashMap<String, String>,
    /// Arguments of the most recent `tools/call`.
    pub last_tool_arguments: Option<Value>,
}

/// A running mock MCP server.
pub struct MockServer {
    url: String,
    recorder: Arc<Mutex<Recorder>>,
    calls: Arc<AtomicUsize>,
}

impl MockServer {
    /// Start a mock on an ephemeral port.
    pub async fn start(script: Script, framing: Framing) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let recorder = Arc::new(Mutex::new(Recorder::default()));
        let calls = Arc::new(AtomicUsize::new(0));

        let task_recorder = Arc::clone(&recorder);
        let task_calls = Arc::clone(&calls);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let recorder = Arc::clone(&task_recorder);
                let calls = Arc::clone(&task_calls);
                let script = script.clone();
                tokio::spawn(async move {
                    serve_connection(stream, script, framing, recorder, calls).await;
                });
            }
        });

        Self {
            url: format!("http://{addr}/mcp"),
            recorder,
            calls,
        }
    }

    /// Endpoint URL for the client under test.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Snapshot of the methods received so far.
    pub fn methods(&self) -> Vec<String> {
        self.recorder.lock().unwrap().methods.clone()
    }

    /// Value of a header on the most recent request (lowercased name).
    pub fn last_header(&self, name: &str) -> Option<String> {
        self.recorder
            .lock()
            .unwrap()
            .last_headers
            .get(name)
            .cloned()
    }

    /// Arguments of the most recent `tools/call`.
    pub fn last_tool_arguments(&self) -> Option<Value> {
        self.recorder.lock().unwrap().last_tool_arguments.clone()
    }

    /// Total requests served.
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

async fn serve_connection(
    mut stream: tokio::net::TcpStream,
    script: Script,
    framing: Framing,
    recorder: Arc<Mutex<Recorder>>,
    calls: Arc<AtomicUsize>,
) {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 4096];

    loop {
        // Read one full request: headers, then Content-Length bytes of body.
        let (headers, body) = loop {
            if let Some(parsed) = try_parse_request(&buf) {
                break parsed;
            }
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        };
        let consumed = headers.total_len + body.len();
        buf.drain(..consumed);

        calls.fetch_add(1, Ordering::SeqCst);
        let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let id = request.get("id").cloned();

        {
            let mut rec = recorder.lock().unwrap();
            rec.methods.push(method.clone());
            rec.last_headers = headers.map.clone();
            if method == "tools/call" {
                rec.last_tool_arguments = request
                    .get("params")
                    .and_then(|p| p.get("arguments"))
                    .cloned();
            }
        }

        let response = build_response(&script, framing, &method, id, &headers, &recorder);
        if stream.write_all(response.as_bytes()).await.is_err() {
            return;
        }
    }
}

fn build_response(
    script: &Script,
    framing: Framing,
    method: &str,
    id: Option<Value>,
    headers: &ParsedHeaders,
    recorder: &Arc<Mutex<Recorder>>,
) -> String {
    if script.require_auth && !headers.map.contains_key("authorization") {
        return http_response(
            401,
            "Unauthorized",
            &[(
                "WWW-Authenticate",
                "Bearer resource_metadata=\"https://auth.test/.well-known/oauth-protected-resource\"",
            )],
            "application/json",
            "{\"error\":\"unauthorized\"}",
        );
    }

    // A notification (no id) needs no body; 202 is what a real server sends.
    let Some(id) = id else {
        return http_response(202, "Accepted", &[], "application/json", "");
    };

    if script.expire_session_once
        && method != "initialize"
        && headers.map.contains_key("mcp-session-id")
    {
        let already = {
            let rec = recorder.lock().unwrap();
            rec.methods.iter().filter(|m| *m == "initialize").count() > 1
        };
        if !already {
            return http_response(404, "Not Found", &[], "application/json", "{}");
        }
    }

    let result = match method {
        "initialize" if script.no_initialize => {
            return jsonrpc_error(framing, id, -32601, "method not found: initialize")
        }
        "initialize" => json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "mock", "version": "1.0.0" },
            "instructions": "a mock server",
        }),
        "server/discover" => json!({
            "resultType": "complete",
            "supportedVersions": ["2026-07-28"],
            "capabilities": { "tools": { "listChanged": false } },
            "ttlMs": 0,
            "cacheScope": "private",
        }),
        "tools/list" => tools_page(script, headers),
        "tools/call" => json!({
            "content": [ { "type": "text", "text": "{\"ok\":true}" } ],
        }),
        other => return jsonrpc_error(framing, id, -32601, &format!("method not found: {other}")),
    };

    let body = json!({ "jsonrpc": "2.0", "id": id, "result": result });
    frame(framing, &body)
}

fn tools_page(script: &Script, headers: &ParsedHeaders) -> Value {
    let cursor = headers.cursor.clone();
    if script.repeat_cursor {
        return json!({ "tools": [ { "name": "loop", "inputSchema": {} } ], "nextCursor": "same" });
    }
    if !script.paginate_tools {
        // Deliberately the LEGACY shape: no resultType, ttlMs or cacheScope.
        return json!({
            "tools": [
                { "name": "search_issues", "description": "Search", "inputSchema": { "type": "object" } },
                { "name": "create_issue", "inputSchema": { "type": "object" } },
            ]
        });
    }
    match cursor.as_deref() {
        None => json!({
            "tools": [ { "name": "page_one", "inputSchema": {} } ],
            "nextCursor": "page-2",
        }),
        Some("page-2") => json!({ "tools": [ { "name": "page_two", "inputSchema": {} } ] }),
        Some(other) => json!({ "tools": [ { "name": other, "inputSchema": {} } ] }),
    }
}

fn jsonrpc_error(framing: Framing, id: Value, code: i32, message: &str) -> String {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    });
    frame(framing, &body)
}

/// Wrap a JSON-RPC body in the configured framing.
fn frame(framing: Framing, body: &Value) -> String {
    match framing {
        Framing::Json => http_response(
            200,
            "OK",
            &[("Mcp-Session-Id", "sess-1")],
            "application/json",
            &body.to_string(),
        ),
        Framing::Sse => {
            // A leading notification the client must skip before finding its
            // own answer — the ordering a real streaming server produces.
            let payload = format!(
                "data: {}\n\ndata: {}\n\n",
                json!({ "jsonrpc": "2.0", "method": "notifications/message" }),
                body
            );
            http_response(
                200,
                "OK",
                &[("Mcp-Session-Id", "sess-1")],
                "text/event-stream",
                &payload,
            )
        }
    }
}

fn http_response(
    status: u16,
    reason: &str,
    extra: &[(&str, &str)],
    content_type: &str,
    body: &str,
) -> String {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in extra {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Connection: keep-alive\r\n\r\n");
    head.push_str(body);
    head
}

struct ParsedHeaders {
    map: HashMap<String, String>,
    content_length: usize,
    total_len: usize,
    /// `params.cursor` of the request body, lifted here for convenience.
    cursor: Option<String>,
}

/// Parse a complete request, or `None` when more bytes are needed.
fn try_parse_request(buf: &[u8]) -> Option<(ParsedHeaders, Vec<u8>)> {
    let text = String::from_utf8_lossy(buf);
    let head_end = text.find("\r\n\r\n")? + 4;
    let mut map = HashMap::new();
    for line in text[..head_end].lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            map.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length = map
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if buf.len() < head_end + content_length {
        return None;
    }
    let body = buf[head_end..head_end + content_length].to_vec();
    let cursor = serde_json::from_slice::<Value>(&body).ok().and_then(|v| {
        v.get("params")
            .and_then(|p| p.get("cursor"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    Some((
        ParsedHeaders {
            map,
            content_length,
            total_len: head_end,
            cursor,
        },
        body,
    ))
}
