// SPDX-License-Identifier: BSL-1.1

//! Anonymous visitor chat over the WebSocket.
//!
//! A public website runs an agent chat straight from the browser: no site
//! server endpoint, no service identity. The pieces:
//!
//! * [`session`] — the session secret the browser keeps, and the home key it
//!   names (`/visitors/<key>` in `raisin:access_control`).
//! * [`config`] — the agent's `anonymous` settings. An agent without the flag
//!   refuses visitors.
//! * [`limits`] — per-session and per-IP rate limits, and the turn lease.
//! * [`daily`] — the limits that span a day and survive a restart: messages
//!   and new sessions per IP, and the agent's daily token budget.
//! * `crate::handlers::visitor_chat` — the two requests (`visitor_chat_start`,
//!   `visitor_chat_send`) and the event forwarder that streams a conversation's
//!   events to the one connection that owns it.
//!
//! Row-level security does the rest (`raisin_models::auth::visitor`): the
//! bound session may read its own home, nobody else may read it, and nobody
//! but the server writes it.

pub mod config;
pub mod daily;
pub mod limits;
pub mod session;

pub use config::AnonymousChatConfig;
pub use limits::VisitorLimits;

use std::collections::HashSet;

/// What a connection is bound to once it has started a visitor chat.
#[derive(Debug, Clone)]
pub struct VisitorBinding {
    /// The session key (`/visitors/<key>`).
    pub key: String,
    /// The session home path.
    pub home: String,
    /// The agent this session talks to (`/agents/<name>`).
    pub agent_path: String,
    /// Conversations this connection may send to and receive from.
    pub conversations: HashSet<String>,
    /// Conversations whose events are already being forwarded.
    pub forwarding: HashSet<String>,
}

/// The client IP of an upgrade request, best effort: the LAST
/// `X-Forwarded-For` entry (the one the nearest proxy appended), else
/// `X-Real-IP`. Used only to key rate limits.
pub fn client_ip(headers: &axum::http::HeaderMap) -> Option<String> {
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    forwarded
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|v| !v.is_empty())
        })
        .map(|v| v.chars().take(64).collect())
}

/// The `Origin` of an upgrade request.
pub fn origin(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.chars().take(256).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn the_nearest_proxys_entry_keys_the_rate_limit() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_static("1.1.1.1, 10.0.0.9"),
        );
        assert_eq!(client_ip(&h).as_deref(), Some("10.0.0.9"));
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", HeaderValue::from_static("2.2.2.2"));
        assert_eq!(client_ip(&h).as_deref(), Some("2.2.2.2"));
        assert_eq!(client_ip(&HeaderMap::new()), None);
    }
}
