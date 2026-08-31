// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Native IMAP protocol binding for server-side functions.
//!
//! This module owns the IMAP protocol in Rust and exposes a small, bounded set
//! of high-level operations to the QuickJS and Starlark runtimes via the
//! `raisin.imap.*` namespace. It is the first worked example of the "add a
//! native capability when `raisin.http.fetch` is not enough" pattern: a real
//! IMAP server is reached over TLS, not JMAP-over-HTTP.
//!
//! Security: callers MUST enforce the function's `NetworkPolicy` (host:port
//! allow-list) BEFORE invoking any operation here. See
//! `api/raisindb/imap.rs`. Connection credentials are never logged.

mod client;
mod parse;
mod transport;

pub use client::{fetch_message, fetch_since, list_mailboxes};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default cap on messages returned by a single `fetch_since` call.
pub const DEFAULT_FETCH_LIMIT: usize = 200;
/// Hard cap so a caller can never request an unbounded fetch.
pub const MAX_FETCH_LIMIT: usize = 1000;
/// Overall per-operation timeout (connect + login + fetch).
pub const OP_TIMEOUT_SECS: u64 = 30;

/// Connection parameters for an IMAP account.
///
/// `password` doubles as an app password or an XOAUTH2 access token; it is
/// redacted from every `Debug` / log rendering (see the manual `Debug` impl).
#[derive(Clone, Deserialize)]
pub struct ImapConn {
    /// IMAP server hostname (e.g. `imap.gmail.com`).
    pub host: String,
    /// IMAP server port (typically 993 for implicit TLS, 143 for plaintext).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Whether to use implicit TLS. `true` (default) connects over TLS on 993;
    /// `false` uses an unencrypted connection (only for trusted/loopback hosts).
    #[serde(default = "default_true")]
    pub tls: bool,
    /// Authentication mechanism. `password` (default) sends `LOGIN`; `xoauth2`
    /// performs a SASL `AUTHENTICATE XOAUTH2` handshake with `password` treated
    /// as the OAuth2 bearer access token.
    #[serde(default)]
    pub auth: ImapAuth,
    /// Login username (usually the full email address).
    pub username: String,
    /// Login secret — app password, or (when `auth = "xoauth2"`) an OAuth2
    /// access token. Never logged.
    pub password: String,
}

/// IMAP authentication mechanism.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImapAuth {
    /// Plain `LOGIN` with username + password/app-password.
    #[default]
    Password,
    /// SASL `AUTHENTICATE XOAUTH2`; `password` carries the bearer token.
    Xoauth2,
}

fn default_port() -> u16 {
    993
}
fn default_true() -> bool {
    true
}

impl ImapConn {
    /// Parse a connection descriptor from a JSON value.
    pub fn from_value(v: &Value) -> Result<Self, ImapError> {
        serde_json::from_value(v.clone())
            .map_err(|e| ImapError::Config(format!("invalid conn: {e}")))
    }

    /// The synthetic URL used for network-policy authorization, e.g.
    /// `imaps://imap.gmail.com:993`. The scheme reflects the TLS mode so an
    /// adapter's allow-list can distinguish plaintext from TLS endpoints.
    pub fn policy_url(&self) -> String {
        let scheme = if self.tls { "imaps" } else { "imap" };
        format!("{scheme}://{}:{}", self.host, self.port)
    }
}

// Redact the password so it can never leak through a `{:?}` / tracing render.
impl std::fmt::Debug for ImapConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapConn")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("tls", &self.tls)
            .field("auth", &self.auth)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Typed IMAP error. `code()` yields the stable machine code the connector
/// adapter maps onto (`auth_expired`, `rate_limited`, ...). The `Display` /
/// `Debug` text never contains credentials.
#[derive(Debug, thiserror::Error)]
pub enum ImapError {
    /// The network policy refused this host:port before any socket was opened.
    #[error("[imap:policy_denied] {0}")]
    PolicyDenied(String),
    /// Authentication failed / credentials rejected -> `auth_expired`.
    #[error("[imap:auth_expired] {0}")]
    Auth(String),
    /// Server signalled throttling / too many connections -> `rate_limited`.
    #[error("[imap:rate_limited] {0}")]
    RateLimited(String),
    /// The operation exceeded the per-op timeout.
    #[error("[imap:timeout] operation timed out")]
    Timeout,
    /// A protocol / IO / TLS failure that is not one of the above.
    #[error("[imap:protocol_error] {0}")]
    Protocol(String),
    /// Malformed connection descriptor or unsupported option.
    #[error("[imap:config] {0}")]
    Config(String),
}

