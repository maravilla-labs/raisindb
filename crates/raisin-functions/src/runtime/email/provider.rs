// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Provider dispatch and the HTTP plumbing the provider modules share.
//!
//! Each provider module owns exactly its request/response shapes; everything
//! that is the same for all of them — the client, the timeouts, the mapping of
//! an HTTP status onto a typed [`EmailError`] — lives here so a new provider
//! cannot quietly get a different error taxonomy.

use std::time::Duration;

use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};

use super::{brevo, resend, smtp};
use super::{
    Credential, EmailError, EmailMessage, EmailSender, Result, CONNECT_TIMEOUT_SECS,
    SEND_TIMEOUT_SECS,
};

/// Which transactional-email API a tenant sends through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EmailProvider {
    /// <https://resend.com> — `POST /emails`, bearer auth.
    Resend,
    /// <https://brevo.com> — `POST /v3/smtp/email`, `api-key` header.
    Brevo,
    /// Direct SMTP submission — any provider with a relay, or your own mail
    /// server. The session is opened by this crate, never by the sandbox: a
    /// function names a provider, and the host, port and password come from
    /// operator configuration and the secret store.
    Smtp,
}

impl EmailProvider {
    /// Human-readable provider name, used in receipts and error text.
    pub fn as_str(&self) -> &'static str {
        match self {
            EmailProvider::Resend => "resend",
            EmailProvider::Brevo => "brevo",
            EmailProvider::Smtp => "smtp",
        }
    }

    /// Whether this provider can carry an inline part addressable as
    /// `cid:{content_id}` from the HTML body.
    ///
    /// Brevo's `POST /v3/smtp/email` attachment objects are `{ name, content }`
    /// with no Content-ID field, so an inline part sent through it arrives as
    /// an ordinary attachment while the `cid:` reference in the body resolves
    /// to nothing. That is refused rather than degraded — a broken image in
    /// the recipient's inbox is invisible to every test we could write.
    pub fn supports_inline(&self) -> bool {
        match self {
            EmailProvider::Resend | EmailProvider::Smtp => true,
            EmailProvider::Brevo => false,
        }
    }

    /// Send one message through `sender`.
    ///
    /// `credential` is the secret resolved from `sender.credential_ref`; it is
    /// borrowed for the duration of the call and immediately wrapped in a
    /// [`Credential`] so no intermediate value can print it.
    ///
    /// The provider is taken from the SENDER, not from `self`: resolution
    /// ([`EmailConfig::resolve`]) has already decided which configured account
    /// this send belongs to, and re-deciding here would be the second
    /// implementation this module exists to avoid.
    ///
    /// [`EmailConfig::resolve`]: super::EmailConfig::resolve
    pub async fn send(
        sender: &EmailSender,
        credential: &str,
        message: &EmailMessage,
    ) -> Result<SendReceipt> {
        // Validate before opening a socket: a malformed message must fail as
        // `invalid_message` locally rather than as whatever the provider
        // happens to answer.
        message.validate()?;
        // Bounds and provider capabilities. Separate from `validate` only
        // because they need the resolved sender; both run before a socket.
        message.validate_attachments(sender.provider, &sender.attachment_limits)?;
        if credential.trim().is_empty() {
            return Err(EmailError::Config(format!(
                "no credential resolved from {} for email provider `{}`",
                sender.credential_ref, sender.name
            )));
        }
        let credential = Credential::new(credential);

        let mut receipt = match sender.provider {
            EmailProvider::Resend => resend::send(sender, &credential, message).await,
            EmailProvider::Brevo => brevo::send(sender, &credential, message).await,
            EmailProvider::Smtp => smtp::send(sender, &credential, message).await,
        }?;
        // Stamped here rather than in each provider module: WHICH configured
        // account sent is this layer's knowledge, and a provider module that
        // forgot to set it would produce a receipt that silently lied.
        receipt.sender = sender.name.clone();
        Ok(receipt)
    }
}

/// Proof that a provider accepted a message for delivery. Acceptance is not
/// delivery — the id is what a later bounce/webhook correlates against.
#[derive(Clone, Debug, Serialize)]
pub struct SendReceipt {
    /// The provider's message identifier.
    pub message_id: String,
    /// Which provider API issued it.
    pub provider: EmailProvider,
    /// The configured sender name it went through — the answer to "which of my
    /// accounts sent this", which `provider` alone cannot give once a tenant
    /// has two entries on the same API.
    #[serde(default)]
    pub sender: String,
}

