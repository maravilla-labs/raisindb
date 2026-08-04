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

//! End-to-end client tests: real HTTP, real framing, against the mock server.

use std::time::Duration;

use raisin_mcp_protocol::client::{
    concat_text, McpClientSession, RemoteToolError, StreamableHttpTransport,
};
use serde_json::json;
use url::Url;

use crate::mock::{Framing, MockServer, Script};

/// No credentials — spelled out so the element type is inferable.
fn no_auth() -> raisin_mcp_protocol::client::ExtraHeaders {
    Vec::new()
}

fn session(url: &str) -> McpClientSession {
    McpClientSession::new(StreamableHttpTransport::new(
        reqwest::Client::new(),
        Url::parse(url).expect("mock url"),
        Duration::from_secs(10),
    ))
}

#[tokio::test]
async fn negotiates_with_initialize_first() {
    let server = MockServer::start(Script::default(), Framing::Json).await;
    let session = session(server.url());

    let handshake = session.handshake(&no_auth()).await.expect("handshake");

    assert_eq!(handshake.protocol_version, "2025-06-18");
    assert_eq!(handshake.server_info.unwrap().name, "mock");
    // `initialize` must be the FIRST method on the wire. Leading with
    // `server/discover` would fail against every server deployed today.
    assert_eq!(
        server.methods().first().map(String::as_str),
        Some("initialize")
    );
}

#[tokio::test]
async fn falls_back_to_discover_when_initialize_is_absent() {
    let script = Script {
        no_initialize: true,
        ..Script::default()
    };
    let server = MockServer::start(script, Framing::Json).await;

    let handshake = session(server.url())
        .handshake(&no_auth())
        .await
        .expect("handshake");

    assert_eq!(handshake.protocol_version, "2026-07-28");
    assert_eq!(
        server.methods(),
        vec!["initialize".to_string(), "server/discover".to_string()]
    );
}

#[tokio::test]
async fn handshake_happens_once_across_many_calls() {
    let server = MockServer::start(Script::default(), Framing::Json).await;
    let session = session(server.url());

    session.list_tools(&no_auth()).await.expect("list");
    session.list_tools(&no_auth()).await.expect("list again");

    let initializes = server
        .methods()
        .iter()
        .filter(|m| *m == "initialize")
        .count();
    assert_eq!(initializes, 1, "the handshake must be cached, not repeated");
}

#[tokio::test]
async fn lists_tools_from_a_legacy_shaped_response() {
    let server = MockServer::start(Script::default(), Framing::Json).await;

    let tools = session(server.url())
        .list_tools(&no_auth())
        .await
        .expect("list");

    // The mock answers WITHOUT resultType/ttlMs/cacheScope. Before the client
    // fixes, this response could not be deserialized at all.
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["search_issues", "create_issue"]
    );
    assert_eq!(tools[1].description, None);
}

#[tokio::test]
async fn follows_tool_list_pagination_to_the_end() {
    let script = Script {
        paginate_tools: true,
        ..Script::default()
    };
    let server = MockServer::start(script, Framing::Json).await;

    let tools = session(server.url())
        .list_tools(&no_auth())
        .await
        .expect("list");

    // Stopping at page one is a silent truncation: the tools simply would not
    // exist as far as any agent is concerned.
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["page_one", "page_two"]
    );
}

#[tokio::test]
async fn refuses_a_repeating_pagination_cursor() {
    let script = Script {
        repeat_cursor: true,
        ..Script::default()
    };
    let server = MockServer::start(script, Framing::Json).await;

    let err = session(server.url())
        .list_tools(&no_auth())
        .await
        .unwrap_err();

    assert!(
        matches!(err, RemoteToolError::Protocol(_)),
        "a repeated cursor must fail fast, not accumulate duplicates: {err:?}"
    );
}

#[tokio::test]
async fn works_over_sse_framing() {
    let server = MockServer::start(Script::default(), Framing::Sse).await;

    let tools = session(server.url())
        .list_tools(&no_auth())
        .await
        .expect("list");

    // The mock precedes every reply with a notification frame; the client must
    // skip it rather than mistake it for the answer.
    assert_eq!(tools.len(), 2);
}

#[tokio::test]
async fn calls_a_tool_and_maps_the_result() {
    let server = MockServer::start(Script::default(), Framing::Json).await;

    let result = session(server.url())
        .call_tool("search_issues", json!({ "q": "bug" }), &no_auth())
        .await
        .expect("call");

    assert!(!result.is_error);
    assert_eq!(concat_text(&result), "{\"ok\":true}");
    // The REMOTE name and the caller's arguments must arrive verbatim.
    assert_eq!(server.last_tool_arguments(), Some(json!({ "q": "bug" })));
}

#[tokio::test]
async fn sends_the_negotiated_protocol_version_header() {
    let server = MockServer::start(Script::default(), Framing::Json).await;

    session(server.url())
        .list_tools(&no_auth())
        .await
        .expect("list");

    assert_eq!(
        server.last_header("mcp-protocol-version").as_deref(),
        Some("2025-06-18")
    );
}

#[tokio::test]
async fn replays_once_when_the_server_drops_the_session() {
    let script = Script {
        expire_session_once: true,
        ..Script::default()
    };
    let server = MockServer::start(script, Framing::Json).await;

    // A dropped session must recover transparently, not surface as an error.
    let tools = session(server.url())
        .list_tools(&no_auth())
        .await
        .expect("list");

    assert_eq!(tools.len(), 2);
    let initializes = server
        .methods()
        .iter()
        .filter(|m| *m == "initialize")
        .count();
    assert_eq!(initializes, 2, "the client must re-initialize exactly once");
}

#[tokio::test]
async fn surfaces_a_401_as_auth_expired() {
    let script = Script {
        require_auth: true,
        ..Script::default()
    };
    let server = MockServer::start(script, Framing::Json).await;

    let err = session(server.url())
        .list_tools(&no_auth())
        .await
        .unwrap_err();

    assert_eq!(err.code(), "auth_expired");
    assert!(
        !err.is_retryable(),
        "re-auth needs an operator, not a retry"
    );
}

#[tokio::test]
async fn an_injected_credential_satisfies_the_challenge() {
    let script = Script {
        require_auth: true,
        ..Script::default()
    };
    let server = MockServer::start(script, Framing::Json).await;
    let auth = vec![("Authorization".to_string(), "Bearer t0ken".to_string())];

    let tools = session(server.url()).list_tools(&auth).await.expect("list");

    assert_eq!(tools.len(), 2);
    assert_eq!(
        server.last_header("authorization").as_deref(),
        Some("Bearer t0ken")
    );
}
