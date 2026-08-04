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

//! Holding a notification stream open.
//!
//! How the stream is *opened* changed between revisions, and both are live
//! concerns:
//!
//! | Revision | Opened with | Liveness |
//! |---|---|---|
//! | 2026-07-28 | `subscriptions/listen` (a POST) with an opt-in filter | `notifications/subscriptions/acknowledged` MUST arrive first |
//! | 2025-06-18 and earlier | a long-lived GET | none — the socket staying up is all there is |
//!
//! Both are needed. No third-party server speaks 2026-07-28 yet
//! (`protocol.rs:44-49` records that even Claude Desktop lists 2025-11-25
//! downward), so the legacy GET is what works against anything real today. The
//! one peer that does speak it is RaisinDB itself.
//!
//! **Only `toolsListChanged` is requested.** Prompts and resource subscriptions
//! are deliberately out of scope, and asking for a type we would then ignore
//! would make the server do work for nothing.

use serde_json::{json, Value};

use super::error::{RemoteToolError, Result};
use super::notification::notification_parts;
use super::stream::{decode_message, SseReader};
use super::transport::{ExtraHeaders, StreamableHttpTransport};
use crate::protocol::{META_SUBSCRIPTION_ID, RESULT_TYPE_COMPLETE};

/// Method that opens a 2026-07-28 notification stream.
pub const SUBSCRIPTIONS_LISTEN: &str = "subscriptions/listen";
/// The acknowledgement a modern server must send first.
pub const SUBSCRIPTIONS_ACKNOWLEDGED: &str = "notifications/subscriptions/acknowledged";

/// Why a subscription stream ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEnd {
    /// The server sent the empty `result` for our listen request.
    ///
    /// Distinct from a drop on purpose: this is the server saying "I am closing
    /// this cleanly", typically a shutdown. Reconnect calmly rather than
    /// treating it as a failure and escalating the backoff.
    Graceful,
    /// The transport closed without a closing response.
    Dropped,
}

/// A live notification stream and what the server agreed to send on it.
pub struct SubscriptionStream {
    reader: SseReader,
    /// JSON-RPC id of the listen request, absent on the legacy GET.
    request_id: Option<u64>,
    /// Whether the server confirmed it will announce tool changes.
    tools_granted: bool,
}

impl SubscriptionStream {
    /// Whether the server agreed to announce tool-list changes.
    ///
    /// On the legacy GET this is the handshake capability rather than a per-
    /// stream grant, because that revision has nothing to negotiate.
    pub fn tools_granted(&self) -> bool {
        self.tools_granted
    }

    /// The id to resume from after a drop.
    pub fn last_event_id(&self) -> Option<&str> {
        self.reader.last_event_id()
    }

    /// Next notification as `(method, params)`, or why the stream ended.
    ///
    /// Anything that is not a notification — a stray response, a server-to-client
    /// request — is skipped rather than surfaced. Only the closing response for
    /// our own listen id is treated as an ending.
    pub async fn next_notification(
        &mut self,
    ) -> Result<std::result::Result<(String, Value), StreamEnd>> {
        loop {
            let Some(event) = self.reader.next_event().await? else {
                return Ok(Err(StreamEnd::Dropped));
            };
            let message = decode_message(&event)?;

            if self.is_closing_response(&message) {
                return Ok(Err(StreamEnd::Graceful));
            }
            if let Some((method, params)) = notification_parts(&message) {
                // The acknowledgement is bookkeeping, not an event to act on.
                if method == SUBSCRIPTIONS_ACKNOWLEDGED {
                    continue;
                }
                return Ok(Ok((method.to_string(), params.clone())));
            }
        }
    }

    /// Whether this message is the graceful close of *our* subscription.
    fn is_closing_response(&self, message: &Value) -> bool {
        let Some(id) = self.request_id else {
            return false;
        };
        message.get("id").and_then(Value::as_u64) == Some(id)
            && message
                .get("result")
                .and_then(|r| r.get("resultType"))
                .and_then(Value::as_str)
                == Some(RESULT_TYPE_COMPLETE)
    }
}

/// Open a notification stream, choosing the dialect from the negotiated revision.
///
/// `modern` comes from
/// [`ServerHandshake::uses_listen_subscriptions`](super::session::ServerHandshake::uses_listen_subscriptions).
pub async fn open(
    transport: &StreamableHttpTransport,
    auth: &ExtraHeaders,
    modern: bool,
    last_event_id: Option<&str>,
) -> Result<SubscriptionStream> {
    if modern {
        open_listen(transport, auth, last_event_id).await
    } else {
        open_legacy_get(transport, auth, last_event_id).await
    }
}