impl ImapError {
    /// Stable machine-readable code for the connector adapter.
    pub fn code(&self) -> &'static str {
        match self {
            ImapError::PolicyDenied(_) => "policy_denied",
            ImapError::Auth(_) => "auth_expired",
            ImapError::RateLimited(_) => "rate_limited",
            ImapError::Timeout => "timeout",
            ImapError::Protocol(_) => "protocol_error",
            ImapError::Config(_) => "config",
        }
    }
}

impl From<ImapError> for raisin_error::Error {
    fn from(e: ImapError) -> Self {
        let msg = e.to_string();
        match e {
            ImapError::PolicyDenied(_) => raisin_error::Error::PermissionDenied(msg),
            ImapError::Auth(_) => raisin_error::Error::Unauthorized(msg),
            _ => raisin_error::Error::Backend(msg),
        }
    }
}

/// Lightweight summary of a message (from an `ENVELOPE`/`FLAGS` fetch).
#[derive(Debug, Clone, Serialize)]
pub struct MessageSummary {
    /// Server UID.
    pub uid: u32,
    /// Envelope-derived header subset (from/to/subject/date/message-id).
    pub headers: serde_json::Map<String, Value>,
    /// Formatted `From` list.
    pub from: String,
    /// Formatted `To` list.
    pub to: String,
    /// Subject line.
    pub subject: String,
    /// Canonical send time: RFC 3339 in UTC (`2026-09-02T07:00:00Z`), fixed
    /// width so it sorts and pages lexically. Empty when the header is absent
    /// or unparsable. The raw header string is preserved in `headers.date`.
    pub date: String,
    /// Short body preview. Empty for `fetch_since` (headers-only); populated by
    /// `fetch_message`.
    pub snippet: String,
    /// IMAP flags (e.g. `\Seen`).
    pub flags: Vec<String>,
    /// RFC822 Message-ID.
    pub message_id: String,
    /// Optional threading hint (References/In-Reply-To root); may be null.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// Result of `fetch_since`.
#[derive(Debug, Clone, Serialize)]
pub struct FetchSinceResult {
    /// Messages with UID strictly greater than the requested cursor.
    pub messages: Vec<MessageSummary>,
    /// Highest UID observed (the new cursor). Never decreases below the input.
    #[serde(rename = "highestUid")]
    pub highest_uid: u32,
    /// Mailbox `UIDVALIDITY`; a change means the adapter must full-resync.
    pub uidvalidity: u32,
}

/// One mailbox as returned by `list_mailboxes`.
#[derive(Debug, Clone, Serialize)]
pub struct MailboxInfo {
    /// Leaf name.
    pub name: String,
    /// Full hierarchical path.
    pub path: String,
    /// IMAP name attributes (e.g. `\HasNoChildren`).
    pub flags: Vec<String>,
}

/// Full message content (from a `BODY[]` fetch, parsed with `mail-parser`).
#[derive(Debug, Clone, Serialize)]
pub struct MessageDetail {
    /// Server UID.
    pub uid: u32,
    /// Parsed header map (well-known headers as strings).
    pub headers: serde_json::Map<String, Value>,
    /// Formatted `From`.
    pub from: String,
    /// Formatted `To`.
    pub to: String,
    /// Subject line.
    pub subject: String,
    /// Canonical send time: RFC 3339 in UTC (`2026-09-02T07:00:00Z`), fixed
    /// width so it sorts and pages lexically. Empty when the header is absent
    /// or unparsable. The raw header string is preserved in `headers.date`.
    pub date: String,
    /// Plain-text body, if present.
    pub text: String,
    /// HTML body, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    /// Short preview derived from the text body.
    pub snippet: String,
    /// IMAP flags.
    pub flags: Vec<String>,
    /// RFC822 Message-ID.
    pub message_id: String,
}
