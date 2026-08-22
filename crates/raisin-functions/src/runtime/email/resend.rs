// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Resend provider: `POST https://api.resend.com/emails` with bearer auth.
//!
//! <https://resend.com/docs/api-reference/emails/send-email>

use serde::{Deserialize, Serialize};

use super::provider::{build_client, error_from_response, map_transport_err, SendReceipt};
use super::{Credential, EmailError, EmailMessage, EmailProvider, EmailSender, Result};

/// Resend's API root. A sender may override it (regional endpoint, or a local
/// stub in tests) through `api_base`.
const API_BASE: &str = "https://api.resend.com";
/// Path of the send endpoint, appended to the API base.
const SEND_PATH: &str = "/emails";

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
    sender: &EmailSender,
    credential: &Credential,
    message: &EmailMessage,
) -> Result<SendReceipt> {
    let body = SendRequest {
        from: sender.from_header(),
        to: &message.to,
        subject: &message.subject,
        text: &message.text,
        html: message.html.as_deref(),
        reply_to: sender.reply_to.as_deref(),
    };

    let endpoint = super::provider::endpoint(sender, API_BASE, SEND_PATH);
    let resp = build_client()?
        .post(&endpoint)
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
        sender: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_takes_the_sender_from_config_only() {
        let sender = EmailSender {
            name: "transactional".to_string(),
            provider: EmailProvider::Resend,
            from_address: "noreply@example.com".to_string(),
            from_name: Some("Example".to_string()),
            reply_to: Some("support@example.com".to_string()),
            credential_ref: "secret://email/api_key".to_string(),
            api_base: None,
            smtp: None,
        };
        let msg = EmailMessage::new("user@example.com", "Sign in", "link");
        let body = SendRequest {
            from: sender.from_header(),
            to: &msg.to,
            subject: &msg.subject,
            text: &msg.text,
            html: msg.html.as_deref(),
            reply_to: sender.reply_to.as_deref(),
        };
        let v = serde_json::to_value(&body).expect("serializes");
        assert_eq!(v["from"], "Example <noreply@example.com>");
        assert_eq!(v["to"][0], "user@example.com");
        assert_eq!(v["reply_to"], "support@example.com");
        // An absent HTML alternative is omitted, not sent as null.
        assert!(v.get("html").is_none());
    }

    /// A sender's `api_base` must actually redirect the request — this is what
    /// a regional endpoint and every stubbed test depend on.
    #[test]
    fn api_base_overrides_the_endpoint() {
        let mut sender = EmailSender {
            name: "r".to_string(),
            provider: EmailProvider::Resend,
            from_address: "noreply@example.com".to_string(),
            from_name: None,
            reply_to: None,
            credential_ref: "secret://email/api_key".to_string(),
            api_base: None,
            smtp: None,
        };
        assert_eq!(
            super::super::provider::endpoint(&sender, API_BASE, SEND_PATH),
            "https://api.resend.com/emails"
        );
        // Trailing slash on the override must not double up.
        sender.api_base = Some("https://stub.test/".to_string());
        assert_eq!(
            super::super::provider::endpoint(&sender, API_BASE, SEND_PATH),
            "https://stub.test/emails"
        );
    }
}
