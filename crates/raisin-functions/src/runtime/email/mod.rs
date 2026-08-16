// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Native transactional-email binding for server-side functions.
//!
//! This module owns outbound transactional email in Rust and is the sibling of
//! `runtime::imap`: IMAP reads a mailbox, this sends one message through a
//! tenant-configured HTTP email provider. Both follow the same shape — typed
//! config, typed errors, credentials that can never reach a log line.
//!
//! Providers are HTTP APIs only (Resend, Brevo). SMTP is deliberately absent in
//! v1: raw TCP is not something the function sandbox should be opening on a
//! caller's behalf. [`EmailProvider::Smtp`] exists as a variant so the config
//! surface does not have to change when it lands, and every send through it
//! fails with [`EmailError::Unsupported`].
//!
//! Security:
//! * The credential lives in the secret store and is passed to [`send`] as a
//!   short-lived borrow. Every type that can hold it has a hand-written `Debug`
//!   that redacts it, and no error string ever embeds it — a provider's 401 is
//!   reported by status, never by echoing what was sent.
//! * `base_url` is an explicit property of the `/config/email` node. It is NOT
//!   derived from a request `Host`/`Origin` header: a link built from an
//!   attacker-controlled header is a poisoned link that still carries a valid
//!   token.
//!
//! [`send`]: EmailProvider::send

mod brevo;
mod provider;
mod resend;

pub use provider::{EmailProvider, SendReceipt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Connect timeout for a provider request.
pub const CONNECT_TIMEOUT_SECS: u64 = 10;
/// Overall per-send timeout (connect + request + response).
pub const SEND_TIMEOUT_SECS: u64 = 30;
/// Hard cap on recipients in a single send. Transactional mail is one-to-one or
/// one-to-a-handful; anything larger is a bulk send that belongs to a campaign
/// tool, and an unbounded list is an amplification primitive.
pub const MAX_RECIPIENTS: usize = 20;

/// The parsed form of the `raisin:EmailConfig` node at `/config/email` in the
/// `raisin:system` workspace. Per tenant — there is no global email config.
///
/// The credential itself is NOT part of this struct: the node only carries a
/// reference (`secret://email/api_key`), and the resolved secret is passed to
/// [`EmailProvider::send`] separately so it never sits in a long-lived,
/// cloneable, serializable value.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EmailConfig {
    /// Which provider API to talk to.
    pub provider: EmailProvider,
    /// Envelope sender address. Must be a domain verified with the provider.
    pub from_address: String,
    /// Optional display name rendered alongside `from_address`.
    #[serde(default)]
    pub from_name: Option<String>,
    /// Optional `Reply-To` address.
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Absolute base URL of this tenant's front end (e.g.
    /// `https://app.example.com`), used to build magic links. Explicit on
    /// purpose: see the module-level note on link poisoning.
    pub base_url: String,
    /// Where the API credential lives in the secret store. Defaults to
    /// `secret://email/api_key`; resolving it is the caller's job.
    #[serde(default = "default_credential_ref")]
    pub credential_ref: String,
    /// Master switch. A tenant that has provisioned the node but not finished
    /// verifying its sending domain can leave this `false`.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_credential_ref() -> String {
    "secret://email/api_key".to_string()
}
fn default_true() -> bool {
    true
}

impl EmailConfig {
    /// Parse the config from the node's property map.
    pub fn from_value(v: &Value) -> Result<Self> {
        serde_json::from_value(v.clone())
            .map_err(|e| EmailError::Config(format!("invalid email config: {e}")))
    }

    /// `From` header value, `"Name <addr>"` when a display name is configured
    /// and the bare address otherwise.
    pub fn from_header(&self) -> String {
        match &self.from_name {
            Some(name) if !name.is_empty() => format!("{name} <{}>", self.from_address),
            _ => self.from_address.clone(),
        }
    }
}

/// A resolved provider credential.
///
/// Exists so that the secret is carried by a type that cannot be printed: the
/// `Debug` impl is hand-written (the [`super::imap::ImapConn`] precedent) and
/// there is no `Display`, no `Serialize`, and no public field. The only way out
/// is [`Credential::expose`], which is deliberately noisy at the call site.
#[derive(Clone)]
pub struct Credential(String);