/// 2026-07-28: `subscriptions/listen`, acknowledgement first.
async fn open_listen(
    transport: &StreamableHttpTransport,
    auth: &ExtraHeaders,
    last_event_id: Option<&str>,
) -> Result<SubscriptionStream> {
    // Any id works as long as we can recognise it later; the spec makes it the
    // subscription's identity, echoed in every frame's `_meta`.
    const LISTEN_ID: u64 = 1;

    let body = json!({
        "jsonrpc": crate::protocol::JSONRPC_VERSION,
        "id": LISTEN_ID,
        "method": SUBSCRIPTIONS_LISTEN,
        "params": { "notifications": { "toolsListChanged": true } },
    });

    let response = transport
        .open_stream(Some(&body), auth, last_event_id)
        .await?;
    let mut reader = SseReader::new(response);

    // The acknowledgement is MANDATORY and MUST be first. Its absence is a
    // protocol violation, not a slow server — waiting patiently for a stream
    // that will never be confirmed is the failure mode this check exists to
    // avoid.
    let granted = read_acknowledgement(&mut reader, LISTEN_ID).await?;

    Ok(SubscriptionStream {
        reader,
        request_id: Some(LISTEN_ID),
        tools_granted: granted,
    })
}

/// Consume the first frame and report whether `toolsListChanged` was granted.
async fn read_acknowledgement(reader: &mut SseReader, listen_id: u64) -> Result<bool> {
    let Some(event) = reader.next_event().await? else {
        return Err(RemoteToolError::Protocol(
            "the subscription stream closed before acknowledging".into(),
        ));
    };
    let message = decode_message(&event)?;

    let Some((method, params)) = notification_parts(&message) else {
        return Err(RemoteToolError::Protocol(format!(
            "expected {SUBSCRIPTIONS_ACKNOWLEDGED} first, got {}",
            describe(&message)
        )));
    };
    if method != SUBSCRIPTIONS_ACKNOWLEDGED {
        return Err(RemoteToolError::Protocol(format!(
            "expected {SUBSCRIPTIONS_ACKNOWLEDGED} first, got {method}"
        )));
    }

    // Correlate: on a shared channel another subscription's frames may be
    // interleaved ahead of ours, so an acknowledgement carrying somebody else's
    // id is not ours to read.
    if let Some(id) = subscription_id(params) {
        if id != listen_id {
            return Err(RemoteToolError::Protocol(format!(
                "acknowledgement is for subscription {id}, not {listen_id}"
            )));
        }
    }

    // The granted filter is the SUBSET the server agreed to honour, which may be
    // narrower than what we asked for. A server that silently declined
    // `toolsListChanged` would otherwise leave us holding a stream forever
    // waiting for a notification it is never going to send.
    Ok(params
        .get("notifications")
        .and_then(|n| n.get("toolsListChanged"))
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// Legacy: a long-lived GET, with nothing to negotiate.
async fn open_legacy_get(
    transport: &StreamableHttpTransport,
    auth: &ExtraHeaders,
    last_event_id: Option<&str>,
) -> Result<SubscriptionStream> {
    let response = transport.open_stream(None, auth, last_event_id).await?;
    Ok(SubscriptionStream {
        reader: SseReader::new(response),
        request_id: None,
        // This revision has no per-stream grant. The caller has already gated on
        // `capabilities.tools.listChanged`, which is the only promise available.
        tools_granted: true,
    })
}

/// The subscription id carried in a frame's `_meta`.
fn subscription_id(params: &Value) -> Option<u64> {
    params
        .get("_meta")
        .and_then(|m| m.get(META_SUBSCRIPTION_ID))
        .and_then(Value::as_u64)
}

/// A short description of an unexpected message, for an error.
fn describe(message: &Value) -> String {
    if let Some(method) = message.get("method").and_then(Value::as_str) {
        return format!("request `{method}`");
    }
    if message.get("error").is_some() {
        return "an error response".to_string();
    }
    if message.get("result").is_some() {
        return "a result".to_string();
    }
    "an unrecognised message".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::notification::TOOLS_LIST_CHANGED;

    fn stream(chunks: Vec<&'static str>, request_id: Option<u64>) -> SubscriptionStream {
        SubscriptionStream {
            reader: SseReader::from_stream(futures::stream::iter(
                chunks.into_iter().map(|c| Ok(c.as_bytes().to_vec())),
            )),
            request_id,
            tools_granted: true,
        }
    }

    async fn read_ack(chunks: Vec<&'static str>) -> Result<bool> {
        let mut reader = SseReader::from_stream(futures::stream::iter(
            chunks.into_iter().map(|c| Ok(c.as_bytes().to_vec())),
        ));
        read_acknowledgement(&mut reader, 1).await
    }

    #[tokio::test]
    async fn an_acknowledgement_granting_tools_is_accepted() {
        let granted = read_ack(vec![concat!(
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/subscriptions/acknowledged\",",
            "\"params\":{\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":1},",
            "\"notifications\":{\"toolsListChanged\":true}}}\n\n"
        )])
        .await
        .expect("a well-formed acknowledgement");
        assert!(granted);
    }

    /// A server may honour a NARROWER filter than requested. Reporting that
    /// truthfully is what lets the caller log "this server declined" instead of
    /// waiting forever on a stream that will stay silent.
    #[tokio::test]
    async fn a_declined_tools_grant_is_reported_not_assumed() {
        let granted = read_ack(vec![concat!(
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/subscriptions/acknowledged\",",
            "\"params\":{\"notifications\":{\"resourcesListChanged\":true}}}\n\n"
        )])
        .await
        .expect("still a valid acknowledgement");
        assert!(!granted, "an omitted type means NOT granted");
    }

    /// The spec makes the acknowledgement mandatory and first. A notification
    /// arriving ahead of it is a protocol violation, and treating it as merely
    /// "early" would mean trusting a stream the server never confirmed.
    #[tokio::test]
    async fn a_notification_before_the_acknowledgement_is_a_protocol_error() {
        let err = read_ack(vec![
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n",
        ])
        .await
        .expect_err("must refuse");
        assert!(matches!(err, RemoteToolError::Protocol(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn a_stream_closing_before_acknowledging_is_an_error() {
        let err = read_ack(vec![]).await.expect_err("must refuse");
        assert!(
            err.to_string().contains("before acknowledging"),
            "got {err}"
        );
    }

    /// Frames of another subscription may be interleaved ahead of ours.
    #[tokio::test]
    async fn an_acknowledgement_for_another_subscription_is_refused() {
        let err = read_ack(vec![concat!(
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/subscriptions/acknowledged\",",
            "\"params\":{\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":9},",
            "\"notifications\":{\"toolsListChanged\":true}}}\n\n"
        )])
        .await
        .expect_err("must not adopt somebody else's stream");
        assert!(err.to_string().contains("subscription 9"), "got {err}");
    }

    #[tokio::test]
    async fn notifications_are_yielded_in_order() {
        let mut s = stream(
            vec![
                "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n",
                "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/resources/updated\",\"params\":{\"uri\":\"x\"}}\n\n",
            ],
            Some(1),
        );
        assert_eq!(
            s.next_notification().await.unwrap().unwrap().0,
            TOOLS_LIST_CHANGED
        );
        assert_eq!(
            s.next_notification().await.unwrap().unwrap().0,
            "notifications/resources/updated"
        );
        assert_eq!(
            s.next_notification().await.unwrap().unwrap_err(),
            StreamEnd::Dropped
        );
    }

    /// Graceful closure and an abrupt drop must be distinguishable — only the
    /// latter deserves an escalating backoff.
    #[tokio::test]
    async fn the_closing_response_is_a_graceful_end() {
        let mut s = stream(
            vec![concat!(
                "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"resultType\":\"complete\"}}\n\n"
            )],
            Some(1),
        );
        assert_eq!(
            s.next_notification().await.unwrap().unwrap_err(),
            StreamEnd::Graceful
        );
    }

    /// A closing response for a DIFFERENT id closes somebody else's
    /// subscription, not ours.
    #[tokio::test]
    async fn a_closing_response_for_another_id_is_ignored() {
        let mut s = stream(
            vec![
                "data: {\"jsonrpc\":\"2.0\",\"id\":8,\"result\":{\"resultType\":\"complete\"}}\n\n",
                "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n",
            ],
            Some(1),
        );
        assert_eq!(
            s.next_notification().await.unwrap().unwrap().0,
            TOOLS_LIST_CHANGED
        );
    }

    /// A server-to-client request must not be mistaken for a notification.
    #[tokio::test]
    async fn a_server_request_on_the_stream_is_skipped() {
        let mut s = stream(
            vec![
                "data: {\"jsonrpc\":\"2.0\",\"id\":50,\"method\":\"sampling/createMessage\"}\n\n",
                "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n",
            ],
            Some(1),
        );
        assert_eq!(
            s.next_notification().await.unwrap().unwrap().0,
            TOOLS_LIST_CHANGED
        );
    }
}
