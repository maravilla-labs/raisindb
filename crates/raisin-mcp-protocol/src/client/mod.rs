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

//! Outbound MCP client — RaisinDB calling somebody else's server.
//!
//! The mirror of [`crate::dispatch`], which serves RaisinDB's own tools to
//! external clients. Both directions share [`crate::protocol`] deliberately: a
//! second copy of the JSON-RPC envelopes and the `tools/*` payloads is exactly
//! the kind of mirrored code path that drifts until the two disagree about the
//! wire format.
//!
//! What the client assumes, and why:
//!
//! - **Streamable HTTP only.** No stdio: spawning a subprocess inside the
//!   database would mean arbitrary process execution and node-local state that
//!   does not replicate.
//! - **`initialize` first, `server/discover` as a fallback.** Every server
//!   deployed today predates the 2026-07-28 handshake; leading with the new one
//!   fails against real servers on the first message.
//! - **No automatic retry of `tools/call`.** MCP has no idempotency key, so
//!   re-sending a call can charge a card or file a ticket twice. Only session
//!   recovery replays, and only once. `tools/list` is idempotent and may be
//!   retried freely by its caller.

pub mod backoff;
pub mod config;
pub mod connection;
pub mod egress;
pub mod error;
pub mod notification;
pub mod oauth;
pub mod reconcile;
pub mod session;
pub mod session_cache;
pub mod stream;
pub mod subscription;
pub mod transport;

pub use backoff::{Backoff, DEFAULT_BASE_DELAY, DEFAULT_MAX_DELAY};
pub use config::{
    install_config, installed_config, installed_egress_policy, McpClientConfig, DEFAULT_TIMEOUT_MS,
};
pub use connection::{
    validate_slug, AuthKind, McpConnectionDescriptor, RefreshPolicy, StaticAuthScheme, ToolFilter,
    DEFAULT_CALL_TIMEOUT_MS, MAX_CALL_TIMEOUT_MS,
};
pub use egress::EgressPolicy;
pub use error::{RemoteToolError, Result};
pub use notification::{notification_parts, NotificationSink, TOOLS_LIST_CHANGED};
pub use oauth::{
    as_metadata_url, authorize_url, exchange_code, expires_at, fetch_as_metadata,
    fetch_protected_resource_metadata, guard_peer_url as guard_peer_endpoint, parse_challenge,
    protected_resource_metadata_urls, refresh_tokens, register_client, resolve_scopes,
    AuthChallenge, AuthServerMetadata, Pkce, ProtectedResourceMetadata, RegisteredClient,
    TokenResponse,
};
pub use reconcile::{
    plan_proxies, reconcile_plan, schema_hash, slugify, ExistingProxy, ProxyPlan, ReconcileAction,
};
pub use session::{
    concat_text, ensure_object, McpClientSession, RemoteToolDescriptor, ServerHandshake,
};
pub use session_cache::{shared_session_cache, SessionCache};
pub use stream::{
    decode_message, SseEvent, SseReader, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_FRAME_BYTES,
};
pub use subscription::{
    open as open_subscription, StreamEnd, SubscriptionStream, SUBSCRIPTIONS_ACKNOWLEDGED,
    SUBSCRIPTIONS_LISTEN,
};
pub use transport::{ExtraHeaders, StreamableHttpTransport, DEFAULT_MAX_RESPONSE_BYTES};
