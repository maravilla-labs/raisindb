// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Resend provider: `POST https://api.resend.com/emails` with bearer auth.
//!
//! <https://resend.com/docs/api-reference/emails/send-email>

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use super::provider::{build_client, error_from_response, map_transport_err, SendReceipt};
use super::{
    Credential, EmailError, EmailMessage, EmailProvider, EmailSender, ResolvedAttachment, Result,
};

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
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    cc: &'a [String],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    bcc: &'a [String],
    subject: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<&'a str>,
    /// Omitted entirely when empty — Resend rejects a null here.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<Attachment<'a>>,
}

/// One attachment in Resend's `attachments` array.
///
/// Resend also accepts `path` (a URL it fetches itself) instead of `content`.
/// That is deliberately not exposed: the fetch would happen from Resend's
/// network rather than ours, escaping the egress policy every other outbound
/// call goes through, and we would never see the bytes we promise to bound.
#[derive(Serialize)]
struct Attachment<'a> {
    filename: &'a str,
    /// Base64, no `data:` prefix.
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<&'a str>,
    /// Present only for inline parts. Bare — Resend matches it against
    /// `src="cid:..."` verbatim, so angle brackets would break the reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    content_id: Option<&'a str>,
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
        cc: &message.cc,
        bcc: &message.bcc,
        subject: &message.subject,
        text: &message.text,
        html: message.html.as_deref(),
        reply_to: sender.reply_to.as_deref(),
        attachments: message
            .attachments
            .iter()
            .map(|a| Attachment {
                filename: &a.filename,
                content: BASE64.encode(&a.bytes),
                content_type: Some(a.content_type.as_str()),
                content_id: a.content_id.as_deref(),
            })
            .collect(),
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
            from_name: Some("Example".to_string()),
            reply_to: Some("support@example.com".to_string()),
            ..Default::default()
        };
        let msg = EmailMessage::new("user@example.com", "Sign in", "link");
        let body = SendRequest {
            from: sender.from_header(),
            to: &msg.to,
            cc: &msg.cc,
            bcc: &msg.bcc,
            subject: &msg.subject,
            text: &msg.text,
            html: msg.html.as_deref(),
            reply_to: sender.reply_to.as_deref(),
            attachments: Vec::new(),
        };
        let v = serde_json::to_value(&body).expect("serializes");
        assert_eq!(v["from"], "Example <noreply@example.com>");
        assert_eq!(v["to"][0], "user@example.com");
        assert_eq!(v["reply_to"], "support@example.com");
        // An absent HTML alternative is omitted, not sent as null.
        assert!(v.get("html").is_none());
    }

    /// Resend's attachment field names, pinned. A wrong name here is accepted
    /// by the API and only shows up as a missing attachment in the inbox.
    #[test]
    fn attachments_and_copies_use_resends_wire_names() {
        let sender = EmailSender::default();
        let mut msg = EmailMessage::new("user@example.com", "Subject", "body")
            .with_html("<img src=\"cid:logo\">")
            .with_attachments(vec![
                ResolvedAttachment {
                    filename: "ticket.pdf".to_string(),
                    content_type: "application/pdf".to_string(),
                    bytes: b"hello".to_vec(),
                    content_id: None,
                },
                ResolvedAttachment {
                    filename: "logo.png".to_string(),
                    content_type: "image/png".to_string(),
                    bytes: b"png".to_vec(),
                    content_id: Some("logo".to_string()),
                },
            ]);
        msg.cc = vec!["cc@example.com".to_string()];
        msg.bcc = vec!["bcc@example.com".to_string()];

        let body = SendRequest {
            from: sender.from_header(),
            to: &msg.to,
            cc: &msg.cc,
            bcc: &msg.bcc,
            subject: &msg.subject,
            text: &msg.text,
            html: msg.html.as_deref(),
            reply_to: sender.reply_to.as_deref(),
            attachments: msg
                .attachments
                .iter()
                .map(|a| Attachment {
                    filename: &a.filename,
                    content: BASE64.encode(&a.bytes),
                    content_type: Some(a.content_type.as_str()),
                    content_id: a.content_id.as_deref(),
                })
                .collect(),
        };
        let v = serde_json::to_value(&body).expect("serializes");

        assert_eq!(v["cc"][0], "cc@example.com");
        assert_eq!(v["bcc"][0], "bcc@example.com");
        assert_eq!(v["attachments"][0]["filename"], "ticket.pdf");
        // Round-trips as base64, not as the raw bytes.
        assert_eq!(v["attachments"][0]["content"], "aGVsbG8=");
        assert_eq!(v["attachments"][0]["content_type"], "application/pdf");
        // Absent, not null: a null content_id would mark it inline.
        assert!(v["attachments"][0].get("content_id").is_none());
        // Bare, no angle brackets — Resend matches `src="cid:logo"` verbatim.
        assert_eq!(v["attachments"][1]["content_id"], "logo");
    }

    /// Empty collections must vanish from the body rather than serialize as
    /// `[]` or `null`.
    #[test]
    fn empty_copies_and_attachments_are_omitted() {
        let sender = EmailSender::default();
        let msg = EmailMessage::new("user@example.com", "Subject", "body");
        let body = SendRequest {
            from: sender.from_header(),
            to: &msg.to,
            cc: &msg.cc,
            bcc: &msg.bcc,
            subject: &msg.subject,
            text: &msg.text,
            html: msg.html.as_deref(),
            reply_to: sender.reply_to.as_deref(),
            attachments: Vec::new(),
        };
        let v = serde_json::to_value(&body).expect("serializes");
        assert!(v.get("cc").is_none());
        assert!(v.get("bcc").is_none());
        assert!(v.get("attachments").is_none());
    }

    /// A sender's `api_base` must actually redirect the request — this is what
    /// a regional endpoint and every stubbed test depend on.
    #[test]
    fn api_base_overrides_the_endpoint() {
        let mut sender = EmailSender {
            name: "r".to_string(),
            ..Default::default()
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
