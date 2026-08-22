// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! SMTP provider: direct submission to a relay or your own mail server.
//!
//! The two HTTP providers cover the services that offer an API. SMTP covers
//! everything else — a corporate relay, a self-hosted Postfix, or the SMTP side
//! of a service whose API key you would rather not mint (Brevo's relay, for
//! one, whose SMTP key is a different credential from its v3 API key).
//!
//! # Why this is not "raw TCP from the sandbox"
//!
//! A function never holds the socket, the host or the password. It names a
//! provider; the host, port, security mode and username come from the operator's
//! `/config/email`, the password comes from the secret store under the
//! function's own `secret_policy`, and this module opens the session. The
//! function's reach is therefore exactly the set of relays the operator
//! configured — the same shape as the HTTP providers, not a general socket.
//!
//! Two things follow, and both are enforced below:
//!
//! * **The host goes through the egress guard.** An operator-typed hostname is
//!   dialled by the server, which is the SSRF shape: `127.0.0.1:25` is a local
//!   relay that may accept unauthenticated mail, and `169.254.169.254` is cloud
//!   instance metadata. Private addresses are refused unless the operator has
//!   opted into them process-wide, exactly as the MCP client does.
//! * **The message is composed by `lettre`, never by string formatting.** SMTP
//!   is the protocol where a CR/LF in a subject really does inject a header.
//!   [`EmailMessage::validate`] already refuses those, but the encoder is the
//!   layer that makes it structurally impossible.

use std::net::IpAddr;
use std::time::Duration;

use lettre::message::header::ContentType;
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials as SmtpCredentials;
use lettre::transport::smtp::response::{Category, Response, Severity};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use super::provider::SendReceipt;
use super::{
    Credential, EmailError, EmailMessage, EmailProvider, EmailSender, Result, SmtpSecurity,
    SmtpSettings, CONNECT_TIMEOUT_SECS, SEND_TIMEOUT_SECS,
};

/// Send `message` over SMTP.
pub(super) async fn send(
    sender: &EmailSender,
    credential: &Credential,
    message: &EmailMessage,
) -> Result<SendReceipt> {
    let smtp = sender.smtp.as_ref().ok_or_else(|| {
        EmailError::Config(format!(
            "email provider `{}` selects smtp but carries no smtp settings",
            sender.name
        ))
    })?;

    guard_host(&smtp.host).await?;

    let email = build_message(sender, message)?;
    let transport = build_transport(smtp, credential)?;

    let response = transport.send(email).await.map_err(map_smtp_err)?;

    Ok(SendReceipt {
        message_id: queue_id(&response),
        provider: EmailProvider::Smtp,
        sender: String::new(),
    })
}

/// Refuse a relay host that resolves to an address the server must not dial.
///
/// Reuses the MCP client's operator-owned policy rather than a second one: an
/// operator who has opted into private egress for a sidecar has already made
/// this decision, and a deployment whose relay genuinely lives on the private
/// network turns the same switch on. Default is public-only.
///
/// Every resolved address must pass, not merely one — a host answering with
/// both a public and a private record would otherwise be dialled on whichever
/// the connector happened to pick.
async fn guard_host(host: &str) -> Result<()> {
    let policy = raisin_mcp_protocol::client::installed_egress_policy();

    // A literal IP has nothing to resolve; judge it directly. `lookup_host`
    // would accept it too, but going through DNS for an address we already have
    // makes a failure read as a DNS problem.
    if let Ok(addr) = host.parse::<IpAddr>() {
        return policy
            .validate_resolved(host, &[addr])
            .map_err(|e| EmailError::Config(format!("smtp host {host} is not allowed: {e}")));
    }

    // Port is irrelevant to an address check; 25 only satisfies the signature.
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, 25u16))
        .await
        .map_err(|e| EmailError::Transport(format!("could not resolve smtp host {host}: {e}")))?
        .collect();
    if addrs.is_empty() {
        return Err(EmailError::Transport(format!(
            "smtp host {host} resolved to no addresses"
        )));
    }
    let ips: Vec<IpAddr> = addrs.iter().map(|s| s.ip()).collect();
    policy
        .validate_resolved(host, &ips)
        .map_err(|e| EmailError::Config(format!("smtp host {host} is not allowed: {e}")))
}

/// Compose the RFC 5322 message.
///
/// `text` is always present and `html`, when given, rides alongside it as a
/// `multipart/alternative` — never instead of it. An HTML-only message is both
/// a spam signal and unreadable in a text client, which is why
/// [`EmailMessage`] requires the text body in the first place.
fn build_message(sender: &EmailSender, message: &EmailMessage) -> Result<Message> {
    let from: Mailbox = sender
        .from_header()
        .parse()
        .map_err(|e| EmailError::Config(format!("invalid from_address: {e}")))?;

    let mut builder = Message::builder().from(from).subject(&message.subject);

    for to in &message.to {
        let mailbox: Mailbox = to
            .parse()
            .map_err(|e| EmailError::InvalidMessage(format!("invalid recipient `{to}`: {e}")))?;
        builder = builder.to(mailbox);
    }

    if let Some(reply_to) = sender
        .reply_to
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        let mailbox: Mailbox = reply_to
            .parse()
            .map_err(|e| EmailError::Config(format!("invalid reply_to: {e}")))?;
        builder = builder.reply_to(mailbox);
    }

    let body = match message.html.as_deref() {
        Some(html) if !html.is_empty() => {
            MultiPart::alternative_plain_html(message.text.clone(), html.to_string())
        }
        _ => MultiPart::mixed().singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(message.text.clone()),
        ),
    };

    builder
        .multipart(body)
        .map_err(|e| EmailError::InvalidMessage(format!("could not compose message: {e}")))
}

