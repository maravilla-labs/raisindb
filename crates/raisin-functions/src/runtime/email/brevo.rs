// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Brevo provider: `POST https://api.brevo.com/v3/smtp/email` with an `api-key`
//! header (Brevo does not use bearer auth).
//!
//! This is Brevo's REST transactional API — the `smtp` in the path is Brevo's
//! naming, not a protocol. It needs a **v3 API key** from Settings → API keys.
//! An **SMTP key** (Settings → SMTP & API → SMTP) is a different credential for
//! Brevo's relay and returns 401 here; that mix-up is the single most common
//! reason a correct-looking Brevo config never delivers. To use an SMTP key,
//! configure a provider with `smtp` instead.
//!
//! <https://developers.brevo.com/reference/sendtransacemail>

use serde::{Deserialize, Serialize};

use super::provider::{build_client, error_from_response, map_transport_err, SendReceipt};
use super::{Credential, EmailError, EmailMessage, EmailProvider, EmailSender, Result};

/// Brevo's API root. Overridable per sender through `api_base`.
const API_BASE: &str = "https://api.brevo.com";
/// Path of the transactional send endpoint, appended to the API base.
const SEND_PATH: &str = "/v3/smtp/email";
/// Header Brevo authenticates with. Not `Authorization`.
const API_KEY_HEADER: &str = "api-key";

/// An address in Brevo's `{email, name}` shape.
#[derive(Serialize)]
struct Address<'a> {
    email: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

/// Request body. Brevo's wire format is camelCase.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendRequest<'a> {
    /// From config, never from the caller.
    sender: Address<'a>,
    to: Vec<Address<'a>>,
    subject: &'a str,
    text_content: &'a str,
    /// NOT optional, unlike every other provider's HTML field.
    ///
    /// Brevo rejects a send that carries neither `htmlContent` nor `templateId`
    /// (see the reference linked above), so a text-only message — which
    /// [`EmailMessage`] permits and Resend accepts — would 400 here. When the
    /// caller supplied no HTML, one is derived from the text rather than
    /// failing: see [`html_from_text`].
    html_content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<Address<'a>>,
}

/// Success body. Brevo answers `{"messageId": "<...>"}` for a single recipient
/// and `{"messageIds": ["<...>", ...]}` when the message fans out, so both
/// shapes have to be accepted.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendResponse {
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    message_ids: Option<Vec<String>>,
}

/// Send `message` through Brevo.
pub(super) async fn send(
    from: &EmailSender,
    credential: &Credential,
    message: &EmailMessage,
) -> Result<SendReceipt> {
    let body = SendRequest {
        sender: Address {
            email: &from.from_address,
            name: from.from_name.as_deref(),
        },
        to: message
            .to
            .iter()
            .map(|email| Address { email, name: None })
            .collect(),
        subject: &message.subject,
        text_content: &message.text,
        html_content: message
            .html
            .clone()
            .unwrap_or_else(|| html_from_text(&message.text)),
        reply_to: from
            .reply_to
            .as_deref()
            .map(|email| Address { email, name: None }),
    };

    let endpoint = super::provider::endpoint(from, API_BASE, SEND_PATH);
    let resp = build_client()?
        .post(&endpoint)
        .header(API_KEY_HEADER, credential.expose())
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&body)
        .send()
        .await
        .map_err(map_transport_err)?;

    if !resp.status().is_success() {
        return Err(error_from_response(EmailProvider::Brevo, resp).await);
    }

    let parsed: SendResponse = resp.json().await.map_err(|e| {
        EmailError::Provider(format!(
            "brevo accepted the send but its body did not parse: {e}"
        ))
    })?;

    let message_id = parsed
        .message_id
        .or_else(|| parsed.message_ids.and_then(|ids| ids.into_iter().next()))
        .ok_or_else(|| EmailError::Provider("brevo returned no messageId".to_string()))?;

    Ok(SendReceipt {
        message_id,
        provider: EmailProvider::Brevo,
        sender: String::new(),
    })
}

/// Wrap plain text as minimal HTML, for Brevo's mandatory `htmlContent`.
///
/// ESCAPED, not interpolated. The text is caller-supplied — a magic-link body, a
/// notification, whatever a function passed in — and dropping it into markup raw
/// would turn any `<` in it into live HTML in the recipient's mail client. `<pre>`
/// keeps the line breaks the plain-text version intended.
fn html_from_text(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<pre style=\"font: inherit; white-space: pre-wrap\">{escaped}</pre>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_uses_brevos_camel_case_shape() {
        let cfg = EmailSender {
            name: "brevo".to_string(),
            provider: EmailProvider::Brevo,
            from_address: "noreply@example.com".to_string(),
            from_name: Some("Example".to_string()),
            reply_to: None,
            credential_ref: "secret://email/api_key".to_string(),
            api_base: None,
            smtp: None,
        };
        let msg = EmailMessage::new("user@example.com", "Sign in", "link").with_html("<b>l</b>");
        let body = SendRequest {
            sender: Address {
                email: &cfg.from_address,
                name: cfg.from_name.as_deref(),
            },
            to: vec![Address {
                email: &msg.to[0],
                name: None,
            }],
            subject: &msg.subject,
            text_content: &msg.text,
            html_content: msg
                .html
                .clone()
                .unwrap_or_else(|| html_from_text(&msg.text)),
            reply_to: None,
        };
        let v = serde_json::to_value(&body).expect("serializes");
        assert_eq!(v["sender"]["email"], "noreply@example.com");
        assert_eq!(v["sender"]["name"], "Example");
        assert_eq!(v["to"][0]["email"], "user@example.com");
        assert_eq!(v["textContent"], "link");
        assert_eq!(v["htmlContent"], "<b>l</b>");
        assert!(v.get("replyTo").is_none());
    }

    #[test]
    fn response_accepts_both_single_and_fanned_out_ids() {
        let single: SendResponse =
            serde_json::from_str(r#"{"messageId":"<a@brevo>"}"#).expect("parses");
        assert_eq!(single.message_id.as_deref(), Some("<a@brevo>"));

        let many: SendResponse =
            serde_json::from_str(r#"{"messageIds":["<a@brevo>","<b@brevo>"]}"#).expect("parses");
        assert!(many.message_id.is_none());
        assert_eq!(many.message_ids.unwrap().len(), 2);
    }
}

#[cfg(test)]
mod html_required_tests {
    use super::*;

    /// Brevo rejects a send carrying neither `htmlContent` nor `templateId`, so
    /// a text-only message must still produce one.
    #[test]
    fn a_text_only_message_still_carries_html_content() {
        let html = html_from_text("Sign in:\nhttps://x.test/a?b=1");
        assert!(html.contains("Sign in:"));
        assert!(html.contains("https://x.test/a?b=1"));
        assert!(!html.is_empty());
    }

    /// The text is caller-supplied, so it must be escaped rather than
    /// interpolated — otherwise a `<` in a notification body becomes live markup
    /// in the recipient's mail client.
    #[test]
    fn text_is_escaped_not_interpolated() {
        let html = html_from_text("<script>alert(1)</script> a & b");
        assert!(
            !html.contains("<script>"),
            "raw markup must not survive: {html}"
        );
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("a &amp; b"));
    }
}
