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

use super::{brevo, resend};
use super::{
    Credential, EmailConfig, EmailError, EmailMessage, Result, CONNECT_TIMEOUT_SECS,
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
    /// Direct SMTP submission. Reserved: the variant exists so the config
    /// surface is stable, but v1 has no implementation and sending through it
    /// returns [`EmailError::Unsupported`]. Opening a raw TCP session from the
    /// function sandbox is a separate security decision.
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

    /// Send one message.
    ///
    /// `credential` is the secret resolved from `cfg.credential_ref`; it is
    /// borrowed for the duration of the call and immediately wrapped in a
    /// [`Credential`] so no intermediate value can print it.
    pub async fn send(
        &self,
        cfg: &EmailConfig,
        credential: &str,
        message: &EmailMessage,
    ) -> Result<SendReceipt> {
        if !cfg.enabled {
            return Err(EmailError::Config(
                "email is disabled for this tenant".to_string(),
            ));
        }
        if cfg.provider != *self {
            return Err(EmailError::Config(format!(
                "config selects provider {} but {} was invoked",
                cfg.provider.as_str(),
                self.as_str()
            )));
        }
        // Validate before opening a socket: a malformed message must fail as
        // `invalid_message` locally rather than as whatever the provider
        // happens to answer.
        message.validate()?;
        if credential.trim().is_empty() {
            return Err(EmailError::Config(format!(
                "no credential resolved from {}",
                cfg.credential_ref
            )));
        }
        let credential = Credential::new(credential);

        match self {
            EmailProvider::Resend => resend::send(cfg, &credential, message).await,
            EmailProvider::Brevo => brevo::send(cfg, &credential, message).await,
            EmailProvider::Smtp => Err(EmailError::Unsupported(
                "smtp sending is not implemented; configure resend or brevo".to_string(),
            )),
        }
    }
}

/// Proof that a provider accepted a message for delivery. Acceptance is not
/// delivery — the id is what a later bounce/webhook correlates against.
#[derive(Clone, Debug, Serialize)]
pub struct SendReceipt {
    /// The provider's message identifier.
    pub message_id: String,
    /// Which provider issued it.
    pub provider: EmailProvider,
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

    #[tokio::test]
    async fn smtp_is_reserved_but_unimplemented() {
        let cfg = EmailConfig {
            provider: EmailProvider::Smtp,
            from_address: "noreply@example.com".to_string(),
            from_name: None,
            reply_to: None,
            base_url: "https://app.example.com".to_string(),
            credential_ref: "secret://email/api_key".to_string(),
            enabled: true,
        };
        let msg = EmailMessage::new("user@example.com", "Sign in", "link");
        let err = EmailProvider::Smtp
            .send(&cfg, "irrelevant", &msg)
            .await
            .expect_err("smtp must not send");
        assert_eq!(err.code(), "unsupported");
    }

    #[tokio::test]
    async fn a_malformed_message_never_reaches_the_network() {
        let cfg = EmailConfig {
            provider: EmailProvider::Resend,
            from_address: "noreply@example.com".to_string(),
            from_name: None,
            reply_to: None,
            base_url: "https://app.example.com".to_string(),
            credential_ref: "secret://email/api_key".to_string(),
            enabled: true,
        };
        let msg = EmailMessage::new("user@example.com", "Sign in\nBcc: x@y.z", "link");
        let err = EmailProvider::Resend
            .send(&cfg, "re_key", &msg)
            .await
            .expect_err("header splitting must be refused");
        assert_eq!(err.code(), "invalid_message");
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