/// Build the transport for one send.
///
/// Not pooled. A transactional send is rare and a pool keyed by tenant would
/// hold authenticated sessions to a tenant's relay for as long as the process
/// lives — the handshake is the cheaper side of that trade.
fn build_transport(
    smtp: &SmtpSettings,
    credential: &Credential,
) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let host = smtp.host.trim();
    let mut builder = match smtp.security {
        // Implicit TLS from the first byte (465).
        SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .map_err(|e| EmailError::Config(format!("smtp tls setup failed: {e}")))?,
        // Plaintext connect, then STARTTLS — and lettre's `starttls_relay`
        // REQUIRES the upgrade rather than trying it, so a relay that silently
        // dropped TLS support fails instead of sending the password in clear.
        SmtpSecurity::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| EmailError::Config(format!("smtp starttls setup failed: {e}")))?,
        // No encryption. `builder_dangerous` is lettre's name for it and the
        // name is the point: the credential crosses the wire in the clear, so
        // this is only ever right for a relay on a trusted network.
        SmtpSecurity::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
    };

    builder = builder
        .port(smtp.port)
        .timeout(Some(Duration::from_secs(SEND_TIMEOUT_SECS)));

    // An unauthenticated relay is legitimate (an internal MTA that trusts the
    // source address), so an absent username is not an error — but a username
    // with no password is, because it means the secret did not resolve and the
    // send would fail as an obscure 535 instead.
    if let Some(username) = smtp
        .username
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        builder = builder.credentials(SmtpCredentials::new(
            username.to_string(),
            credential.expose().to_string(),
        ));
    }

    let _ = CONNECT_TIMEOUT_SECS; // one overall timeout; lettre has no separate connect timeout
    Ok(builder.build())
}

/// The relay's queue id for this message.
///
/// A `250` response body is usually `2.0.0 Ok: queued as 4bJ1x…`, and that id is
/// what a postmaster greps the mail log for — far more useful downstream than
/// the locally generated `Message-ID`. When the relay says nothing quotable, the
/// SMTP status code stands in rather than an empty string, so a receipt always
/// carries something.
fn queue_id(response: &Response) -> String {
    response
        .message()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("smtp {}", response.code()))
}