/// The URL an HTTP provider posts to: the sender's `api_base` when it sets
/// one, the provider's own root otherwise.
///
/// Shared rather than repeated per provider so the trailing-slash handling
/// cannot differ between them — `https://stub.test/` and `https://stub.test`
/// must produce the same URL, and a doubled slash is a 404 that reads like an
/// outage.
pub(super) fn endpoint(sender: &EmailSender, default_base: &str, path: &str) -> String {
    let base = sender
        .api_base
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .unwrap_or(default_base);
    format!("{}{}", base.trim_end_matches('/'), path)
}

/// HTTP client for a provider request. Built per send rather than cached: a
/// send is rare (one magic link), so the handshake cost is irrelevant next to
/// the simplicity of not holding a pool keyed by tenant.
pub(super) fn build_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(SEND_TIMEOUT_SECS))
        .build()
        .map_err(|e| EmailError::Transport(format!("could not build http client: {e}")))
}

/// Map a `reqwest` transport failure onto a typed error. The `reqwest` error
/// renders the URL and the IO cause, never the request headers, so the
/// credential cannot ride along.
pub(super) fn map_transport_err(e: reqwest::Error) -> EmailError {
    if e.is_timeout() {
        EmailError::Timeout
    } else {
        EmailError::Transport(e.to_string())
    }
}

/// Turn a non-2xx provider response into the right typed error.
///
/// The distinction that matters operationally: 401/403 means *our* API key is
/// wrong or revoked (page the operator, rotate the secret), while 400/422 means
/// the message we built is wrong (fix the caller). Collapsing both into one
/// "provider said no" error is what makes email outages hard to triage.
pub(super) async fn error_from_response(provider: EmailProvider, resp: Response) -> EmailError {
    let status = resp.status();
    // Body text is the provider's own diagnostic; it echoes the message fields
    // it disliked, never the credential. Truncated so a verbose provider cannot
    // blow up a log line.
    let body = resp.text().await.unwrap_or_default();
    let detail = format!(
        "{} returned {}: {}",
        provider.as_str(),
        status,
        excerpt(&body)
    );

    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => EmailError::Auth(detail),
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            EmailError::InvalidMessage(detail)
        }
        StatusCode::TOO_MANY_REQUESTS => EmailError::RateLimited(detail),
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => EmailError::Timeout,
        _ => EmailError::Provider(detail),
    }
}

/// First 300 characters of a provider body, with whitespace collapsed.
fn excerpt(body: &str) -> String {
    let flat: String = body
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = flat.trim();
    if trimmed.chars().count() <= 300 {
        trimmed.to_string()
    } else {
        let head: String = trimmed.chars().take(300).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_round_trips_through_serde() {
        for (p, s) in [
            (EmailProvider::Resend, "\"resend\""),
            (EmailProvider::Brevo, "\"brevo\""),
            (EmailProvider::Smtp, "\"smtp\""),
        ] {
            assert_eq!(serde_json::to_string(&p).unwrap(), s);
            assert_eq!(serde_json::from_str::<EmailProvider>(s).unwrap(), p);
        }
    }

    fn sender(provider: EmailProvider) -> EmailSender {
        EmailSender {
            provider,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_malformed_message_never_reaches_the_network() {
        let msg = EmailMessage::new("user@example.com", "Sign in\nBcc: x@y.z", "link");
        let err = EmailProvider::send(&sender(EmailProvider::Resend), "re_key", &msg)
            .await
            .expect_err("header splitting must be refused");
        assert_eq!(err.code(), "invalid_message");
    }

    /// An empty credential must fail as configuration, before a socket — the
    /// alternative is an unauthenticated request and a provider 401 that reads
    /// like a revoked key rather than a missing secret.
    #[tokio::test]
    async fn an_unresolved_credential_fails_as_config() {
        let msg = EmailMessage::new("user@example.com", "Sign in", "link");
        let err = EmailProvider::send(&sender(EmailProvider::Resend), "   ", &msg)
            .await
            .expect_err("an empty credential must not be sent with");
        assert_eq!(err.code(), "config");
    }

    #[test]
    fn excerpt_truncates_and_flattens() {
        assert_eq!(excerpt("  a\nb  "), "a b");
        let long = "x".repeat(400);
        let out = excerpt(&long);
        assert_eq!(out.chars().count(), 301);
        assert!(out.ends_with('…'));
    }
}
