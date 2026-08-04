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

//! Where server-initiated notifications go.
//!
//! A remote server may push a notification at two moments, and both end up
//! here so there is one handling path rather than two:
//!
//! 1. **On the SSE body of any POST response.** Streamable HTTP lets a server
//!    wrap its reply in an event stream and put notifications ahead of it. The
//!    transport used to drop those frames on the floor — they carried an id
//!    that was not ours, so the response scan skipped them and nothing else
//!    looked. That is free freshness we were discarding.
//! 2. **On a held-open subscription stream**, which is the deliberate case.
//!
//! `notifications/tools/list_changed` carries **no params**. It means "re-list",
//! nothing more, so a sink's whole job is to schedule a re-list and return.

use serde_json::Value;

/// Method name for the tools-changed notification, in every revision.
pub const TOOLS_LIST_CHANGED: &str = "notifications/tools/list_changed";

/// Receives server-initiated notifications for one connection.
///
/// **Implementations must not block and must not await.** This is called from
/// the synchronous decode path of an in-flight request — on the POST route that
/// request is an agent's `tools/call`, and a sink that did real work there would
/// put a third party's bookkeeping on the latency path of a tool the model is
/// waiting for. Hand the work to a queue and return.
pub trait NotificationSink: Send + Sync {
    /// A notification arrived. `params` is `Null` when the server sent none.
    fn on_notification(&self, method: &str, params: &Value);
}

/// Whether a decoded JSON-RPC message is a server-initiated notification.
///
/// Notifications are exactly "has a `method`, has no `id`". A response carries
/// an `id` and no `method`; a server-to-client *request* (sampling, elicitation)
/// carries both, and must not be mistaken for one — answering a request by
/// dropping it is a hang on the server's side, not a no-op.
pub fn notification_parts(value: &Value) -> Option<(&str, &Value)> {
    if value.get("id").is_some() {
        return None;
    }
    let method = value.get("method").and_then(Value::as_str)?;
    Some((method, value.get("params").unwrap_or(&Value::Null)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_bare_notification_is_recognized() {
        let value = json!({ "jsonrpc": "2.0", "method": TOOLS_LIST_CHANGED });
        let (method, params) = notification_parts(&value).expect("must be a notification");
        assert_eq!(method, TOOLS_LIST_CHANGED);
        // The spec sends no params for this one; the sink must cope.
        assert_eq!(params, &Value::Null);
    }

    #[test]
    fn a_response_is_not_a_notification() {
        assert!(notification_parts(&json!({ "jsonrpc": "2.0", "id": 7, "result": {} })).is_none());
    }

    /// A server-to-client REQUEST has both `method` and `id`. Treating it as a
    /// notification would silently swallow something the server is waiting on.
    #[test]
    fn a_server_initiated_request_is_not_a_notification() {
        let value = json!({ "jsonrpc": "2.0", "id": 1, "method": "sampling/createMessage" });
        assert!(notification_parts(&value).is_none());
    }

    #[test]
    fn params_are_passed_through_when_present() {
        let value = json!({
            "jsonrpc": "2.0",
            "method": "notifications/resources/updated",
            "params": { "uri": "file:///x" },
        });
        let (_, params) = notification_parts(&value).unwrap();
        assert_eq!(params["uri"], "file:///x");
    }
}