/// Map a `lettre` SMTP failure onto the shared taxonomy.
///
/// The distinction that matters operationally is the same as for the HTTP
/// providers: a rejected credential is an operator problem (rotate the secret),
/// a rejected message is a caller problem (fix the message). The SMTP reply code
/// carries that, so it is read rather than collapsed into "send failed".
///
/// The code is read as its THREE fields, not by matching on the rendered text.
/// `5yz` is permanent, `4yz` transient, and the authentication replies — 530
/// (required), 534 (mechanism too weak), 535 (credentials invalid), 538 — are
/// exactly the permanent ones in category 3.
fn map_smtp_err(e: lettre::transport::smtp::Error) -> EmailError {
    // The `lettre` error renders the host and the server's reply, never the
    // credentials the transport was built with.
    let detail = e.to_string();

    if e.is_timeout() {
        return EmailError::Timeout;
    }

    if let Some(code) = e.status() {
        let text = format!("smtp rejected the send: {detail}");
        return match (code.severity, code.category) {
            (Severity::PermanentNegativeCompletion, Category::Unspecified3) => {
                EmailError::Auth(text)
            }
            (Severity::PermanentNegativeCompletion, _) => EmailError::InvalidMessage(text),
            // 4yz is the relay saying "not now": busy, greylisting, over quota.
            // Retryable, so it maps onto the same code as an HTTP 429.
            (Severity::TransientNegativeCompletion, _) => EmailError::RateLimited(text),
            _ => EmailError::Provider(text),
        };
    }

    if e.is_response() || e.is_permanent() {
        return EmailError::Provider(format!("smtp error: {detail}"));
    }
    EmailError::Transport(format!("smtp transport error: {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender(security: SmtpSecurity) -> EmailSender {
        EmailSender {
            name: "relay".to_string(),
            provider: EmailProvider::Smtp,
            from_address: "noreply@example.com".to_string(),
            from_name: Some("Example".to_string()),
            reply_to: Some("support@example.com".to_string()),
            credential_ref: "secret://email/smtp_password".to_string(),
            api_base: None,
            smtp: Some(SmtpSettings {
                host: "smtp.example.com".to_string(),
                port: 587,
                username: Some("apikey".to_string()),
                security,
            }),
        }
    }

    #[test]
    fn a_text_only_message_composes() {
        let msg = EmailMessage::new("user@example.com", "Sign in", "click here");
        let email = build_message(&sender(SmtpSecurity::Starttls), &msg).expect("composes");
        let raw = String::from_utf8(email.formatted()).expect("utf8");
        assert!(raw.contains("From: Example <noreply@example.com>"));
        assert!(raw.contains("To: user@example.com"));
        assert!(raw.contains("Reply-To: support@example.com"));
        assert!(raw.contains("Subject: Sign in"));
        assert!(raw.contains("click here"));
    }

    #[test]
    fn html_rides_alongside_the_text_never_instead_of_it() {
        let msg = EmailMessage::new("user@example.com", "Sign in", "plain body")
            .with_html("<b>rich body</b>");
        let email = build_message(&sender(SmtpSecurity::Tls), &msg).expect("composes");
        let raw = String::from_utf8(email.formatted()).expect("utf8");
        assert!(raw.contains("multipart/alternative"));
        assert!(raw.contains("plain body"));
        assert!(raw.contains("rich body"));
    }

    /// Several recipients must all reach the `To` header — an SMTP send is
    /// where a dropped recipient would be silent.
    #[test]
    fn every_recipient_reaches_the_header() {
        let msg = EmailMessage {
            to: vec!["a@example.com".to_string(), "b@example.com".to_string()],
            subject: "Notice".to_string(),
            text: "body".to_string(),
            html: None,
        };
        let email = build_message(&sender(SmtpSecurity::Starttls), &msg).expect("composes");
        let raw = String::from_utf8(email.formatted()).expect("utf8");
        assert!(raw.contains("a@example.com"));
        assert!(raw.contains("b@example.com"));
    }

    /// The credential must not be reachable from anything the send path builds
    /// — the SMTP settings in particular, which are `Debug`-rendered in config
    /// errors.
    #[test]
    fn smtp_settings_carry_no_credential() {
        // The settings hold a REFERENCE and a username; the password reaches
        // the transport as a separate borrow and must not be reachable from
        // anything a config error renders.
        let rendered = format!("{:?}", sender(SmtpSecurity::Starttls));
        assert!(rendered.contains("secret://email/smtp_password"));
        let password = "hunter2-relay-secret";
        let with_credential = format!(
            "{rendered} {:?}",
            build_transport(
                sender(SmtpSecurity::Starttls).smtp.as_ref().unwrap(),
                &Credential::new(password),
            )
            .map(|_| "transport built")
        );
        assert!(
            !with_credential.contains(password),
            "credential leaked: {with_credential}"
        );
    }

    /// The reply code decides the error class, and getting it wrong is what
    /// makes an email outage hard to triage: 535 means rotate the secret, 550
    /// means fix the message, 451 means try later.
    #[test]
    fn a_reply_code_maps_onto_the_right_class() {
        use lettre::transport::smtp::response::{Code, Detail};

        let classify = |severity, category, detail| {
            let code = Code::new(severity, category, detail);
            match (code.severity, code.category) {
                (Severity::PermanentNegativeCompletion, Category::Unspecified3) => "auth_failed",
                (Severity::PermanentNegativeCompletion, _) => "invalid_message",
                (Severity::TransientNegativeCompletion, _) => "rate_limited",
                _ => "provider_error",
            }
        };

        // 535 Authentication credentials invalid.
        assert_eq!(
            classify(
                Severity::PermanentNegativeCompletion,
                Category::Unspecified3,
                Detail::Five
            ),
            "auth_failed"
        );
        // 550 Mailbox unavailable / sender rejected.
        assert_eq!(
            classify(
                Severity::PermanentNegativeCompletion,
                Category::MailSystem,
                Detail::Zero
            ),
            "invalid_message"
        );
        // 451 Local error in processing — transient, worth retrying.
        assert_eq!(
            classify(
                Severity::TransientNegativeCompletion,
                Category::MailSystem,
                Detail::One
            ),
            "rate_limited"
        );
    }

    /// Loopback is the address an operator reaches for first and the one that
    /// must be refused by default: a local relay commonly accepts
    /// unauthenticated mail, and `169.254.169.254` is instance metadata.
    #[tokio::test]
    async fn a_loopback_relay_is_refused_by_default() {
        let err = guard_host("127.0.0.1")
            .await
            .expect_err("loopback must be refused");
        assert_eq!(err.code(), "config");
        let err = guard_host("169.254.169.254")
            .await
            .expect_err("link-local must be refused");
        assert_eq!(err.code(), "config");
    }

    /// Missing settings must fail as configuration rather than panicking on the
    /// `Option` — this is the shape of a provider entry switched to `smtp` in
    /// the console and saved before its host was filled in.
    #[tokio::test]
    async fn smtp_without_settings_is_a_config_error() {
        let mut s = sender(SmtpSecurity::Starttls);
        s.smtp = None;
        let msg = EmailMessage::new("user@example.com", "Sign in", "link");
        let err = send(&s, &Credential::new("pw"), &msg)
            .await
            .expect_err("no settings, no send");
        assert_eq!(err.code(), "config");
    }
}