impl Credential {
    /// Wrap a resolved secret.
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// The raw secret, for placing into an outbound `Authorization` / `api-key`
    /// header. Never log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

// Redact so the credential can never leak through a `{:?}` / tracing render.
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

impl From<&str> for Credential {
    fn from(s: &str) -> Self {
        Credential::new(s)
    }
}

/// One outbound message. `from` / `from_name` / `reply_to` are intentionally
/// absent: they come from [`EmailConfig`], so a function author cannot forge a
/// sender the tenant has not verified.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EmailMessage {
    /// One or more recipient addresses. At least one, at most
    /// [`MAX_RECIPIENTS`].
    pub to: Vec<String>,
    /// Subject line.
    pub subject: String,
    /// Plain-text body. Always required — an HTML-only message is a spam signal
    /// and unreadable in a text client.
    pub text: String,
    /// Optional HTML body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
}

impl EmailMessage {
    /// Build a plain-text message to a single recipient.
    pub fn new(to: impl Into<String>, subject: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            to: vec![to.into()],
            subject: subject.into(),
            text: text.into(),
            html: None,
        }
    }

    /// Attach an HTML alternative body.
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }

    /// Reject a message the provider would either bounce or, worse, accept with
    /// smuggled headers.
    ///
    /// Both providers take JSON rather than a raw RFC 5322 blob, so a CR/LF in
    /// a subject or address is not a header-injection vector *for them* — but it
    /// is for any future SMTP provider, and a header-splitting attempt is never
    /// legitimate traffic. Rejecting here keeps the check in one place.
    pub(crate) fn validate(&self) -> Result<()> {
        if self.to.is_empty() {
            return Err(EmailError::InvalidMessage("no recipients".to_string()));
        }
        if self.to.len() > MAX_RECIPIENTS {
            return Err(EmailError::InvalidMessage(format!(
                "too many recipients: {} (max {MAX_RECIPIENTS})",
                self.to.len()
            )));
        }
        for addr in &self.to {
            if addr.trim().is_empty() || !addr.contains('@') {
                return Err(EmailError::InvalidMessage(
                    "recipient is not an email address".to_string(),
                ));
            }
            if has_control_chars(addr) {
                return Err(EmailError::InvalidMessage(
                    "recipient contains control characters".to_string(),
                ));
            }
        }
        if self.subject.is_empty() {
            return Err(EmailError::InvalidMessage("empty subject".to_string()));
        }
        if has_control_chars(&self.subject) {
            return Err(EmailError::InvalidMessage(
                "subject contains control characters".to_string(),
            ));
        }
        if self.text.is_empty() {
            return Err(EmailError::InvalidMessage("empty text body".to_string()));
        }
        Ok(())
    }
}

/// True when `s` contains a CR, LF or NUL — the header-splitting trio.
fn has_control_chars(s: &str) -> bool {
    s.chars().any(|c| c == '\r' || c == '\n' || c == '\0')
}

/// Typed email error. `code()` yields the stable machine code an adapter or an
/// HTTP layer maps onto. The `Display` / `Debug` text never contains the
/// credential nor the request body.
#[derive(Debug, thiserror::Error)]
pub enum EmailError {
    /// The provider rejected the credential (HTTP 401/403). Distinct from
    /// [`EmailError::InvalidMessage`] on purpose: one is an operator problem
    /// (rotate the secret), the other is a caller problem (fix the message).
    #[error("[email:auth_failed] {0}")]
    Auth(String),
    /// The provider rejected the message itself (HTTP 400/422) — unverified
    /// sender domain, malformed address, missing field — or local validation
    /// refused it before a socket was opened.
    #[error("[email:invalid_message] {0}")]
    InvalidMessage(String),
    /// The provider is throttling us (HTTP 429).
    #[error("[email:rate_limited] {0}")]
    RateLimited(String),
    /// The provider answered with a status this module does not special-case,
    /// or a body it could not parse.
    #[error("[email:provider_error] {0}")]
    Provider(String),
    /// DNS / TCP / TLS failure reaching the provider.
    #[error("[email:transport] {0}")]
    Transport(String),
    /// The send exceeded [`SEND_TIMEOUT_SECS`].
    #[error("[email:timeout] send timed out")]
    Timeout,
    /// Malformed `/config/email`, or email disabled for this tenant.
    #[error("[email:config] {0}")]
    Config(String),
    /// A provider variant that exists in the enum but has no implementation
    /// yet (today: SMTP).
    #[error("[email:unsupported] {0}")]
    Unsupported(String),
}

impl EmailError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            EmailError::Auth(_) => "auth_failed",
            EmailError::InvalidMessage(_) => "invalid_message",
            EmailError::RateLimited(_) => "rate_limited",
            EmailError::Provider(_) => "provider_error",
            EmailError::Transport(_) => "transport",
            EmailError::Timeout => "timeout",
            EmailError::Config(_) => "config",
            EmailError::Unsupported(_) => "unsupported",
        }
    }
}

