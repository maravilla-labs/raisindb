// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Resend provider: `POST https://api.resend.com/emails` with bearer auth.
//!
//! <https://resend.com/docs/api-reference/emails/send-email>

use serde::{Deserialize, Serialize};

use super::provider::{build_client, error_from_response, map_transport_err, SendReceipt};
use super::{Credential, EmailConfig, EmailError, EmailMessage, EmailProvider, Result};

/// Resend's send endpoint.
const ENDPOINT: &str = "https://api.resend.com/emails";

/// Request body. Field names are Resend's wire names; `snake_case` here already
/// matches, so no rename attribute is needed.
#[derive(Serialize)]
struct SendRequest<'a> {
    /// `"Name <addr>"` or a bare address — from config, never from the caller.
    from: String,
    to: &'a [String],
    subject: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<&'a str>,
}

/// Success body: `{"id": "..."}`.
#[derive(Deserialize)]
struct SendResponse {
    id: String,
}

/// Send `message` through Resend.
pub(super) async fn send(
    cfg: &EmailConfig,
    credential: &Credential,
    message: &EmailMessage,
) -> Result<SendReceipt> {
    let body = SendRequest {
        from: cfg.from_header(),
        to: &message.to,
        subject: &message.subject,
        text: &message.text,
        html: message.html.as_deref(),
        reply_to: cfg.reply_to.as_deref(),
    };

    let resp = build_client()?
        .post(ENDPOINT)
        .bearer_auth(credential.expose())
        .json(&body)
        .send()
        .await
        .map_err(map_transport_err)?;

    if !resp.status().is_success() {
        return Err(error_from_response(EmailProvider::Resend, resp).await);
    }

    let parsed: SendResponse = resp.json().await.map_err(|e| {
        EmailError::Provider(format!(
            "resend accepted the send but its body did not parse: {e}"
        ))
    })?;

    Ok(SendReceipt {
        message_id: parsed.id,
        provider: EmailProvider::Resend,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_takes_the_sender_from_config_only() {
        let cfg = EmailConfig {
            provider: EmailProvider::Resend,
            from_address: "noreply@example.com".to_string(),
            from_name: Some("Example".to_string()),
            reply_to: Some("support@example.com".to_string()),
            base_url: "https://app.example.com".to_string(),
            credential_ref: "secret://email/api_key".to_string(),
            enabled: true,
        };
        let msg = EmailMessage::new("user@example.com", "Sign in", "link");
        let body = SendRequest {
            from: cfg.from_header(),
            to: &msg.to,
            subject: &msg.subject,
            text: &msg.text,
            html: msg.html.as_deref(),
            reply_to: cfg.reply_to.as_deref(),
        };
        let v = serde_json::to_value(&body).expect("serializes");
        assert_eq!(v["from"], "Example <noreply@example.com>");
        assert_eq!(v["to"][0], "user@example.com");
        assert_eq!(v["reply_to"], "support@example.com");
        // An absent HTML alternative is omitted, not sent as null.
        assert!(v.get("html").is_none());
    }
}