impl From<EmailError> for raisin_error::Error {
    fn from(e: EmailError) -> Self {
        let msg = e.to_string();
        match e {
            // An auth failure here is *our* credential failing against the
            // provider, not the caller's — it must not be rendered to an end
            // user as "you are unauthorized".
            EmailError::Auth(_) => raisin_error::Error::Backend(msg),
            EmailError::InvalidMessage(_) => raisin_error::Error::Validation(msg),
            EmailError::Config(_) => raisin_error::Error::InvalidState(msg),
            _ => raisin_error::Error::Backend(msg),
        }
    }
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, EmailError>;

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "re_supersecret_ABC123";

    fn cfg() -> EmailConfig {
        EmailConfig {
            provider: EmailProvider::Resend,
            from_address: "noreply@example.com".to_string(),
            from_name: Some("Example".to_string()),
            reply_to: None,
            base_url: "https://app.example.com".to_string(),
            credential_ref: default_credential_ref(),
            enabled: true,
        }
    }

    #[test]
    fn credential_is_redacted_in_debug() {
        let c = Credential::new(SECRET);
        let rendered = format!("{c:?}");
        assert!(
            !rendered.contains(SECRET),
            "credential leaked through Debug: {rendered}"
        );
        assert_eq!(rendered, "Credential(<redacted>)");
        // And still readable where it is actually needed.
        assert_eq!(c.expose(), SECRET);
    }

    #[test]
    fn nothing_reachable_from_a_send_renders_the_credential() {
        // Every value the send path constructs and might end up in a tracing
        // field or an error log, checked in one place.
        let c = Credential::new(SECRET);
        let msg = EmailMessage::new("user@example.com", "Sign in", "link").with_html("<b>link</b>");
        let receipt = SendReceipt {
            message_id: "abc-123".to_string(),
            provider: EmailProvider::Resend,
        };
        let err = EmailError::Auth("provider returned 401".to_string());
        for rendered in [
            format!("{c:?}"),
            format!("{:?}", cfg()),
            format!("{msg:?}"),
            format!("{receipt:?}"),
            format!("{err:?}"),
            err.to_string(),
        ] {
            assert!(!rendered.contains(SECRET), "credential leaked: {rendered}");
        }
    }

    #[test]
    fn from_header_uses_display_name_when_present() {
        assert_eq!(cfg().from_header(), "Example <noreply@example.com>");
        let mut bare = cfg();
        bare.from_name = None;
        assert_eq!(bare.from_header(), "noreply@example.com");
    }

    #[test]
    fn validate_rejects_header_splitting_and_empty_fields() {
        let mut m = EmailMessage::new("user@example.com", "Sign in", "link");
        assert!(m.validate().is_ok());

        m.subject = "Sign in\r\nBcc: attacker@evil.test".to_string();
        assert!(matches!(m.validate(), Err(EmailError::InvalidMessage(_))));

        let m = EmailMessage::new("user@example.com", "", "link");
        assert!(matches!(m.validate(), Err(EmailError::InvalidMessage(_))));

        let m = EmailMessage::new("not-an-address", "Sign in", "link");
        assert!(matches!(m.validate(), Err(EmailError::InvalidMessage(_))));

        let m = EmailMessage {
            to: vec![],
            subject: "Sign in".to_string(),
            text: "link".to_string(),
            html: None,
        };
        assert!(matches!(m.validate(), Err(EmailError::InvalidMessage(_))));

        let m = EmailMessage {
            to: vec!["user@example.com".to_string(); MAX_RECIPIENTS + 1],
            subject: "Sign in".to_string(),
            text: "link".to_string(),
            html: None,
        };
        assert!(matches!(m.validate(), Err(EmailError::InvalidMessage(_))));
    }

    #[test]
    fn config_parses_from_node_properties() {
        let v = serde_json::json!({
            "provider": "brevo",
            "from_address": "noreply@example.com",
            "base_url": "https://app.example.com",
        });
        let parsed = EmailConfig::from_value(&v).expect("valid config");
        assert_eq!(parsed.provider, EmailProvider::Brevo);
        assert_eq!(parsed.credential_ref, "secret://email/api_key");
        assert!(parsed.enabled);
        assert!(parsed.from_name.is_none());

        // An unknown provider is a config error, not a panic.
        let bad = serde_json::json!({
            "provider": "carrier_pigeon",
            "from_address": "noreply@example.com",
            "base_url": "https://app.example.com",
        });
        assert!(matches!(
            EmailConfig::from_value(&bad),
            Err(EmailError::Config(_))
        ));
    }
}
